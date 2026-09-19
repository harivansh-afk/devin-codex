//! Authenticated loopback HTTP boundary. Dropping an SSE body drops inference.
use crate::{
    config::Credentials, gateway::GatewayClient, request::translate_request, responses::Translator,
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
use futures_util::StreamExt;
use serde_json::{Value, json};
use std::{convert::Infallible, io::Read, sync::Arc};
use subtle::ConstantTimeEq;

pub const MAX_BODY: usize = 16 * 1024 * 1024;

struct AppState {
    credentials: Credentials,
    authorization: Vec<u8>,
    gateway: GatewayClient,
    compatibility: bool,
}

pub fn router(credentials: Credentials, token: String, gateway: GatewayClient) -> Router {
    router_with_mode(credentials, token, gateway, false)
}

pub fn router_with_mode(
    credentials: Credentials,
    token: String,
    gateway: GatewayClient,
    compatibility: bool,
) -> Router {
    let state = Arc::new(AppState {
        credentials,
        authorization: format!("Bearer {token}").into_bytes(),
        gateway,
        compatibility,
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
    (status, Json(json!({"error": {"type": if status.is_server_error() { "server_error" } else { "invalid_request_error" }, "code": code, "message": message}}))).into_response()
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
    Json(
        json!({"status":"ok", "mode":if state.compatibility {"compatibility"} else {"verified-subset"}, "codex_parity":false}),
    )
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
        data
    } else {
        raw.to_vec()
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
    let prepared = if state.compatibility {
        crate::compat::Compatibility::prepare(&body)
    } else {
        Ok((body.clone(), crate::compat::Compatibility::default()))
    };
    let translated = prepared.and_then(|(request, mapping)| {
        translate_request(&request, &state.credentials).map(|translated| (translated, mapping))
    });
    let (translated, mapping) = match translated {
        Ok(value) => value,
        Err(problems) => return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": {
                "type":"invalid_request_error", "code":"unsupported_semantics",
                "message":format!("Unsupported request: {}", problems.iter().take(3).map(|p| format!("{}: {}", p.path, p.reason)).collect::<Vec<_>>().join("; ")), "unsupported":problems
            }})),
        )
            .into_response(),
    };
    let streaming = body.get("stream").and_then(Value::as_bool).unwrap_or(false);
    let mut translator = Translator::new(
        translated.model,
        translated.tool_map,
        translated.allow_tools,
    );
    translator.compatibility = state.compatibility;
    let frames = state.gateway.stream(translated.request);
    let events = async_stream::stream! {
        for event in translator.created_events() { yield event; }
        futures_util::pin_mut!(frames);
        let mut failure = None;
        while let Some(frame) = frames.next().await {
            match frame {
                Ok(frame) => match translator.push(frame) {
                    Ok(()) => {},
                    Err(error) => {
                        failure = Some(("response_translation_error".to_owned(), format!("Adapter rejected gateway frame: {error}")));
                        break;
                    }
                },
                Err(error) => {
                    let status = error.status.map(|status| format!(" (HTTP {status})")).unwrap_or_default();
                    let message = format!("Devin gateway failed: {}{status}", error.code);
                    failure = Some((error.code, message));
                    break;
                }
            }
        }
        if let Some((code, message)) = failure { yield translator.failed_event(&code, &message); }
        else {
            match translator.finish() {
                Ok(events) => for event in events { yield event; },
                Err(error) => yield translator.failed_event("response_translation_error", &format!("Adapter rejected completed gateway response: {error}")),
            }
        }
    };
    let events = events.map(move |mut event| {
        mapping.restore(&mut event);
        event
    });
    if streaming {
        let stream = events.map(|event| {
            let kind = event
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("response.failed")
                .to_owned();
            Ok::<_, Infallible>(Event::default().event(kind).data(event.to_string()))
        });
        let mut response = Sse::new(stream)
            .keep_alive(axum::response::sse::KeepAlive::default())
            .into_response();
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
        response
            .headers_mut()
            .insert("x-accel-buffering", "no".parse().unwrap());
        response
    } else {
        futures_util::pin_mut!(events);
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
}

#[cfg(test)]
mod tests {
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
}
