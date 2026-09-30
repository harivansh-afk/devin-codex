//! Authenticated loopback HTTP boundary. Dropping an SSE body drops inference.
use crate::{
    Mode, adapter::Adapter, config::Credentials, gateway::GatewayClient, models::Models,
    request::Problem,
};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, State, rejection::BytesRejection},
    http::{HeaderMap, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response, Sse, sse::Event},
    routing::{get, post},
};
use futures_util::{StreamExt, stream::BoxStream};
use serde_json::{Value, json};
use std::{convert::Infallible, io::Read, sync::Arc};
use subtle::ConstantTimeEq;

pub const MAX_BODY: usize = 16 * 1024 * 1024;

#[cfg(test)]
#[path = "../tests/unit/server.rs"]
mod tests;

struct AppState {
    authorization: Vec<u8>,
    adapter: Adapter,
}

pub fn router(
    credentials: Credentials,
    token: String,
    gateway: GatewayClient,
    mode: Mode,
    models: Models,
) -> Router {
    let state = Arc::new(AppState {
        authorization: format!("Bearer {token}").into_bytes(),
        adapter: Adapter::with_models(credentials, gateway, mode, models),
    });
    Router::new()
        .route("/healthz", get(health))
        .route("/v1/responses", post(respond).get(unsupported))
        .route("/responses", post(respond).get(unsupported))
        .route(
            "/backend-api/codex/responses",
            post(respond).get(unsupported),
        )
        .fallback(unsupported)
        .layer(DefaultBodyLimit::max(MAX_BODY))
        .layer(middleware::from_fn_with_state(state.clone(), authenticate))
        .with_state(state)
}

fn error(status: StatusCode, code: &str, message: &str) -> Response {
    let kind = if status.is_server_error() {
        "server_error"
    } else {
        "invalid_request_error"
    };
    (
        status,
        Json(json!({"error":{"type":kind,"code":code,"message":message}})),
    )
        .into_response()
}

fn unsupported_semantics(problems: Vec<Problem>) -> Response {
    let message = problems
        .iter()
        .take(3)
        .map(|p| format!("{}: {}", p.path, p.reason))
        .collect::<Vec<_>>()
        .join("; ");
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({"error": {
            "type":"invalid_request_error", "code":"unsupported_semantics",
            "message":format!("Unsupported request: {message}"), "unsupported":problems
        }})),
    )
        .into_response()
}

async fn authenticate(
    State(state): State<Arc<AppState>>,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    let actual = request
        .headers()
        .get(header::AUTHORIZATION)
        .map(|v| v.as_bytes())
        .unwrap_or_default();
    if !bool::from(actual.ct_eq(&state.authorization)) {
        return error(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "A local adapter token is required.",
        );
    }
    next.run(request).await
}

async fn health(State(state): State<Arc<AppState>>) -> Json<Value> {
    let mode = match state.adapter.mode {
        Mode::Strict => "verified-subset",
        Mode::Codex => "compatibility",
    };
    Json(json!({"status":"ok", "mode":mode, "codex_parity":false}))
}

async fn unsupported(headers: HeaderMap) -> Response {
    if headers
        .get(header::UPGRADE)
        .is_some_and(|v| v.as_bytes().eq_ignore_ascii_case(b"websocket"))
    {
        error(
            StatusCode::UPGRADE_REQUIRED,
            "sse_required",
            "Use HTTP Responses with SSE streaming.",
        )
    } else {
        error(
            StatusCode::NOT_IMPLEMENTED,
            "unsupported_endpoint",
            "Only the Responses endpoint is implemented.",
        )
    }
}

fn decode_body(raw: Bytes, compressed: bool) -> Result<Value, (StatusCode, &'static str)> {
    let data = if compressed {
        let decoder = zstd::stream::read::Decoder::new(raw.as_ref())
            .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid zstd body."))?;
        let mut data = Vec::new();
        decoder
            .take((MAX_BODY + 1) as u64)
            .read_to_end(&mut data)
            .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid zstd body."))?;
        if data.len() > MAX_BODY {
            return Err((
                StatusCode::PAYLOAD_TOO_LARGE,
                "Decoded request exceeds size limit.",
            ));
        }
        Bytes::from(data)
    } else {
        raw
    };
    serde_json::from_slice(&data)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Request must be valid JSON."))
}

async fn respond(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    raw: Result<Bytes, BytesRejection>,
) -> Response {
    let compressed = match headers.get(header::CONTENT_ENCODING).map(|v| v.as_bytes()) {
        None | Some(b"identity") => false,
        Some(b"zstd") => true,
        _ => {
            return error(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "unsupported_encoding",
                "Only identity and zstd request bodies are supported.",
            );
        }
    };
    let raw = match raw {
        Ok(raw) => raw,
        Err(_) => {
            return error(
                StatusCode::PAYLOAD_TOO_LARGE,
                "request_body_error",
                "Cannot read request within size limit.",
            );
        }
    };
    let body = match tokio::task::spawn_blocking(move || decode_body(raw, compressed)).await {
        Ok(Ok(body)) => body,
        Ok(Err((status, message))) => return error(status, "invalid_body", message),
        Err(_) => {
            return error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "decode_failed",
                "Request decoding failed.",
            );
        }
    };
    let prepared = match state.adapter.prepare(body) {
        Ok(prepared) => prepared,
        Err(problems) => return unsupported_semantics(problems),
    };
    let streaming = prepared.streaming;
    let events = state.adapter.stream(prepared);
    if streaming {
        streaming_response(events)
    } else {
        collected_response(events).await
    }
}

fn streaming_response(events: BoxStream<'static, Value>) -> Response {
    let stream = events.map(|event| {
        let kind = event["type"].as_str().unwrap_or("response.failed");
        Ok::<_, Infallible>(Event::default().event(kind).data(event.to_string()))
    });
    let mut response = Sse::new(stream)
        // Codex times out waiting for parsed SSE events; comments do not reset it.
        // A transport-only event keeps buffered generations and retries alive
        // without exposing partial model output or executable tool calls.
        .keep_alive(
            axum::response::sse::KeepAlive::default()
                .event(Event::default().event("ping").data(r#"{"type":"ping"}"#)),
        )
        .into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    response
        .headers_mut()
        .insert("x-accel-buffering", "no".parse().unwrap());
    response
}

async fn collected_response(mut events: BoxStream<'static, Value>) -> Response {
    let mut final_response = None;
    while let Some(event) = events.next().await {
        if matches!(
            event["type"].as_str(),
            Some("response.completed" | "response.failed" | "response.incomplete")
        ) {
            final_response = event.get("response").cloned();
        }
    }
    match final_response {
        Some(response) => {
            let status = if response["status"] == "failed" {
                StatusCode::BAD_GATEWAY
            } else {
                StatusCode::OK
            };
            (status, Json(response)).into_response()
        }
        None => error(
            StatusCode::BAD_GATEWAY,
            "missing_response",
            "Upstream ended without a response.",
        ),
    }
}
