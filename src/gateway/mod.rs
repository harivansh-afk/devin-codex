//! Credential-isolated HTTP transport; Connect framing lives in `connect`.
use crate::proto::{ChatRequest, ChatResponse};
use futures_util::{Stream, StreamExt};
use prost::Message;
use std::{pin::Pin, time::Duration};

mod connect;
use connect::{ConnectParser, frame};

#[cfg(test)]
#[path = "../../tests/unit/gateway.rs"]
mod tests;

const MAX_FRAME: usize = 16 * 1024 * 1024;

mod error;
pub use error::GatewayError;

fn protocol() -> GatewayError {
    GatewayError::new("upstream_protocol_error")
}

#[derive(Clone)]
pub struct GatewayClient {
    client: reqwest::Client,
    endpoint: reqwest::Url,
    auth: reqwest::header::HeaderValue,
    pub(crate) diagnostics: Option<std::path::PathBuf>,
}

impl GatewayClient {
    pub fn new(endpoint: &str, api_key: &str) -> Result<Self, GatewayError> {
        Self::with_read_timeout(endpoint, api_key, Duration::from_secs(120))
    }

    fn with_read_timeout(
        endpoint: &str,
        api_key: &str,
        read_timeout: Duration,
    ) -> Result<Self, GatewayError> {
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
            .connect_timeout(Duration::from_secs(30))
            .read_timeout(read_timeout)
            .no_proxy()
            .build()
            .map_err(|_| GatewayError::new("tls_configuration_error"))?;
        Ok(Self {
            client,
            endpoint: url,
            auth,
            diagnostics: None,
        })
    }

    /// Enable private, content-free failure/recovery records for a running adapter.
    pub fn with_diagnostics(mut self) -> Self {
        self.diagnostics = crate::diagnostics::path();
        self
    }

    async fn open(&self, request: ChatRequest) -> Result<reqwest::Response, GatewayError> {
        if request.encoded_len() > MAX_FRAME {
            return Err(GatewayError::new("request_too_large"));
        }
        let response = self
            .client
            .post(self.endpoint.clone())
            .header("Authorization", self.auth.clone())
            .header("Content-Type", "application/connect+proto")
            .header("Connect-Protocol-Version", "1")
            .header("Connect-Accept-Encoding", "gzip")
            .header("Accept-Encoding", "identity")
            .body(frame(&request.encode_to_vec(), 0))
            .send()
            .await
            .map_err(http_error)?;
        if response.status() != reqwest::StatusCode::OK {
            return Err(GatewayError::http(
                response.status().as_u16(),
                response
                    .headers()
                    .get("retry-after")
                    .and_then(|value| value.to_str().ok()),
            ));
        }
        Ok(response)
    }

    pub fn stream(
        &self,
        request: ChatRequest,
    ) -> Pin<Box<dyn Stream<Item = Result<ChatResponse, GatewayError>> + Send + 'static>> {
        let this = self.clone();
        Box::pin(async_stream::try_stream! {
            let response = this.open(request).await?;
            let mut parser = ConnectParser::new(MAX_FRAME, gzip_enabled(&response)?);
            let mut upstream = response.bytes_stream();
            while let Some(chunk) = upstream.next().await {
                let chunk = chunk.map_err(http_error)?;
                // Bound buffering even if the HTTP library coalesces a large chunk.
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

fn gzip_enabled(response: &reqwest::Response) -> Result<bool, GatewayError> {
    let headers = response.headers();
    let content_type = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim();
    if content_type != "application/connect+proto"
        || headers
            .get("content-encoding")
            .is_some_and(|v| v != "identity")
    {
        return Err(protocol());
    }
    match headers.get("connect-content-encoding").map(|v| v.to_str()) {
        None | Some(Ok("identity")) => Ok(false),
        Some(Ok("gzip")) => Ok(true),
        _ => Err(protocol()),
    }
}

fn http_error(error: reqwest::Error) -> GatewayError {
    GatewayError::new(if error.is_timeout() {
        "upstream_timeout"
    } else {
        "upstream_connection_error"
    })
}
