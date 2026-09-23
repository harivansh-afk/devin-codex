use crate::{
    Mode,
    compat::Compatibility,
    config::Credentials,
    gateway::GatewayClient,
    models::Models,
    proto::ChatRequest,
    request::{Problem, translate_with_models},
    responses::Translator,
};
use futures_util::{StreamExt, stream::BoxStream};
use serde_json::Value;

pub(crate) struct Adapter {
    credentials: Credentials,
    gateway: GatewayClient,
    pub mode: Mode,
    models: Models,
}

pub(crate) struct PreparedRequest {
    request: ChatRequest,
    translator: Translator,
    mapping: Compatibility,
    pub streaming: bool,
}

impl Adapter {
    pub fn with_models(
        credentials: Credentials,
        gateway: GatewayClient,
        mode: Mode,
        models: Models,
    ) -> Self {
        Self {
            credentials,
            gateway,
            mode,
            models,
        }
    }

    pub fn prepare(&self, body: Value) -> Result<PreparedRequest, Vec<Problem>> {
        let streaming = body.get("stream").and_then(Value::as_bool).unwrap_or(false);
        let (body, mapping) = match self.mode {
            Mode::Strict => (body, Compatibility::default()),
            Mode::Codex => Compatibility::prepare_with_models(&body, &self.models)?,
        };
        let translated = translate_with_models(&body, &self.credentials, &self.models)?;
        Ok(PreparedRequest {
            request: translated.request,
            translator: Translator::new(
                translated.model,
                translated.tool_map,
                translated.allow_tools,
                self.mode,
            ),
            mapping,
            streaming,
        })
    }

    pub fn stream(&self, prepared: PreparedRequest) -> BoxStream<'static, Value> {
        let PreparedRequest {
            request,
            mut translator,
            mapping,
            ..
        } = prepared;
        let mut frames = self.gateway.stream(request);
        let events = async_stream::stream! {
            for event in translator.created_events() {
                yield event;
            }
            while let Some(frame) = frames.next().await {
                let frame = match frame {
                    Ok(frame) => frame,
                    Err(error) => {
                        let status = error.status.map(|s| format!(" (HTTP {s})")).unwrap_or_default();
                        let message = format!("Devin gateway failed: {}{status}", error.code);
                        yield translator.failed_event(&error.code, &message);
                        return;
                    }
                };
                if let Err(error) = translator.push(frame) {
                    yield translator.failed_event(
                        "response_translation_error",
                        &format!("Adapter rejected gateway frame: {error}"),
                    );
                    return;
                }
            }
            match translator.finish() {
                Ok(events) => {
                    for event in events {
                        yield event;
                    }
                }
                Err(error) => yield translator.failed_event(
                    "response_translation_error",
                    &format!("Adapter rejected completed gateway response: {error}"),
                ),
            }
        };
        events
            .map(move |mut event| {
                mapping.restore(&mut event);
                event
            })
            .boxed()
    }
}
