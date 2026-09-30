//! Offline regression using the installed Codex binary; no real inference or tools.
//! cargo test --test idle_reproduction -- --ignored --nocapture
use axum::{
    Router,
    body::{Body, Bytes},
    response::Response,
    routing::post,
};
use devin_codex::{
    Mode, config::Credentials, gateway::GatewayClient, models::Models, proto::ChatResponse, server,
};
use futures_util::StreamExt;
use prost::Message;
use serde_json::json;
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

fn frame(payload: &[u8], flags: u8) -> Bytes {
    let mut bytes = vec![flags];
    bytes.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    bytes.extend_from_slice(payload);
    bytes.into()
}

#[tokio::test]
#[ignore = "Runs installed Codex against a synthetic local gateway, about 65 seconds"]
async fn data_heartbeats_keep_codex_alive_during_buffered_generation() {
    let upstream_frames = Arc::new(AtomicUsize::new(0));
    let counter = upstream_frames.clone();
    let upstream = Router::new().route("/rpc", post(move |_: Bytes| {
        let counter = counter.clone();
        async move {
            let stream = async_stream::stream! {
                // Healthy upstream activity every second, total generation 22 seconds.
                for _ in 0..22 {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    counter.fetch_add(1, Ordering::SeqCst);
                    yield Ok::<_, std::io::Error>(frame(&ChatResponse { delta_text: "waiting ".into(), ..Default::default() }.encode_to_vec(), 0));
                }
                yield Ok(frame(&ChatResponse { delta_text: "IDLE_REPRO_OK".into(), stop_reason: Some(2), ..Default::default() }.encode_to_vec(), 0));
                yield Ok(frame(b"{}", 2));
            };
            Response::builder().header("content-type", "application/connect+proto").body(Body::from_stream(stream)).unwrap()
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/rpc", listener.local_addr().unwrap());
    let upstream_task = tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });
    let token = "synthetic-idle-reproduction-token";
    let app = server::router(
        Credentials {
            endpoint: endpoint.clone(),
            api_key: "synthetic".into(),
            device_id: "test".into(),
            client_version: "test".into(),
        },
        token.into(),
        GatewayClient::new(&endpoint, "synthetic").unwrap(),
        Mode::Codex,
        Models::default(),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let adapter_url = format!("http://{}/v1/responses", listener.local_addr().unwrap());
    let adapter_task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let temp = std::env::temp_dir().join(format!("codex-idle-repro-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&temp).unwrap();
    let config: toml::Value = toml::from_str(
        &std::fs::read_to_string(
            std::path::Path::new(&std::env::var("HOME").unwrap()).join(".codex/config.toml"),
        )
        .unwrap(),
    )
    .unwrap_or_else(|_| panic!("Cannot parse local Codex configuration"));
    for (case, data_heartbeat, timeout_ms, expect_success) in [
        ("comments", false, 18000, false),
        ("data_heartbeat", true, 18000, true),
        ("longer_timeout", false, 30000, true),
    ] {
        let observed_heartbeats = Arc::new(AtomicUsize::new(0));
        let heartbeats = observed_heartbeats.clone();
        let destination = adapter_url.clone();
        let proxy = Router::new().route(
            "/v1/responses",
            post(move |body: Bytes| {
                let destination = destination.clone();
                let heartbeats = heartbeats.clone();
                async move {
                    let upstream = reqwest::Client::new()
                        .post(destination)
                        .bearer_auth(token)
                        .header("content-type", "application/json")
                        .body(body)
                        .send()
                        .await
                        .unwrap();
                    let stream = async_stream::stream! {
                        let mut chunks = upstream.bytes_stream();
                        while let Some(chunk) = chunks.next().await {
                            let chunk = chunk.unwrap();
                            if chunk.as_ref() == b"event: ping\ndata: {\"type\":\"ping\"}\n\n" {
                                heartbeats.fetch_add(1, Ordering::SeqCst);
                                if !data_heartbeat {
                                    // Recreate the pre-fix wire format as a negative control.
                                    yield Ok::<_, std::io::Error>(Bytes::from_static(b":\n\n"));
                                    continue;
                                }
                            }
                            yield Ok(chunk);
                        }
                    };
                    Response::builder()
                        .header("content-type", "text/event-stream")
                        .body(Body::from_stream(stream))
                        .unwrap()
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
        let proxy_task = tokio::spawn(async move { axum::serve(listener, proxy).await.unwrap() });
        let mut cmd = tokio::process::Command::new("codex");
        cmd.kill_on_drop(true)
            .args([
                "exec",
                "--ephemeral",
                "--skip-git-repo-check",
                "--json",
                "-C",
            ])
            .arg(&temp);
        for setting in [
            "model_provider=\"idle_repro\"".to_owned(),
            "model=\"gpt-6-astra-devin\"".into(),
            "web_search=\"disabled\"".into(),
            "developer_instructions=\"\"".into(),
            format!(
                "model_providers.idle_repro={{name=\"Offline reproduction\",base_url=\"{base_url}\",wire_api=\"responses\",requires_openai_auth=false,supports_websockets=false,request_max_retries=0,stream_max_retries=0,stream_idle_timeout_ms={timeout_ms}}}"
            ),
        ] {
            cmd.args(["-c", &setting]);
        }
        if let Some(servers) = config.get("mcp_servers").and_then(toml::Value::as_table) {
            for name in servers.keys() {
                cmd.args(["-c", &format!("mcp_servers.{name}.enabled=false")]);
            }
        }
        cmd.arg("Reply IDLE_REPRO_OK. Do not use tools.");
        let before = upstream_frames.load(Ordering::SeqCst);
        let started = Instant::now();
        let result = tokio::time::timeout(Duration::from_secs(40), cmd.output())
            .await
            .unwrap()
            .unwrap();
        let stdout = String::from_utf8_lossy(&result.stdout);
        let stderr = String::from_utf8_lossy(&result.stderr);
        println!(
            "{}",
            json!({"case":case,"elapsed_ms":started.elapsed().as_millis(),"exit":result.status.code(),"adapter_data_heartbeats":observed_heartbeats.load(Ordering::SeqCst),"upstream_frames":upstream_frames.load(Ordering::SeqCst)-before,"exact_idle_error":stdout.contains("idle timeout waiting for SSE") || stderr.contains("idle timeout waiting for SSE"),"completed":stdout.contains("IDLE_REPRO_OK")})
        );
        if result.status.success() != expect_success {
            println!("stdout={stdout}\nstderr={stderr}");
        }
        assert_eq!(result.status.success(), expect_success);
        assert!(observed_heartbeats.load(Ordering::SeqCst) > 0);
        if expect_success {
            assert!(stdout.contains("IDLE_REPRO_OK"));
        } else {
            assert!(
                stdout.contains("idle timeout waiting for SSE")
                    || stderr.contains("idle timeout waiting for SSE")
            );
        }
        proxy_task.abort();
    }
    adapter_task.abort();
    upstream_task.abort();
    std::fs::remove_dir_all(temp).unwrap();
}
