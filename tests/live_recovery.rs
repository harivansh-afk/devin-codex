//! Opt-in real gateway check. Synthetic tool calls are validated, never executed.
use axum::{
    Router,
    body::{Body, Bytes},
    http::HeaderMap,
    response::Response,
    routing::post,
};
use devin_codex::{Mode, config, models::Models, session::Session, smoke};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[tokio::test]
#[ignore = "Requires Devin login and spends inference credits; cargo test --test live_recovery -- --ignored"]
async fn live_tool_round_trip_recovers_from_injected_unknown_and_http_503() {
    let credentials = config::load_credentials(None).expect("Devin login required");
    let endpoint = credentials.endpoint.clone();
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let upstream = Router::new().route(
        "/rpc",
        post(move |headers: HeaderMap, body: Bytes| {
            let attempt = counter.fetch_add(1, Ordering::SeqCst);
            let endpoint = endpoint.clone();
            let client = client.clone();
            async move {
                if attempt == 0 {
                    let trailer =
                        br#"{"error":{"code":"unknown","message":"synthetic transient failure"}}"#;
                    let mut framed = vec![2];
                    framed.extend_from_slice(&(trailer.len() as u32).to_be_bytes());
                    framed.extend_from_slice(trailer);
                    return Response::builder()
                        .header("content-type", "application/connect+proto")
                        .body(Body::from(framed))
                        .unwrap();
                }
                if attempt == 2 {
                    return Response::builder().status(503).body(Body::empty()).unwrap();
                }
                let upstream = client
                    .post(endpoint)
                    .header("authorization", headers["authorization"].clone())
                    .header("content-type", "application/connect+proto")
                    .header("connect-protocol-version", "1")
                    .header("connect-accept-encoding", "gzip")
                    .header("accept-encoding", "identity")
                    .body(body)
                    .send()
                    .await
                    .unwrap();
                let mut response = Response::builder().status(upstream.status());
                for name in [
                    "content-type",
                    "content-encoding",
                    "connect-content-encoding",
                    "retry-after",
                ] {
                    if let Some(value) = upstream.headers().get(name) {
                        response = response.header(name, value);
                    }
                }
                response
                    .body(Body::from_stream(upstream.bytes_stream()))
                    .unwrap()
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy = format!("http://{}/rpc", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });
    let mut credentials = credentials;
    credentials.endpoint = proxy;
    let session = Session::start(credentials, Mode::Strict, Models::default())
        .await
        .unwrap();
    let result = smoke::run(&session).await;
    task.abort();
    result.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 4);
}
