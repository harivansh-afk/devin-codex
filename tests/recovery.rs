use axum::{
    Router,
    body::{Body, Bytes, to_bytes},
    http::{Request, StatusCode},
    response::Response,
    routing::post,
};
use devin_codex::{
    Mode,
    config::Credentials,
    gateway::GatewayClient,
    models::Models,
    proto::{ChatResponse, ToolCall},
    server,
};
use futures_util::StreamExt;
use prost::Message;
use serde_json::{Value, json};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tower::ServiceExt;

const TOKEN: &str = "synthetic-private-recovery-token";

type Reply = (StatusCode, Option<&'static str>, Vec<Vec<u8>>);

fn frame(payload: &[u8], flags: u8) -> Vec<u8> {
    let mut bytes = vec![flags];
    bytes.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    bytes.extend_from_slice(payload);
    bytes
}

fn partial() -> Vec<u8> {
    frame(
        &ChatResponse {
            delta_text: "PRIVATE partial text".into(),
            delta_signature: "PRIVATE incomplete signature".into(),
            delta_signature_type: "openai".into(),
            delta_tool_calls: vec![ToolCall {
                id: "discarded_call".into(),
                name: "run".into(),
                arguments_json: "{\"PRIVATE\":".into(),
                ..Default::default()
            }],
            ..Default::default()
        }
        .encode_to_vec(),
        0,
    )
}

fn success() -> Reply {
    (
        StatusCode::OK,
        None,
        vec![
            frame(
                &ChatResponse {
                    delta_tool_calls: vec![ToolCall {
                        id: "kept_call".into(),
                        name: "run".into(),
                        arguments_json: "{\"ok\":true}".into(),
                        ..Default::default()
                    }],
                    stop_reason: Some(10),
                    ..Default::default()
                }
                .encode_to_vec(),
                0,
            ),
            frame(b"{}", 2),
        ],
    )
}

fn failure(code: &str, message: &str) -> Reply {
    (
        StatusCode::OK,
        None,
        vec![
            partial(),
            frame(
                json!({"error":{"code":code,"message":message}})
                    .to_string()
                    .as_bytes(),
                2,
            ),
        ],
    )
}

struct Fixture {
    app: Router,
    calls: Arc<AtomicUsize>,
    upstream: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.upstream.abort();
    }
}

async fn fixture(replies: Vec<Reply>) -> Fixture {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let first_body = Arc::new(Mutex::new(None::<Bytes>));
    let upstream = Router::new().route(
        "/rpc",
        post(move |body: Bytes| {
            let index = counter.fetch_add(1, Ordering::SeqCst);
            let mut first = first_body.lock().unwrap();
            if let Some(first) = &*first {
                assert_eq!(&body, first, "retry must replay the identical request");
            } else {
                *first = Some(body);
            }
            let (status, retry_after, chunks) = replies[index.min(replies.len() - 1)].clone();
            async move {
                let mut builder = Response::builder()
                    .status(status)
                    .header("content-type", "application/connect+proto");
                if let Some(delay) = retry_after {
                    builder = builder.header("retry-after", delay);
                }
                let stream = async_stream::stream! {
                    for chunk in chunks {
                        yield Ok::<_, std::io::Error>(chunk);
                        // Ensure partial output reaches the translator before the failure.
                        tokio::time::sleep(Duration::from_millis(5)).await;
                    }
                };
                builder.body(Body::from_stream(stream)).unwrap()
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/rpc", listener.local_addr().unwrap());
    let upstream = tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });
    let app = server::router(
        Credentials {
            endpoint: endpoint.clone(),
            api_key: "test-key".into(),
            device_id: "test".into(),
            client_version: "test".into(),
        },
        TOKEN.into(),
        GatewayClient::new(&endpoint, "test-key").unwrap(),
        Mode::Strict,
        Models::default(),
    );
    Fixture {
        app,
        calls,
        upstream,
    }
}

fn body(streaming: bool) -> Value {
    json!({"model":"gpt-6-astra", "input":"hello", "stream":streaming, "tools":[{"type":"function","name":"run","parameters":{"type":"object"}}]})
}

async fn collect(fixture: &Fixture, streaming: bool) -> (Value, Vec<Value>) {
    let request = Request::post("/v1/responses")
        .header("authorization", format!("Bearer {TOKEN}"))
        .header("content-type", "application/json")
        .body(Body::from(body(streaming).to_string()))
        .unwrap();
    let response = fixture.app.clone().oneshot(request).await.unwrap();
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(!text.contains("PRIVATE"));
    assert!(!text.contains("discarded_call"));
    if streaming {
        let events: Vec<Value> = text
            .lines()
            .filter_map(|line| line.strip_prefix("data: "))
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        for (index, event) in events.iter().enumerate() {
            assert_eq!(event["sequence_number"], index);
        }
        assert_eq!(
            events
                .iter()
                .filter(|e| e["type"] == "response.created")
                .count(),
            1
        );
        let final_response = events.last().unwrap()["response"].clone();
        assert_eq!(events[0]["response"]["id"], final_response["id"]);
        (final_response, events)
    } else {
        (serde_json::from_str(&text).unwrap(), vec![])
    }
}

#[tokio::test]
async fn unknown_after_partial_tool_and_reasoning_recovers_without_duplicate_output() {
    for streaming in [false, true] {
        let fixture = fixture(vec![
            failure(
                "unknown",
                "PRIVATE upstream cause (trace ID: 0123456789abcdef0123456789abcdef)",
            ),
            success(),
        ])
        .await;
        let (response, events) = collect(&fixture, streaming).await;
        assert_eq!(fixture.calls.load(Ordering::SeqCst), 2);
        assert_eq!(response["status"], "completed");
        assert_eq!(response["output"].as_array().unwrap().len(), 1);
        assert_eq!(response["output"][0]["call_id"], "kept_call");
        assert_eq!(response["output"][0]["arguments"], "{\"ok\":true}");
        assert_eq!(response["x_devin_gateway"]["attempts"], 2);
        assert_eq!(
            response["x_devin_gateway"]["failures"][0]["trace_id"],
            "0123456789abcdef0123456789abcdef"
        );
        if streaming {
            assert_eq!(
                events
                    .iter()
                    .filter(|e| e["type"] == "response.output_item.done")
                    .count(),
                1
            );
            assert_eq!(events.last().unwrap()["type"], "response.completed");
        }
    }
}

#[tokio::test]
async fn transient_http_and_truncated_streams_recover() {
    for failed in [
        (
            StatusCode::SERVICE_UNAVAILABLE,
            None,
            vec![b"PRIVATE".to_vec()],
        ),
        (StatusCode::TOO_MANY_REQUESTS, Some("1"), vec![]),
        (StatusCode::OK, None, vec![]),
        (StatusCode::OK, None, vec![partial(), vec![2, 0, 0]]),
    ] {
        let rate_limited = failed.0 == StatusCode::TOO_MANY_REQUESTS;
        let fixture = fixture(vec![failed, success()]).await;
        let started = Instant::now();
        let (response, _) = collect(&fixture, true).await;
        assert_eq!(response["status"], "completed");
        assert_eq!(fixture.calls.load(Ordering::SeqCst), 2);
        if rate_limited {
            assert!(started.elapsed() >= Duration::from_secs(1));
        }
    }
}

#[tokio::test]
async fn persistent_failure_exhausts_four_attempts_without_executable_output() {
    let fixture = fixture(vec![failure("unknown", "PRIVATE")]).await;
    let (response, events) = collect(&fixture, true).await;
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 4);
    assert_eq!(response["status"], "failed");
    assert_eq!(response["output"], json!([]));
    assert_eq!(response["x_devin_gateway"]["attempts"], 4);
    assert_eq!(
        response["x_devin_gateway"]["failures"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
    assert!(
        response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("4 attempts")
    );
    assert_eq!(events.len(), 3);
}

#[tokio::test]
async fn permanent_failures_and_long_limits_are_not_retried() {
    for failed in [
        failure("unauthenticated", "PRIVATE"),
        failure("permission_denied", "PRIVATE"),
        failure("invalid_argument", "PRIVATE"),
        failure("unknown", "PRIVATE out of credits"),
        failure("unknown", "PRIVATE maximum context length exceeded"),
        failure(
            "permission_denied",
            "PRIVATE rate limit will reset in 13 minutes",
        ),
        (StatusCode::TOO_MANY_REQUESTS, Some("61"), vec![]),
        (StatusCode::UNAUTHORIZED, None, vec![]),
        (StatusCode::OK, None, vec![frame(b"not-json", 2)]),
        (StatusCode::OK, None, vec![frame(b"{}", 2), vec![0]]),
    ] {
        let fixture = fixture(vec![failed, success()]).await;
        let (response, _) = collect(&fixture, false).await;
        assert_eq!(response["status"], "failed");
        assert_eq!(fixture.calls.load(Ordering::SeqCst), 1);
        assert_eq!(response["output"], json!([]));
    }
}

#[tokio::test]
async fn dropping_downstream_during_backoff_stops_retries() {
    let fixture = fixture(vec![
        (StatusCode::SERVICE_UNAVAILABLE, Some("2"), vec![]),
        success(),
    ])
    .await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1/responses", listener.local_addr().unwrap());
    let app = fixture.app.clone();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::new();
    let response = client
        .post(url)
        .bearer_auth(TOKEN)
        .json(&body(true))
        .send()
        .await
        .unwrap();
    let mut stream = response.bytes_stream();
    assert!(stream.next().await.unwrap().is_ok());
    tokio::time::timeout(Duration::from_secs(1), async {
        while fixture.calls.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    drop(stream);
    tokio::time::sleep(Duration::from_millis(2500)).await;
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 1);
    server.abort();
}
