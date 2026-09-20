//! Buffer and validate a gateway response before exposing executable output.
use crate::{
    Mode,
    proto::{ChatResponse, ToolCall},
};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

mod events;
mod reasoning;

pub use crate::tools::ToolKind;
use events::Response;
use reasoning::ReasoningSignature;

#[cfg(test)]
#[path = "../../tests/unit/responses.rs"]
mod tests;

const MAX_BYTES: usize = 16 * 1024 * 1024;
const MAX_FRAMES: usize = 100_000;
const MAX_ITEMS: usize = 4096;

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct Error(pub &'static str);

pub struct Translator {
    mode: Mode,
    response: Response,
    tool_map: HashMap<String, ToolKind>,
    allow_tools: bool,
    message: Option<usize>,
    reasoning: Option<usize>,
    call: Option<(usize, ToolKind)>,
    signature: ReasoningSignature,
    stop: Option<i32>,
    received_bytes: usize,
    received_frames: usize,
    terminal: bool,
}

impl Translator {
    pub fn new(
        model: String,
        tool_map: HashMap<String, ToolKind>,
        allow_tools: bool,
        mode: Mode,
    ) -> Self {
        Self {
            mode,
            response: Response::new(model),
            tool_map,
            allow_tools,
            message: None,
            reasoning: None,
            call: None,
            signature: ReasoningSignature::default(),
            stop: None,
            received_bytes: 0,
            received_frames: 0,
            terminal: false,
        }
    }

    fn append(&mut self, item: Value) -> Result<usize, Error> {
        if self.response.output.len() >= MAX_ITEMS {
            return Err(Error("Gateway response exceeds output item limit"));
        }
        let index = self.response.output.len();
        self.response.output.push(item);
        Ok(index)
    }

    pub fn created_events(&mut self) -> Vec<Value> {
        self.response.created_events()
    }

    pub fn push(&mut self, frame: ChatResponse) -> Result<(), Error> {
        let result = self.push_inner(frame);
        if result.is_err() {
            self.terminal = true;
        }
        result
    }

    fn push_inner(&mut self, frame: ChatResponse) -> Result<(), Error> {
        if self.terminal {
            return Err(Error("Response stream is already terminal"));
        }
        self.received_frames += 1;
        self.received_bytes = self.received_bytes.saturating_add(frame_bytes(&frame));
        if self.received_bytes > MAX_BYTES || self.received_frames > MAX_FRAMES {
            return Err(Error("Gateway response exceeds accumulated size limit"));
        }
        if frame.redact {
            return Err(Error("Gateway redacted the response"));
        }
        if !frame.actual_model_uid.is_empty() {
            if frame.actual_model_uid != self.response.model
                && !frame
                    .actual_model_uid
                    .starts_with(&format!("{}-", self.response.model))
            {
                return Err(Error("Gateway reported an unexpected model family"));
            }
            self.response.gateway_usage["actual_model_uid"] = json!(frame.actual_model_uid);
        }
        if (!frame.delta_thinking.is_empty() || !frame.delta_signature.is_empty())
            && self.reasoning.is_none()
        {
            let id = if frame.thinking_id.is_empty() {
                self.response.item_id("rs", 0)
            } else {
                frame.thinking_id
            };
            self.reasoning = Some(self.append(json!({"type":"reasoning","id":id,"summary":[]}))?);
        }
        self.signature
            .push(&frame.delta_signature, &frame.delta_signature_type)?;
        if let Some(stop) = frame.stop_reason.filter(|stop| *stop != 0) {
            self.stop = Some(stop);
        }
        if let Some(usage) = frame.usage {
            self.response.record_usage(usage);
        }
        if let Some(cost) = frame.committed_acu_cost {
            self.response.gateway_usage["committed_acu_cost"] = json!(cost);
        }
        self.push_text(&frame.delta_text, &frame.phase, &frame.output_id)?;
        for delta in frame.delta_tool_calls {
            self.push_call(delta)?;
        }
        Ok(())
    }

    fn push_text(&mut self, delta: &str, phase: &str, output_id: &str) -> Result<(), Error> {
        if !matches!(phase, "" | "commentary" | "final_answer") {
            return Err(Error("Unsupported gateway message phase"));
        }
        if !delta.is_empty() {
            let new_message = self.message.is_none_or(|index| {
                (!phase.is_empty() && self.response.output[index]["phase"].as_str() != Some(phase))
                    || (!output_id.is_empty()
                        && self.response.output[index]["id"].as_str() != Some(output_id))
            });
            if new_message {
                let index = self.response.output.len();
                let id = if output_id.is_empty() {
                    self.response.item_id("msg", index)
                } else {
                    output_id.to_owned()
                };
                let mut item = json!({"type":"message","id":id,"role":"assistant","status":"in_progress",
                    "content":[{"type":"output_text","text":"","annotations":[]}]});
                if !phase.is_empty() {
                    item["phase"] = json!(phase);
                }
                self.message = Some(self.append(item)?);
            }
            let index = self.message.expect("message allocated above");
            let Value::String(text) = &mut self.response.output[index]["content"][0]["text"] else {
                return Err(Error("Invalid accumulated text state"));
            };
            text.push_str(delta);
        } else if !phase.is_empty()
            && let Some(index) = self.message
        {
            self.response.output[index]["phase"] = json!(phase);
        }
        Ok(())
    }

    fn push_call(&mut self, delta: ToolCall) -> Result<(), Error> {
        if !self.allow_tools {
            return Err(Error("Gateway called a tool with tool_choice none"));
        }
        if !delta.invalid_json_str.is_empty() || !delta.invalid_json_err.is_empty() {
            return Err(Error("Gateway returned an invalid tool call"));
        }
        if !delta.name.is_empty() && !self.tool_map.contains_key(&delta.name) {
            return Err(Error(
                "Gateway returned a tool absent from the supplied catalog",
            ));
        }
        if self.call.is_none() {
            if delta.id.is_empty() || delta.name.is_empty() {
                return Err(Error("Initial tool delta lacks identity"));
            }
            let kind = *self
                .tool_map
                .get(&delta.name)
                .ok_or(Error("Unknown tool"))?;
            if delta.is_custom_tool_call.unwrap_or(false) != (kind == ToolKind::Custom) {
                return Err(Error("Gateway changed tool call type"));
            }
            let index = self.response.output.len();
            let mut item = json!({"type":kind.call_type(),
                "id":self.response.item_id(kind.id_prefix(), index),"call_id":delta.id,
                "name":delta.name,"status":"in_progress"});
            item[kind.input_key()] = json!("");
            self.append(item)?;
            self.message = None;
            self.call = Some((index, kind));
        }
        let (index, kind) = self.call.expect("call allocated above");
        if !delta.id.is_empty() && self.response.output[index]["call_id"] != delta.id {
            return Err(Error(
                "Multiple tool calls are unsupported while parallel calls are disabled",
            ));
        }
        if !delta.name.is_empty()
            && self.response.output[index]["name"].as_str() != Some(&delta.name)
        {
            return Err(Error("Tool name changed within stream"));
        }
        if delta
            .is_custom_tool_call
            .is_some_and(|value| value != (kind == ToolKind::Custom))
        {
            return Err(Error("Tool type changed within stream"));
        }
        let Value::String(input) = &mut self.response.output[index][kind.input_key()] else {
            return Err(Error("Invalid accumulated tool state"));
        };
        input.push_str(&delta.arguments_json);
        Ok(())
    }

    /// Call only after the gateway has validated its terminal envelope and EOF.
    pub fn finish(&mut self) -> Result<Vec<Value>, Error> {
        let result = self.finish_inner();
        self.terminal = true;
        result
    }

    fn finish_inner(&mut self) -> Result<Vec<Value>, Error> {
        if self.terminal {
            return Err(Error("Response stream is already terminal"));
        }
        if !matches!(self.stop, None | Some(2 | 3 | 10 | 11)) {
            return Err(Error("Unsupported terminal stop reason"));
        }
        if let Some(index) = self.reasoning {
            let signature = self
                .signature
                .items(MAX_ITEMS - (self.response.output.len() - 1))?;
            if self.response.output.len() == 1 {
                return Err(Error("Reasoning has no assistant output for replay"));
            }
            if self.response.output.len() > 2 && self.mode == Mode::Strict {
                return Err(Error(
                    "Reasoning attachment to multiple assistant items is unverified",
                ));
            }
            // The signed state belongs before its assistant turn, even when it arrives after text.
            self.response.output.remove(index);
            self.response.output.splice(0..0, signature);
        }
        let mut ids = HashSet::new();
        if self
            .response
            .output
            .iter()
            .any(|item| !ids.insert(item["id"].as_str().unwrap_or_default()))
        {
            return Err(Error("Gateway reused an output item ID"));
        }
        if !matches!(self.stop, Some(3 | 11)) {
            for item in &self.response.output {
                if item["type"] == "function_call" {
                    let input: Value =
                        serde_json::from_str(item["arguments"].as_str().unwrap_or_default())
                            .map_err(|_| Error("Incomplete function-call JSON"))?;
                    if !input.is_object() {
                        return Err(Error("Function-call arguments are not a JSON object"));
                    }
                }
            }
        }
        Ok(self.response.finished_events(self.stop))
    }

    pub fn failed_event(&mut self, code: &str, message: &str) -> Value {
        self.terminal = true;
        let mut message = message.to_owned();
        if code == "response_translation_error"
            && let Some(shape) = self.signature.shape()
        {
            message.push_str(&format!(" ({shape})"));
        }
        self.response.failed_event(code, &message)
    }
}

fn frame_bytes(frame: &ChatResponse) -> usize {
    frame.delta_text.len()
        + frame.delta_thinking.len()
        + frame.delta_signature.len()
        + frame.delta_signature_type.len()
        + frame.phase.len()
        + frame.output_id.len()
        + frame.thinking_id.len()
        + frame.actual_model_uid.len()
        + frame
            .delta_tool_calls
            .iter()
            .map(|call| {
                call.id.len()
                    + call.name.len()
                    + call.arguments_json.len()
                    + call.invalid_json_str.len()
                    + call.invalid_json_err.len()
            })
            .sum::<usize>()
}
