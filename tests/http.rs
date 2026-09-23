use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
    routing::post,
};
use devin_codex::{
    Mode,
    config::Credentials,
    gateway::GatewayClient,
    models::Models,
    proto::{ChatRequest, ChatResponse, ToolCall},
    server,
};
use prost::Message;
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tower::ServiceExt;

const TOKEN: &str = "synthetic-private-test-token";

async fn app() -> (Router, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    app_with_mode(false).await
}

async fn app_with_mode(
    compatibility: bool,
) -> (Router, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    app_with_signature(compatibility, None).await
}

async fn app_with_signature(
    compatibility: bool,
    signature: Option<String>,
) -> (Router, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    app_with_models(compatibility, signature, Models::default()).await
}

async fn app_with_models(
    compatibility: bool,
    signature: Option<String>,
    models: Models,
) -> (Router, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let upstream = Router::new().route(
        "/rpc",
        post(
            move |headers: axum::http::HeaderMap, body: axum::body::Bytes| {
                let calls = counter.clone();
                let signature = signature.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    assert_eq!(
                        headers["authorization"],
                        "Basic gateway-test-key-gateway-test-key"
                    );
                    assert!(headers.get("chatgpt-account-id").is_none());
                    assert_eq!(body[0], 0);
                    let request = ChatRequest::decode(&body[5..]).unwrap();
                    let mut response = ChatResponse {
                        delta_text: "hello".into(),
                        phase: "final_answer".into(),
                        actual_model_uid: request.model.clone(),
                        ..Default::default()
                    };
                    if let Some(signature) = signature {
                        for message in &request.messages {
                            if !message.reasoning.is_empty() {
                                assert_eq!(
                                    serde_json::from_str::<Value>(&message.reasoning).unwrap(),
                                    serde_json::from_str::<Value>(&signature).unwrap()
                                );
                                assert_eq!(message.reasoning_type, "openai");
                            }
                        }
                        response.delta_signature = signature;
                        response.delta_signature_type = "openai".into();
                    }
                    if let Some(tool) = request.tools.first() {
                        assert_eq!(tool.name, "collaboration__spawn_agent");
                        let schema: Value = serde_json::from_str(&tool.parameters_json).unwrap();
                        assert!(schema["properties"]["message"].get("encrypted").is_none());
                        if !request.messages.iter().any(|message| message.role == 4) {
                            response.delta_text.clear();
                            response.delta_tool_calls.push(ToolCall {
                                id: "call-child".into(),
                                name: tool.name.clone(),
                                arguments_json:
                                    "{\"message\":\"Reply CHILD_OK\",\"task_name\":\"probe\"}"
                                        .into(),
                                ..Default::default()
                            });
                        } else {
                            assert!(
                                request
                                    .messages
                                    .iter()
                                    .any(|message| message.content.contains("CHILD_OK"))
                            );
                        }
                    }
                    let response = response.encode_to_vec();
                    let mut framed = vec![0];
                    framed.extend_from_slice(&(response.len() as u32).to_be_bytes());
                    framed.extend(response);
                    framed.extend_from_slice(&[2, 0, 0, 0, 2, b'{', b'}']);
                    ([("content-type", "application/connect+proto")], framed)
                }
            },
        ),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/rpc", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, upstream).await.unwrap();
    });
    let credentials = Credentials {
        endpoint: endpoint.clone(),
        api_key: "gateway-test-key".into(),
        device_id: "synthetic-device".into(),
        client_version: "test".into(),
    };
    let gateway = GatewayClient::new(&endpoint, &credentials.api_key).unwrap();
    (
        server::router_with_models(
            credentials,
            TOKEN.into(),
            gateway,
            if compatibility {
                Mode::Codex
            } else {
                Mode::Strict
            },
            models,
        ),
        calls,
        task,
    )
}

#[tokio::test]
async fn account_models_route_exact_uids_and_reject_unlisted_variants() {
    let models = Models::parse(&json!({"families":[{"variants":[
        {"model_uid":"MODEL_PRIVATE_11","label":"Legacy Claude"},
        {"model_uid":"fusion-claude-sidekick-swe","label":"Fusion"}
    ]}]}))
    .unwrap();
    let (app, calls, task) = app_with_models(true, None, models).await;
    for uid in ["MODEL_PRIVATE_11", "fusion-claude-sidekick-swe"] {
        let alias = format!("devin-{uid}");
        let response = app
            .clone()
            .oneshot(request(json!({"model":alias,"input":"hello"})))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
        let result: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(result["model"], alias);
        assert_eq!(result["x_devin_usage"]["actual_model_uid"], uid);
    }
    let response = app
        .oneshot(request(json!({"model":"devin-unlisted","input":"hello"})))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    task.abort();
}

fn request(body: Value) -> Request<Body> {
    Request::post("/v1/responses")
        .header("authorization", format!("Bearer {TOKEN}"))
        .header("chatgpt-account-id", "must-not-forward")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

#[tokio::test]
async fn authentication_and_semantic_rejection_precede_inference() {
    let (app, calls, task) = app().await;
    let mut unauthorized = request(json!({"model":"gpt-6-astra","input":"hello"}));
    unauthorized.headers_mut().remove("authorization");
    assert_eq!(
        app.clone().oneshot(unauthorized).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    let response=app.oneshot(request(json!({"model":"gpt-6-astra","input":[{"role":"developer","content":"private nonce"}],"tools":[{"type":"namespace","name":"functions"}]}))).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("private nonce"));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    task.abort();
}

#[tokio::test]
async fn authenticated_json_and_compressed_sse_roundtrip() {
    let (app, calls, task) = app().await;
    let response = app
        .clone()
        .oneshot(request(json!({"model":"gpt-6-astra","input":"hello"})))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap()).unwrap();
    assert_eq!(body["status"], "completed");
    assert_eq!(body["output"][0]["content"][0]["text"], "hello");
    let compressed = zstd::encode_all(
        json!({"model":"gpt-6-astra","input":"hello","stream":true})
            .to_string()
            .as_bytes(),
        1,
    )
    .unwrap();
    let request = Request::post("/v1/responses")
        .header("authorization", format!("Bearer {TOKEN}"))
        .header("content-encoding", "zstd")
        .body(Body::from(compressed))
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(text.contains("response.completed"));
    assert!(!text.contains("response.failed"));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    task.abort();
}

#[tokio::test]
async fn multiple_reasoning_items_sse_and_history_roundtrip() {
    let signature = json!([
        {"type":"reasoning","id":"rs_first","summary":[],"encrypted_content":"opaque-first"},
        {"type":"reasoning","id":"rs_second","summary":[],"encrypted_content":"opaque-second"}
    ]);
    for compatibility in [false, true] {
        let (app, calls, task) =
            app_with_signature(compatibility, Some(signature.to_string())).await;
        let response = app
            .clone()
            .oneshot(request(
                json!({"model":"gpt-6-astra","input":"hello","stream":true}),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        let events: Vec<Value> = text
            .lines()
            .filter_map(|line| line.strip_prefix("data: "))
            .map(|data| serde_json::from_str(data).unwrap())
            .collect();
        assert_eq!(events.last().unwrap()["type"], "response.completed");
        let mut input = events.last().unwrap()["response"]["output"]
            .as_array()
            .unwrap()
            .clone();
        assert_eq!(&input[..2], signature.as_array().unwrap());
        input.push(json!({"role":"user","content":"continue"}));
        let response = app
            .oneshot(request(json!({"model":"gpt-6-astra","input":input})))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap()).unwrap();
        assert_eq!(body["status"], "completed");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        task.abort();
    }
}

#[tokio::test]
async fn reasoning_signature_failures_have_safe_shape_diagnostics() {
    for (signature, reason, shape) in [
        (
            json!([]),
            "Empty reasoning signature cannot be replayed",
            "signature_items=0, reasoning_items=0, other_items=0",
        ),
        (
            json!([
                {"type":"reasoning","id":"rs_first","encrypted_content":"PRIVATE"},
                {"type":"PRIVATE","id":"PRIVATE","encrypted_content":"PRIVATE"}
            ]),
            "Invalid replayable reasoning identity",
            "signature_items=2, reasoning_items=1, other_items=1",
        ),
    ] {
        let (app, _, task) = app_with_signature(true, Some(signature.to_string())).await;
        for streaming in [true, false] {
            let response = app
                .clone()
                .oneshot(request(
                    json!({"model":"gpt-6-astra","input":"hello","stream":streaming}),
                ))
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                if streaming {
                    StatusCode::OK
                } else {
                    StatusCode::BAD_GATEWAY
                }
            );
            let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
            let text = String::from_utf8(bytes.to_vec()).unwrap();
            assert!(text.contains(reason));
            assert!(text.contains(shape));
            assert!(!text.contains("PRIVATE"));
            assert!(!text.contains("hello"));
            assert!(!text.contains("response.output_item.done"));
        }
        task.abort();
    }
}

#[tokio::test]
async fn collaboration_plaintext_sse_and_history_roundtrip() {
    let (app, calls, task) = app_with_mode(true).await;
    let mut body = json!({"model":"gpt-6-astra-devin","input":"delegate","stream":true,"tools":[{
        "type":"namespace","name":"collaboration","tools":[{
            "type":"function","name":"spawn_agent","parameters":{
                "type":"object","properties":{"message":{"type":"string","encrypted":true},"task_name":{"type":"string"}}
            }
        }]
    }]});
    let response = app.clone().oneshot(request(body.clone())).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    let events: Vec<Value> = text
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .map(|data| serde_json::from_str(data).unwrap())
        .collect();
    assert!(
        !events
            .iter()
            .any(|event| event["type"] == "response.failed")
    );
    for event in &events {
        if let Some(item) = event
            .get("item")
            .filter(|item| item["type"] == "function_call")
        {
            assert_eq!(item["namespace"], "collaboration");
            assert_eq!(item["encrypted_function_args"], json!([]));
        }
    }
    let response = &events.last().unwrap()["response"];
    assert_eq!(response["model"], "gpt-6-astra-devin");
    let call = &response["output"][0];
    assert_eq!(call["encrypted_function_args"], json!([]));
    body["stream"] = json!(false);
    body["input"] = json!([
        call,
        {"type":"function_call_output","call_id":"call-child","output":"spawned"},
        {"type":"agent_message","author":"/root/probe","recipient":"/root","content":[{"type":"input_text","text":"CHILD_OK"}]}
    ]);
    let response = app.clone().oneshot(request(body.clone())).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let response: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(response["status"], "completed");
    assert_eq!(response["model"], "gpt-6-astra-devin");
    let mut compact = body.clone();
    compact["tools"] = json!([]);
    compact["tool_choice"] = json!("none");
    compact["input"].as_array_mut().unwrap().insert(
        0,
        json!({
            "type":"reasoning","encrypted_content":"opaque-state","summary":[],"content":null
        }),
    );
    let response = app.clone().oneshot(request(compact)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    body["input"][2]["content"] =
        json!([{"type":"encrypted_content","encrypted_content":"opaque"}]);
    assert_eq!(
        app.oneshot(request(body)).await.unwrap().status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    task.abort();
}

#[tokio::test]
async fn stream_failures_preserve_safe_causes_without_partial_output() {
    let frame = |payload: Vec<u8>, flags: u8| {
        let mut bytes = vec![flags];
        bytes.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        bytes.extend(payload);
        bytes
    };
    let partial = frame(
        ChatResponse {
            delta_tool_calls: vec![ToolCall {
                id: "c1".into(),
                name: "run".into(),
                arguments_json: "PRIVATE".into(),
                ..Default::default()
            }],
            ..Default::default()
        }
        .encode_to_vec(),
        0,
    );
    for (status, body, code, message) in [
        (
            StatusCode::SERVICE_UNAVAILABLE,
            b"PRIVATE".to_vec(),
            "upstream_http_error",
            "HTTP 503",
        ),
        (
            StatusCode::OK,
            [
                partial.clone(),
                frame(
                    br#"{"error":{"code":"resource_exhausted","message":"PRIVATE"}}"#.to_vec(),
                    2,
                ),
            ]
            .concat(),
            "resource_exhausted",
            "resource_exhausted",
        ),
        (
            StatusCode::OK,
            [
                frame(
                    ChatResponse {
                        delta_text: "PRIVATE".into(),
                        phase: "unknown".into(),
                        ..Default::default()
                    }
                    .encode_to_vec(),
                    0,
                ),
                frame(b"{}".to_vec(), 2),
            ]
            .concat(),
            "response_translation_error",
            "Unsupported gateway message phase",
        ),
        (
            StatusCode::OK,
            [partial, frame(b"{}".to_vec(), 2)].concat(),
            "response_translation_error",
            "Incomplete function-call JSON",
        ),
    ] {
        let upstream = Router::new().route(
            "/rpc",
            post(move || {
                let body = body.clone();
                async move {
                    (
                        status,
                        [("content-type", "application/connect+proto")],
                        body,
                    )
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/rpc", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });
        let credentials = Credentials {
            endpoint: endpoint.clone(),
            api_key: "test-key".into(),
            device_id: String::new(),
            client_version: String::new(),
        };
        let app = server::router(
            credentials,
            TOKEN.into(),
            GatewayClient::new(&endpoint, "test-key").unwrap(),
        );
        for streaming in [true, false] {
            let response = app.clone().oneshot(request(json!({"model":"gpt-6-astra","input":"hello","stream":streaming,"tools":[{"type":"function","name":"run","parameters":{"type":"object"}}]}))).await.unwrap();
            assert_eq!(
                response.status(),
                if streaming {
                    StatusCode::OK
                } else {
                    StatusCode::BAD_GATEWAY
                }
            );
            let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
            let text = String::from_utf8(bytes.to_vec()).unwrap();
            assert!(!text.contains("PRIVATE"), "Failure leaked upstream content");
            assert!(!text.contains("response.output_item.done"));
            let response: Value = if streaming {
                let events: Vec<Value> = text
                    .lines()
                    .filter_map(|line| line.strip_prefix("data: "))
                    .map(|data| serde_json::from_str(data).unwrap())
                    .collect();
                assert_eq!(events.last().unwrap()["type"], "response.failed");
                events.last().unwrap()["response"].clone()
            } else {
                serde_json::from_str(&text).unwrap()
            };
            assert_eq!(response["error"]["code"], code);
            assert!(
                response["error"]["message"]
                    .as_str()
                    .unwrap()
                    .contains(message)
            );
            assert_eq!(response["output"], json!([]));
        }
        task.abort();
    }
}

#[tokio::test]
async fn explicit_unsupported_routes_and_websocket_fallback() {
    let (app, _, task) = app().await;
    let req = Request::get("/v1/responses")
        .header("authorization", format!("Bearer {TOKEN}"))
        .header("upgrade", "websocket")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app.clone().oneshot(req).await.unwrap().status(),
        StatusCode::UPGRADE_REQUIRED
    );
    let req = Request::post("/v1/responses/compact")
        .header("authorization", format!("Bearer {TOKEN}"))
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app.oneshot(req).await.unwrap().status(),
        StatusCode::NOT_IMPLEMENTED
    );
    task.abort();
}

#[tokio::test]
async fn dropping_downstream_sse_cancels_gateway_response() {
    use futures_util::StreamExt;
    use std::{convert::Infallible, time::Duration};
    use tokio::sync::Notify;
    struct OnDrop(Arc<Notify>);
    impl Drop for OnDrop {
        fn drop(&mut self) {
            self.0.notify_one();
        }
    }
    let disconnected = Arc::new(Notify::new());
    let connected = Arc::new(Notify::new());
    let closed = disconnected.clone();
    let started = connected.clone();
    let upstream = Router::new().route(
        "/rpc",
        post(move || {
            let guard = OnDrop(closed.clone());
            let started = started.clone();
            async move {
                let output = async_stream::stream! {
                    let _guard=guard;
                    started.notify_one();
                    // Keep emitting small valid frames until the adapter drops us.
                    loop {
                        yield Ok::<_,Infallible>(axum::body::Bytes::from_static(&[0,0,0,0,0]));
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                };
                (
                    [("content-type", "application/connect+proto")],
                    Body::from_stream(output),
                )
            }
        }),
    );
    let upstream_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/rpc", upstream_listener.local_addr().unwrap());
    let upstream_task =
        tokio::spawn(async move { axum::serve(upstream_listener, upstream).await.unwrap() });
    let credentials = Credentials {
        endpoint: endpoint.clone(),
        api_key: "test-key".into(),
        device_id: "test".into(),
        client_version: "test".into(),
    };
    let router = server::router(
        credentials,
        TOKEN.into(),
        GatewayClient::new(&endpoint, "test-key").unwrap(),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1/responses", listener.local_addr().unwrap());
    let adapter_task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = reqwest::Client::new();
    let response = client
        .post(url)
        .bearer_auth(TOKEN)
        .json(&json!({"model":"gpt-6-astra","input":"hello","stream":true}))
        .send()
        .await
        .unwrap();
    let mut stream = response.bytes_stream();
    assert!(stream.next().await.unwrap().is_ok());
    tokio::time::timeout(Duration::from_secs(2), connected.notified())
        .await
        .unwrap();
    drop(stream);
    tokio::time::timeout(Duration::from_secs(2), disconnected.notified())
        .await
        .unwrap();
    adapter_task.abort();
    upstream_task.abort();
}
