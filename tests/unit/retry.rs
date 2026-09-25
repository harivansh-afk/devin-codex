use super::*;

#[test]
fn retries_back_off_and_stop_without_shortening_upstream_wait() {
    let mut error = GatewayError::new("unknown");
    for attempt in 1..MAX_ATTEMPTS {
        let delay = retry_delay(&error, attempt).unwrap();
        let minimum = Duration::from_secs(1 << (attempt - 1));
        assert!(delay >= minimum && delay <= minimum + Duration::from_millis(250));
    }
    assert!(retry_delay(&error, MAX_ATTEMPTS).is_none());
    error.retry_after = Some(Duration::from_secs(30));
    assert!(retry_delay(&error, 1).unwrap() >= Duration::from_secs(30));
    error.retry_after = Some(Duration::from_secs(61));
    assert!(retry_delay(&error, 1).is_none());
    for code in [
        "unauthenticated",
        "permission_denied",
        "invalid_argument",
        "upstream_protocol_error",
        "canceled",
        "request_too_large",
    ] {
        assert!(retry_delay(&GatewayError::new(code), 1).is_none(), "{code}");
    }
    for code in [
        "internal",
        "unavailable",
        "aborted",
        "deadline_exceeded",
        "resource_exhausted",
        "upstream_timeout",
        "upstream_connection_error",
        "upstream_truncated_stream",
    ] {
        assert!(retry_delay(&GatewayError::new(code), 1).is_some(), "{code}");
    }
}
