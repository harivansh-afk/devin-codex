use crate::{
    Mode,
    compat::Compatibility,
    config::Credentials,
    gateway::{GatewayClient, GatewayError},
    models::Models,
    proto::ChatRequest,
    request::{Problem, translate_with_models},
    responses::Translator,
};
use futures_util::{StreamExt, stream::BoxStream};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

const MAX_ATTEMPTS: usize = 4;
const MAX_RETRY_WAIT: Duration = Duration::from_secs(60);

pub(crate) struct Adapter {
    credentials: Credentials,
    gateway: GatewayClient,
    pub mode: Mode,
    models: Models,
}

pub(crate) struct PreparedRequest {
    request: ChatRequest,
    translator: Translator,
    mapping: Compatibility,
    pub streaming: bool,
}

impl Adapter {
    pub fn with_models(
        credentials: Credentials,
        gateway: GatewayClient,
        mode: Mode,
        models: Models,
    ) -> Self {
        Self {
            credentials,
            gateway,
            mode,
            models,
        }
    }

    pub fn prepare(&self, body: Value) -> Result<PreparedRequest, Vec<Problem>> {
        let streaming = body.get("stream").and_then(Value::as_bool).unwrap_or(false);
        let (body, mapping) = match self.mode {
            Mode::Strict => (body, Compatibility::default()),
            Mode::Codex => Compatibility::prepare_with_models(&body, &self.models)?,
        };
        let translated = translate_with_models(&body, &self.credentials, &self.models)?;
        Ok(PreparedRequest {
            request: translated.request,
            translator: Translator::new(
                translated.model,
                translated.tool_map,
                translated.allow_tools,
                self.mode,
            ),
            mapping,
            streaming,
        })
    }

    pub fn stream(&self, prepared: PreparedRequest) -> BoxStream<'static, Value> {
        let PreparedRequest {
            request,
            mut translator,
            mapping,
            ..
        } = prepared;
        let gateway = self.gateway.clone();
        let events = async_stream::stream! {
            for event in translator.created_events() {
                yield event;
            }
            // Snapshot only the empty response, after created events advance its sequence.
            // No generated text or executable tool output escapes before finish succeeds.
            let baseline = translator.clone();
            let mut failures = Vec::new();
            let request_id = uuid::Uuid::new_v4().to_string();
            'attempts: for attempt in 1..=MAX_ATTEMPTS {
                let started = Instant::now();
                let mut frames = gateway.stream(request.clone());
                while let Some(frame) = frames.next().await {
                    let frame = match frame {
                        Ok(frame) => frame,
                        Err(error) => {
                            // Close this upstream before backoff, including on downstream cancel.
                            drop(frames);
                            let mut diagnostic = error.diagnostic();
                            diagnostic["attempt"] = json!(attempt);
                            diagnostic["elapsed_ms"] = json!(started.elapsed().as_millis());
                            let delay = retry_delay(&error, attempt);
                            diagnostic["retry_in_ms"] = json!(delay.map(|d| d.as_millis()));
                            crate::diagnostics::record(gateway.diagnostics.clone(), json!({
                                "event":"failure", "request_id":request_id, "failure":diagnostic,
                            })).await;
                            failures.push(diagnostic);
                            if let Some(delay) = delay {
                                translator = baseline.clone();
                                tokio::time::sleep(delay).await;
                                continue 'attempts;
                            }
                            yield with_diagnostics(
                                translator.failed_event(&error.code, &error.message(attempt)),
                                attempt, &request_id, &failures,
                            );
                            return;
                        }
                    };
                    if let Err(error) = translator.push(frame) {
                        yield with_diagnostics(translator.failed_event(
                            "response_translation_error",
                            &format!("Adapter rejected gateway frame: {error}"),
                        ), attempt, &request_id, &failures);
                        return;
                    }
                }
                match translator.finish() {
                    Ok(events) => {
                        if !failures.is_empty() {
                            crate::diagnostics::record(gateway.diagnostics.clone(), json!({
                                "event":"recovered", "request_id":request_id, "attempts":attempt,
                            })).await;
                        }
                        for event in events {
                            yield with_diagnostics(event, attempt, &request_id, &failures);
                        }
                    }
                    Err(error) => yield with_diagnostics(translator.failed_event(
                        "response_translation_error",
                        &format!("Adapter rejected completed gateway response: {error}"),
                    ), attempt, &request_id, &failures),
                }
                return;
            }
        };
        events
            .map(move |mut event| {
                mapping.restore(&mut event);
                event
            })
            .boxed()
    }
}

// Retries belong here, where an entire failed generation can be discarded. Leaving
// Codex/reqwest retries disabled prevents independent retry loops from multiplying.
fn retry_delay(error: &GatewayError, attempt: usize) -> Option<Duration> {
    if attempt >= MAX_ATTEMPTS || !error.retryable() {
        return None;
    }
    if error
        .retry_after
        .is_some_and(|delay| delay > MAX_RETRY_WAIT)
    {
        return None;
    }
    let backoff = Duration::from_secs(1 << (attempt - 1));
    let minimum = backoff.max(error.retry_after.unwrap_or_default());
    Some(minimum + Duration::from_millis(rand::random_range(0..=250)))
}

fn with_diagnostics(
    mut event: Value,
    attempts: usize,
    request_id: &str,
    failures: &[Value],
) -> Value {
    if matches!(
        event["type"].as_str(),
        Some("response.completed" | "response.failed" | "response.incomplete")
    ) {
        event["response"]["x_devin_gateway"] =
            json!({"request_id":request_id,"attempts":attempts,"failures":failures});
    }
    event
}

#[cfg(test)]
#[path = "../tests/unit/retry.rs"]
mod tests;
