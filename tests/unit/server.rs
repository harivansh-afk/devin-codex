use super::*;

#[test]
fn decoded_body_limit_and_invalid_json() {
    assert!(decode_body(Bytes::from_static(b"NaN"), false).is_err());
    assert!(decode_body(Bytes::from_static(b"bad"), true).is_err());
    let encoded = zstd::encode_all(vec![b' '; MAX_BODY + 1].as_slice(), 1).unwrap();
    assert_eq!(
        decode_body(encoded.into(), true).unwrap_err().0,
        StatusCode::PAYLOAD_TOO_LARGE
    );
    let encoded = zstd::encode_all(b"{\"input\":\"hello\"}".as_slice(), 1).unwrap();
    assert_eq!(decode_body(encoded.into(), true).unwrap()["input"], "hello");
}

#[tokio::test(start_paused = true)]
async fn buffered_response_sends_data_heartbeats_and_stops_at_eof() {
    use std::time::Duration;

    let events = async_stream::stream! {
        yield json!({"type":"response.created"});
        tokio::time::sleep(Duration::from_secs(46)).await;
        yield json!({"type":"response.completed"});
    };
    let mut body = streaming_response(events.boxed())
        .into_body()
        .into_data_stream();
    let created = body.next().await.unwrap().unwrap();
    assert!(
        std::str::from_utf8(&created)
            .unwrap()
            .contains("response.created")
    );

    // Codex's idle timer wraps parsed events, so an SSE comment cannot help.
    // No response output is released while the generation is still buffered.
    for _ in 0..3 {
        let heartbeat = tokio::time::timeout(Duration::from_secs(20), body.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let heartbeat = std::str::from_utf8(&heartbeat).unwrap();
        let data = heartbeat
            .lines()
            .find_map(|line| line.strip_prefix("data: "))
            .expect("keepalive must have SSE data, not just a comment");
        assert_eq!(
            serde_json::from_str::<Value>(data).unwrap(),
            json!({"type":"ping"})
        );
    }
    let completed = body.next().await.unwrap().unwrap();
    assert!(
        std::str::from_utf8(&completed)
            .unwrap()
            .contains("response.completed")
    );
    assert!(body.next().await.is_none(), "heartbeats must not mask EOF");
}
