use serde_json::{Value, json};
use std::time::{Duration, SystemTime};

/// Deliberately stores no raw upstream message: errors can quote credentials or prompts.
#[derive(Debug, Clone, thiserror::Error)]
#[error("gateway request failed ({code})")]
pub struct GatewayError {
    pub code: String,
    pub status: Option<u16>,
    pub(crate) category: &'static str,
    pub(crate) trace_id: Option<String>,
    pub(crate) retry_after: Option<Duration>,
    retry_delay_unrecognized: bool,
}

impl GatewayError {
    pub fn new(code: &str) -> Self {
        let category = match code {
            "unauthenticated" => "authentication",
            "permission_denied" => "permission",
            "resource_exhausted" => "rate_limit",
            "invalid_argument" | "failed_precondition" | "out_of_range" => "invalid_request",
            "not_found" | "unimplemented" => "unsupported_request",
            "upstream_timeout" | "deadline_exceeded" => "timeout",
            "upstream_connection_error" | "upstream_truncated_stream" => "connection",
            "internal" | "unavailable" | "aborted" => "upstream_failure",
            _ => "unclassified",
        };
        Self {
            code: code.into(),
            status: None,
            category,
            trace_id: None,
            retry_after: None,
            retry_delay_unrecognized: false,
        }
    }

    pub(super) fn trailer(code: &str, message: Option<&str>) -> Self {
        let mut error = Self::new(code);
        if let Some(message) = message {
            error.read_message(message);
        }
        error
    }

    pub(super) fn http(status: u16, retry_after: Option<&str>) -> Self {
        let mut error = Self::new("upstream_http_error");
        error.status = Some(status);
        error.category = match status {
            401 => "authentication",
            403 => "permission",
            429 => "rate_limit",
            408 | 504 => "timeout",
            500 | 502 | 503 => "upstream_failure",
            400 | 413 | 422 => "invalid_request",
            _ => "unclassified",
        };
        error.retry_after = retry_after.and_then(|value| {
            value
                .trim()
                .parse::<u64>()
                .ok()
                .map(Duration::from_secs)
                .or_else(|| {
                    httpdate::parse_http_date(value)
                        .ok()
                        .map(|when| when.duration_since(SystemTime::now()).unwrap_or_default())
                })
        });
        error
    }

    fn read_message(&mut self, message: &str) {
        // Classify a bounded prefix without retaining or displaying upstream free text.
        let message: String = message.chars().take(8192).collect();
        let lower = message.to_ascii_lowercase();
        if let Some((_, tail)) = lower.split_once("(trace id: ")
            && let Some((id, _)) = tail.split_once(')')
            && ((id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit()))
                || (id.len() == 36 && uuid::Uuid::parse_str(id).is_ok()))
        {
            self.trace_id = Some(id.to_owned());
        }
        // Codes carrying authorization/validation failures stay authoritative.
        if !matches!(
            self.code.as_str(),
            "unknown" | "internal" | "resource_exhausted" | "permission_denied" | "unavailable"
        ) {
            return;
        }
        if [
            "out of credits",
            "insufficient credits",
            "quota exceeded",
            "billing limit",
            "spending limit",
        ]
        .iter()
        .any(|s| lower.contains(s))
        {
            self.category = "quota_exhausted";
        } else if [
            "context length",
            "context window",
            "too many tokens",
            "maximum context",
        ]
        .iter()
        .any(|s| lower.contains(s))
        {
            self.category = "context_length";
        } else if lower.contains("rate limit") || lower.contains("limit will reset") {
            self.category = "rate_limit";
        }
        if let Some((_, tail)) = lower.split_once("reset in ") {
            self.retry_after = reset_delay(tail);
            self.retry_delay_unrecognized = self.retry_after.is_none();
        }
    }

    pub(crate) fn retryable(&self) -> bool {
        if self.retry_delay_unrecognized
            || matches!(
                self.category,
                "quota_exhausted" | "context_length" | "authentication" | "invalid_request"
            )
        {
            return false;
        }
        if let Some(status) = self.status {
            return matches!(status, 408 | 429 | 500 | 502 | 503 | 504);
        }
        matches!(
            self.code.as_str(),
            "unknown"
                | "internal"
                | "unavailable"
                | "deadline_exceeded"
                | "aborted"
                | "resource_exhausted"
                | "upstream_timeout"
                | "upstream_connection_error"
                | "upstream_truncated_stream"
        ) || (self.code == "permission_denied"
            && self.category == "rate_limit"
            && self.retry_after.is_some())
    }

    pub(crate) fn diagnostic(&self) -> Value {
        json!({"code":self.code,"category":self.category,"http_status":self.status,
            "trace_id":self.trace_id,"retry_after_seconds":self.retry_after.map(|d| d.as_secs()),
            "retry_delay_unrecognized":self.retry_delay_unrecognized,"retryable":self.retryable()})
    }

    pub(crate) fn message(&self, attempts: usize) -> String {
        let status = self
            .status
            .map(|s| format!(" (HTTP {s})"))
            .unwrap_or_default();
        let hint = match self.category {
            "authentication" => "Sign in again with the Devin CLI.",
            "permission" => "Check Devin account access to this model.",
            "quota_exhausted" => "Check your Devin credits or account quota.",
            "context_length" => "Compact the conversation or start a shorter session.",
            "rate_limit" => "Devin is rate limiting requests.",
            "invalid_request" | "unsupported_request" => {
                "Devin rejected this request; retrying it unchanged will not help."
            }
            "timeout" => "Devin did not respond within the upstream timeout.",
            "connection" => "The connection to Devin ended before a complete response arrived.",
            _ => "Devin returned an upstream failure without a recognized cause.",
        };
        let unit = if attempts == 1 { "attempt" } else { "attempts" };
        let mut message = format!(
            "Devin gateway failed: {}{status} after {attempts} {unit}. {hint}",
            self.code
        );
        if let Some(delay) = self.retry_after {
            message.push_str(&format!(" Retry after {} seconds.", delay.as_secs()));
        }
        if self.retry_delay_unrecognized {
            message.push_str(" The server's reset delay could not be safely interpreted; automatic retry was skipped.");
        }
        if let Some(trace) = &self.trace_id {
            message.push_str(&format!(" Trace ID: {trace}."));
        }
        message
    }
}

// Accept whole and compound durations; never shorten "1 minute and 30 seconds"
// to one minute or immediately retry a stated delay that we cannot interpret.
fn reset_delay(tail: &str) -> Option<Duration> {
    let mut words = tail.split_whitespace().peekable();
    let mut seconds = 0u64;
    loop {
        let number = words.next()?.parse::<u64>().ok()?;
        let unit = words.next()?;
        let multiplier = match unit.trim_end_matches(['.', ',', ')']) {
            "second" | "seconds" => 1,
            "minute" | "minutes" => 60,
            "hour" | "hours" => 3600,
            _ => return None,
        };
        seconds = seconds.checked_add(number.checked_mul(multiplier)?)?;
        if words.peek() == Some(&"and") {
            words.next();
        } else if !words
            .peek()
            .is_some_and(|word| word.starts_with(|c: char| c.is_ascii_digit()))
        {
            return Some(Duration::from_secs(seconds));
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/gateway_error.rs"]
mod tests;
