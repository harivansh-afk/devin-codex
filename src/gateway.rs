//! Bounded Connect framing and cancellable, credential-isolated HTTP transport.
use crate::proto::{ChatRequest, ChatResponse};
use bytes::{Buf, BytesMut};
use futures_util::{Stream, StreamExt};
use prost::Message;
use std::{io::Read, pin::Pin, time::Duration};

const MAX_FRAME: usize = 16 * 1024 * 1024;
#[derive(Debug, Clone, thiserror::Error)]
#[error("gateway request failed ({code})")]
pub struct GatewayError {
    pub code: String,
    pub status: Option<u16>,
}
impl GatewayError {
    pub fn new(code: &str) -> Self {
        Self {
            code: code.into(),
            status: None,
        }
    }
}
fn protocol() -> GatewayError {
    GatewayError::new("upstream_protocol_error")
}

#[derive(Clone)]
pub struct GatewayClient {
    client: reqwest::Client,
    endpoint: reqwest::Url,
    auth: reqwest::header::HeaderValue,
}
impl GatewayClient {
    pub fn new(endpoint: &str, api_key: &str) -> Result<Self, GatewayError> {
        let url =
            reqwest::Url::parse(endpoint).map_err(|_| GatewayError::new("invalid_endpoint"))?;
        if !matches!(url.scheme(), "https" | "http")
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
            || url.query().is_some()
            || (url.scheme() == "http"
                && !matches!(url.host_str(), Some("127.0.0.1" | "[::1]" | "localhost")))
        {
            return Err(GatewayError::new("invalid_endpoint"));
        }
        if api_key.is_empty() || !api_key.bytes().all(|b| (33..=126).contains(&b)) {
            return Err(GatewayError::new("invalid_credentials"));
        }
        let mut auth =
            reqwest::header::HeaderValue::from_str(&format!("Basic {api_key}-{api_key}"))
                .map_err(|_| GatewayError::new("invalid_credentials"))?;
        auth.set_sensitive(true);
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .timeout(Duration::from_secs(120))
            .no_proxy()
            .build()
            .map_err(|_| GatewayError::new("tls_configuration_error"))?;
        Ok(Self {
            client,
            endpoint: url,
            auth,
        })
    }
    pub fn stream(
        &self,
        request: ChatRequest,
    ) -> Pin<Box<dyn Stream<Item = Result<ChatResponse, GatewayError>> + Send + 'static>> {
        let this = self.clone();
        Box::pin(async_stream::try_stream! {
            if request.encoded_len() > MAX_FRAME { Err(GatewayError::new("request_too_large"))?; }
            let payload = request.encode_to_vec();
            let response = this.client.post(this.endpoint).header("Authorization", this.auth)
                .header("Content-Type", "application/connect+proto").header("Connect-Protocol-Version", "1")
                .header("Connect-Accept-Encoding", "gzip").header("Accept-Encoding", "identity")
                .body(frame(&payload, 0)).send().await.map_err(http_error)?;
            if response.status() != reqwest::StatusCode::OK {
                Err(GatewayError { code:"upstream_http_error".into(), status:Some(response.status().as_u16()) })?;
            }
            let content_type = response.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").split(';').next().unwrap_or("").trim();
            if content_type != "application/connect+proto" { Err(protocol())?; }
            if response.headers().get("content-encoding").is_some_and(|v| v != "identity") { Err(protocol())?; }
            let compression = match response.headers().get("connect-content-encoding").map(|v| v.to_str()) {
                None | Some(Ok("identity")) => false,
                Some(Ok("gzip")) => true,
                _ => { Err(protocol())?; false }
            };
            let mut parser = ConnectParser::new(MAX_FRAME, compression);
            let mut upstream = response.bytes_stream();
            while let Some(chunk) = upstream.next().await {
                let chunk = chunk.map_err(http_error)?;
                // Bound temporary buffering even if the HTTP library coalesces a large chunk.
                for part in chunk.chunks(65536) {
                    for message in parser.feed(part)? {
                        yield ChatResponse::decode(message.as_slice()).map_err(|_| protocol())?;
                    }
                }
            }
            parser.finish()?;
        })
    }
}
fn http_error(error: reqwest::Error) -> GatewayError {
    GatewayError::new(if error.is_timeout() {
        "upstream_timeout"
    } else {
        "upstream_connection_error"
    })
}
fn frame(payload: &[u8], flags: u8) -> Vec<u8> {
    let mut output = Vec::with_capacity(payload.len() + 5);
    output.push(flags);
    output.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    output.extend_from_slice(payload);
    output
}

struct ConnectParser {
    buffer: BytesMut,
    limit: usize,
    gzip: bool,
    ended: bool,
}
impl ConnectParser {
    fn new(limit: usize, gzip: bool) -> Self {
        Self {
            buffer: BytesMut::new(),
            limit,
            gzip,
            ended: false,
        }
    }
    fn feed(&mut self, bytes: &[u8]) -> Result<Vec<Vec<u8>>, GatewayError> {
        if self.ended && !bytes.is_empty() {
            return Err(protocol());
        }
        self.buffer.extend_from_slice(bytes);
        let mut output = Vec::new();
        while self.buffer.len() >= 5 {
            let flags = self.buffer[0];
            let length = u32::from_be_bytes(self.buffer[1..5].try_into().unwrap()) as usize;
            if flags & !3 != 0 || length > self.limit {
                return Err(protocol());
            }
            if self.buffer.len() < length + 5 {
                break;
            }
            self.buffer.advance(5);
            let mut payload = self.buffer.split_to(length).to_vec();
            if flags & 1 != 0 {
                if !self.gzip {
                    return Err(protocol());
                }
                let mut decoder = flate2::bufread::GzDecoder::new(payload.as_slice());
                let mut inflated = Vec::new();
                decoder
                    .by_ref()
                    .take(self.limit as u64 + 1)
                    .read_to_end(&mut inflated)
                    .map_err(|_| protocol())?;
                if inflated.len() > self.limit || !decoder.get_ref().is_empty() {
                    return Err(protocol());
                }
                payload = inflated;
            }
            if flags & 2 != 0 {
                let trailer: serde_json::Value =
                    serde_json::from_slice(&payload).map_err(|_| protocol())?;
                let object = trailer.as_object().ok_or_else(protocol)?;
                if let Some(error) = object.get("error") {
                    let code = error
                        .get("code")
                        .and_then(|v| v.as_str())
                        .ok_or_else(protocol)?;
                    if !matches!(
                        code,
                        "canceled"
                            | "unknown"
                            | "invalid_argument"
                            | "deadline_exceeded"
                            | "not_found"
                            | "already_exists"
                            | "permission_denied"
                            | "resource_exhausted"
                            | "failed_precondition"
                            | "aborted"
                            | "out_of_range"
                            | "unimplemented"
                            | "internal"
                            | "unavailable"
                            | "data_loss"
                            | "unauthenticated"
                    ) {
                        return Err(protocol());
                    }
                    return Err(GatewayError::new(code));
                }
                self.ended = true;
                if !self.buffer.is_empty() {
                    return Err(protocol());
                }
            } else {
                output.push(payload);
            }
        }
        Ok(output)
    }
    fn finish(&self) -> Result<(), GatewayError> {
        if !self.ended || !self.buffer.is_empty() {
            Err(protocol())
        } else {
            Ok(())
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    fn gzip(value: &[u8]) -> Vec<u8> {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(value).unwrap();
        encoder.finish().unwrap()
    }
    #[test]
    fn fragmented_coalesced() {
        let data = [frame(b"first", 0), frame(b"second", 0), frame(b"{}", 2)].concat();
        for size in [1, 2, 5, 13, 65536] {
            let mut parser = ConnectParser::new(100, false);
            let mut output = Vec::new();
            for part in data.chunks(size) {
                output.extend(parser.feed(part).unwrap());
            }
            parser.finish().unwrap();
            assert_eq!(output, vec![b"first".to_vec(), b"second".to_vec()]);
        }
    }
    #[test]
    fn compressed_frames_and_trailer() {
        let mut parser = ConnectParser::new(100, true);
        assert_eq!(
            parser
                .feed(&[frame(&gzip(b"hello"), 1), frame(&gzip(b"{}"), 3)].concat())
                .unwrap(),
            vec![b"hello".to_vec()]
        );
        parser.finish().unwrap();
    }
    #[test]
    fn error_trailers_and_truncation_fail() {
        for input in [
            frame(b"hello", 0),
            frame(b"[]", 2),
            frame(b"broken", 2),
            frame(br#"{"error":{"code":"internal","message":"secret"}}"#, 2),
            frame(br#"{"error":null}"#, 2),
            [frame(b"{}", 2), frame(b"late", 0)].concat(),
            vec![0, 0, 0],
            vec![4, 0, 0, 0, 0],
        ] {
            let mut parser = ConnectParser::new(100, false);
            let result = parser.feed(&input).and_then(|_| parser.finish());
            assert!(result.is_err());
            assert!(!result.unwrap_err().to_string().contains("secret"));
        }
    }
    #[test]
    fn compression_size_and_trailing_bytes_fail() {
        let compressed = gzip(&vec![b'x'; 1000]);
        let mut trailing = gzip(b"hello");
        trailing.push(0);
        for (data, enabled) in [
            (frame(&compressed, 1), true),
            (frame(&trailing, 1), true),
            (frame(&gzip(b"hello"), 1), false),
            (frame(&[0; 101], 0), false),
        ] {
            assert!(ConnectParser::new(100, enabled).feed(&data).is_err());
        }
    }
    #[test]
    fn prevents_plaintext_remote_auth() {
        assert!(GatewayClient::new("http://example.com/rpc", "key").is_err());
        assert!(GatewayClient::new("https://secret@example.com/rpc", "key").is_err());
        assert!(GatewayClient::new("https://example.com/rpc", "bad\nkey").is_err());
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
        let make = |path: &str| {
            GatewayClient::new(&format!("http://{address}/{path}"), "test-key").unwrap()
        };
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
}
