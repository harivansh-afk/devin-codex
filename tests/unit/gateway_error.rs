use super::*;

#[test]
fn generic_provider_rejection_is_not_reported_as_a_known_bad_argument() {
    let error = GatewayError::trailer(
        "invalid_argument",
        Some(
            "The third-party model provider is experiencing issues and is currently not available. Please try this model again later. (trace ID: 0123456789abcdef0123456789abcdef)",
        ),
    );
    assert_eq!(error.category, "provider_failure");
    assert!(!error.retryable());
    assert!(error.message(1).contains("/compact"));
    assert!(
        !error
            .message(1)
            .contains("retrying it unchanged will not help")
    );
    assert_eq!(
        error.trace_id.as_deref(),
        Some("0123456789abcdef0123456789abcdef")
    );
    let invalid = GatewayError::trailer("invalid_argument", Some("invalid PRIVATE field"));
    assert_eq!(invalid.category, "invalid_request");
    assert!(!invalid.retryable());
    assert!(!invalid.message(1).contains("PRIVATE"));
}

#[test]
fn known_causes_and_trace_are_retained_without_upstream_text() {
    let error = GatewayError::trailer(
        "unknown",
        Some("PRIVATE request=secret quota exceeded (trace ID: 0123456789abcdef0123456789abcdef)"),
    );
    assert_eq!(error.category, "quota_exhausted");
    assert!(!error.retryable());
    assert_eq!(
        error.trace_id.as_deref(),
        Some("0123456789abcdef0123456789abcdef")
    );
    for output in [
        error.message(1),
        error.diagnostic().to_string(),
        format!("{error:?}"),
    ] {
        assert!(!output.contains("PRIVATE"));
        assert!(!output.contains("secret"));
    }
    assert!(error.message(1).contains("credits"));
    let error = GatewayError::trailer(
        "unknown",
        Some("maximum context length exceeded (trace ID: secret)"),
    );
    assert_eq!(error.category, "context_length");
    assert!(!error.retryable());
    assert_eq!(error.trace_id, None);
    assert!(GatewayError::trailer("unknown", Some("unrecognized private cause")).retryable());
}

#[test]
fn rate_limits_can_arrive_as_permission_errors() {
    let error = GatewayError::trailer(
        "permission_denied",
        Some("Reached overall message rate limit. Your limit will reset in 13 minutes."),
    );
    assert!(error.retryable());
    assert_eq!(error.category, "rate_limit");
    assert_eq!(error.retry_after, Some(Duration::from_secs(780)));
    assert!(error.message(1).contains("780 seconds"));
    assert!(!GatewayError::trailer("permission_denied", Some("model access denied")).retryable());
    assert!(
        !GatewayError::trailer("unauthenticated", Some("rate limit reset in 1 second")).retryable()
    );
}

#[test]
fn statuses_and_retry_after_are_bounded_by_policy_not_transport() {
    for code in [408, 429, 500, 502, 503, 504] {
        assert!(GatewayError::http(code, None).retryable(), "{code}");
    }
    for code in [307, 400, 401, 403, 404, 413, 422, 501] {
        assert!(!GatewayError::http(code, None).retryable(), "{code}");
    }
    assert_eq!(
        GatewayError::http(429, Some("17")).retry_after,
        Some(Duration::from_secs(17))
    );
    assert_eq!(GatewayError::http(429, Some("broken")).retry_after, None);
    let future = SystemTime::now() + Duration::from_secs(120);
    let delay = GatewayError::http(429, Some(&httpdate::fmt_http_date(future)))
        .retry_after
        .unwrap();
    assert!((118..=120).contains(&delay.as_secs()));
    assert_eq!(
        GatewayError::http(503, Some("Thu, 01 Jan 1970 00:00:00 GMT")).retry_after,
        Some(Duration::ZERO)
    );
}

#[test]
fn compound_or_unrecognized_reset_delays_do_not_cause_early_retries() {
    let error = GatewayError::trailer(
        "permission_denied",
        Some("rate limit will reset in 1 minute and 30 seconds."),
    );
    assert_eq!(error.retry_after, Some(Duration::from_secs(90)));
    for message in [
        "rate limit will reset in 1.5 minutes",
        "rate limit will reset in 18446744073709551615 hours",
    ] {
        let error = GatewayError::trailer("unknown", Some(message));
        assert!(!error.retryable());
        assert!(error.message(1).contains("could not be safely interpreted"));
    }
    let error = GatewayError::http(429, Some("18446744073709551615"));
    assert_eq!(error.diagnostic()["retry_after_seconds"], u64::MAX);
}
