use super::*;

#[test]
fn prevents_plaintext_remote_auth() {
    assert!(GatewayClient::new("http://example.com/rpc", "key").is_err());
    assert!(GatewayClient::new("https://secret@example.com/rpc", "key").is_err());
    assert!(GatewayClient::new("https://example.com/rpc", "bad\nkey").is_err());
}

#[tokio::test]
async fn active_stream_can_outlive_timeout_but_idle_stream_cannot() {
    use axum::{Router, body::Body, response::Response, routing::post};
    let upstream = Router::new()
        .route("/active", post(|| async {
            let stream = async_stream::stream! {
                for _ in 0..16 {
                    yield Ok::<_, std::io::Error>(frame(&ChatResponse { delta_text:"x".into(), ..Default::default() }.encode_to_vec(), 0));
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
                yield Ok(frame(b"{}", 2));
            };
            Response::builder().header("content-type", "application/connect+proto")
                .body(Body::from_stream(stream)).unwrap()
        }))
        .route("/idle", post(|| async {
            let stream = async_stream::stream! {
                yield Ok::<_, std::io::Error>(frame(&ChatResponse::default().encode_to_vec(), 0));
                std::future::pending::<()>().await;
            };
            Response::builder().header("content-type", "application/connect+proto")
                .body(Body::from_stream(stream)).unwrap()
        }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });
    let client = |path| {
        GatewayClient::with_read_timeout(
            &format!("http://{address}/{path}"),
            "test-key",
            Duration::from_millis(200),
        )
        .unwrap()
    };
    let frames = client("active")
        .stream(ChatRequest::default())
        .collect::<Vec<_>>()
        .await;
    assert_eq!(frames.len(), 16, "{frames:?}");
    assert!(frames.iter().all(Result::is_ok), "{frames:?}");
    let frames = tokio::time::timeout(
        Duration::from_secs(3),
        client("idle")
            .stream(ChatRequest::default())
            .collect::<Vec<_>>(),
    )
    .await
    .unwrap();
    assert_eq!(
        frames.last().unwrap().as_ref().unwrap_err().code,
        "upstream_timeout"
    );
    task.abort();
}

#[tokio::test]
async fn http_auth_errors_and_drop_cancel_upstream() {
    use axum::{Router, body::Body, response::Response, routing::post};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let closed = Arc::new(AtomicBool::new(false));
    struct OnDrop(Arc<AtomicBool>);
    impl Drop for OnDrop {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
    let watched = closed.clone();
    let app=Router::new().route("/ok",post(|headers: axum::http::HeaderMap| async move {
        assert_eq!(headers["authorization"],"Basic test-key-test-key");
        Response::builder().header("content-type","application/connect+proto").body(Body::from([frame(&ChatResponse {delta_text:"hello".into(),..Default::default()}.encode_to_vec(),0),frame(b"{}",2)].concat())).unwrap()
    })).route("/fail",post(|| async {
        Response::builder().header("content-type","application/connect+proto").body(Body::from(frame(br#"{"error":{"code":"internal","message":"private"}}"#,2))).unwrap()
    })).route("/redirect",post(|| async {
        Response::builder().status(307).header("location","http://127.0.0.1:1/leak").body(Body::empty()).unwrap()
    })).route("/slow",post(move || {
        let watched=watched.clone();
        async move {
            let stream=async_stream::stream! {
                let _guard=OnDrop(watched);
                loop {
                    yield Ok::<_,std::io::Error>(bytes::Bytes::from(frame(&ChatResponse {delta_text:"hello".into(),..Default::default()}.encode_to_vec(),0)));
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            };
            Response::builder().header("content-type","application/connect+proto").body(Body::from_stream(stream)).unwrap()
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let make =
        |path: &str| GatewayClient::new(&format!("http://{address}/{path}"), "test-key").unwrap();
    let output = make("ok")
        .stream(ChatRequest::default())
        .collect::<Vec<_>>()
        .await;
    assert_eq!(output.len(), 1);
    assert_eq!(output[0].as_ref().unwrap().delta_text, "hello");
    let error = make("fail")
        .stream(ChatRequest::default())
        .next()
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(error.code, "internal");
    assert!(!error.to_string().contains("private"));
    let error = make("redirect")
        .stream(ChatRequest::default())
        .next()
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(error.status, Some(307));
    let mut stream = make("slow").stream(ChatRequest::default());
    assert!(stream.next().await.unwrap().is_ok());
    drop(stream);
    tokio::time::timeout(Duration::from_secs(1), async {
        while !closed.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    task.abort();
}
