//! Responses events without invented instructions, tools, or continuation state.

use crate::proto::ChatResponse;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_BYTES: usize = 16 * 1024 * 1024;
const MAX_FRAMES: usize = 100_000;
const MAX_ITEMS: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolKind {
    Function,
    Custom,
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct Error(pub &'static str);

pub struct Translator {
    pub compatibility: bool,
    model: String,
    response_id: String,
    created_at: u64,
    sequence: u64,
    tool_map: HashMap<String, ToolKind>,
    allow_tools: bool,
    output: Vec<Value>,
    message: Option<usize>,
    reasoning: Option<usize>,
    call: Option<(usize, ToolKind)>,
    signature: String,
    signature_type: String,
    stop: Option<i32>,
    usage: Value,
    gateway_usage: Value,
    received_bytes: usize,
    received_frames: usize,
    terminal: bool,
}

impl Translator {
    pub fn new(model: String, tool_map: HashMap<String, ToolKind>, allow_tools: bool) -> Self {
        Self {
            compatibility: false,
            model,
            response_id: format!("resp_{}", uuid::Uuid::new_v4().simple()),
            created_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            sequence: 0,
            tool_map,
            allow_tools,
            output: Vec::new(),
            message: None,
            reasoning: None,
            call: None,
            signature: String::new(),
            signature_type: String::new(),
            stop: None,
            usage: Value::Null,
            gateway_usage: json!({}),
            received_bytes: 0,
            received_frames: 0,
            terminal: false,
        }
    }

    fn event(&mut self, kind: &str, mut data: Value) -> Value {
        data["type"] = json!(kind);
        data["sequence_number"] = json!(self.sequence);
        self.sequence += 1;
        data
    }

    fn response(&self, status: &str) -> Value {
        json!({"id":self.response_id,"object":"response","created_at":self.created_at,
            "model":self.model,"status":status,"output":self.output,"usage":self.usage,
            "x_devin_usage":self.gateway_usage,"error":null,"incomplete_details":null})
    }

    fn id(&self, prefix: &str, index: usize) -> String {
        format!(
            "{prefix}_{}_{index}",
            self.response_id.trim_start_matches("resp_")
        )
    }

    fn append(&mut self, item: Value) -> Result<usize, Error> {
        if self.output.len() >= MAX_ITEMS {
            return Err(Error("Gateway response exceeds output item limit"));
        }
        let index = self.output.len();
        self.output.push(item);
        Ok(index)
    }

    pub fn created_events(&mut self) -> Vec<Value> {
        let response = self.response("in_progress");
        vec![
            self.event("response.created", json!({"response":response})),
            self.event("response.in_progress", json!({"response":response})),
        ]
    }

    /// Buffer model output until checked EOF reveals the response-scoped signature.
    /// This initial subset intentionally does not preserve token streaming latency.
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
        let bytes = frame.delta_text.len()
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
                .sum::<usize>();
        self.received_bytes = self.received_bytes.saturating_add(bytes);
        if self.received_bytes > MAX_BYTES || self.received_frames > MAX_FRAMES {
            return Err(Error("Gateway response exceeds accumulated size limit"));
        }
        if frame.redact {
            return Err(Error("Gateway redacted the response"));
        }
        if !frame.actual_model_uid.is_empty() {
            if frame.actual_model_uid != self.model
                && !frame
                    .actual_model_uid
                    .starts_with(&format!("{}-", self.model))
            {
                return Err(Error("Gateway reported an unexpected model family"));
            }
            self.gateway_usage["actual_model_uid"] = json!(frame.actual_model_uid);
        }
        if (!frame.delta_thinking.is_empty() || !frame.delta_signature.is_empty())
            && self.reasoning.is_none()
        {
            let id = if frame.thinking_id.is_empty() {
                self.id("rs", 0)
            } else {
                frame.thinking_id
            };
            self.reasoning = Some(self.append(json!({"type":"reasoning","id":id,"summary":[]}))?);
        }
        self.signature.push_str(&frame.delta_signature);
        if !frame.delta_signature_type.is_empty() {
            if frame.delta_signature_type != "openai" {
                return Err(Error("Unsupported gateway reasoning signature type"));
            }
            self.signature_type = frame.delta_signature_type;
        }
        if let Some(stop) = frame.stop_reason.filter(|stop| *stop != 0) {
            self.stop = Some(stop);
        }
        if let Some(usage) = frame.usage {
            self.gateway_usage["input_tokens"] = json!(usage.input_tokens);
            self.gateway_usage["output_tokens"] = json!(usage.output_tokens);
            self.gateway_usage["cache_read_tokens"] = json!(usage.cache_read_tokens);
            self.gateway_usage["cache_write_tokens"] = json!(usage.cache_write_tokens);
            self.usage = Value::Null;
            // Cache/input overlap is unverified. Preserve counters, not guesses.
            if usage.cache_read_tokens.unwrap_or(0) == 0
                && usage.cache_write_tokens.unwrap_or(0) == 0
                && let (Some(input), Some(output)) = (usage.input_tokens, usage.output_tokens)
                && let Some(total) = input.checked_add(output)
            {
                self.usage =
                    json!({"input_tokens":input,"output_tokens":output,"total_tokens":total});
            }
        }
        if let Some(cost) = frame.committed_acu_cost {
            self.gateway_usage["committed_acu_cost"] = json!(cost);
        }
        if !matches!(frame.phase.as_str(), "" | "commentary" | "final_answer") {
            return Err(Error("Unsupported gateway message phase"));
        }
        if !frame.delta_text.is_empty() {
            let new_message = match self.message {
                None => true,
                Some(index) => {
                    (!frame.phase.is_empty()
                        && self.output[index]["phase"].as_str() != Some(&frame.phase))
                        || (!frame.output_id.is_empty()
                            && self.output[index]["id"].as_str() != Some(&frame.output_id))
                }
            };
            if new_message {
                let index = self.output.len();
                let id = if frame.output_id.is_empty() {
                    self.id("msg", index)
                } else {
                    frame.output_id
                };
                let mut item = json!({"type":"message","id":id,"role":"assistant","status":"in_progress",
                    "content":[{"type":"output_text","text":"","annotations":[]}]});
                if !frame.phase.is_empty() {
                    item["phase"] = json!(frame.phase);
                }
                self.message = Some(self.append(item)?);
            }
            let index = self.message.expect("message allocated above");
            let Value::String(text) = &mut self.output[index]["content"][0]["text"] else {
                return Err(Error("Invalid accumulated text state"));
            };
            text.push_str(&frame.delta_text);
        } else if !frame.phase.is_empty()
            && let Some(index) = self.message
        {
            self.output[index]["phase"] = json!(frame.phase);
        }
        for delta in frame.delta_tool_calls {
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
                let custom = kind == ToolKind::Custom;
                if delta.is_custom_tool_call.unwrap_or(false) != custom {
                    return Err(Error("Gateway changed tool call type"));
                }
                let index = self.output.len();
                let mut item = json!({"type":if custom {"custom_tool_call"} else {"function_call"},
                    "id":self.id(if custom {"ctc"} else {"fc"}, index),"call_id":delta.id,
                    "name":delta.name,"status":"in_progress"});
                item[if custom { "input" } else { "arguments" }] = json!("");
                self.append(item)?;
                self.message = None;
                self.call = Some((index, kind));
            }
            let (index, kind) = self.call.expect("call allocated above");
            if !delta.id.is_empty() && self.output[index]["call_id"] != delta.id {
                return Err(Error(
                    "Multiple tool calls are unsupported while parallel calls are disabled",
                ));
            }
            if !delta.name.is_empty() && self.output[index]["name"].as_str() != Some(&delta.name) {
                return Err(Error("Tool name changed within stream"));
            }
            let custom = kind == ToolKind::Custom;
            if delta
                .is_custom_tool_call
                .is_some_and(|value| value != custom)
            {
                return Err(Error("Tool type changed within stream"));
            }
            let key = if custom { "input" } else { "arguments" };
            let Value::String(input) = &mut self.output[index][key] else {
                return Err(Error("Invalid accumulated tool state"));
            };
            input.push_str(&delta.arguments_json);
        }
        Ok(())
    }

    /// Call ONLY after Gateway has validated its terminal envelope and EOF.
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
            if self.signature_type != "openai" || self.signature.is_empty() {
                return Err(Error("Reasoning lacks replayable OpenAI signature"));
            }
            let mut signature: Vec<Value> = serde_json::from_str(&self.signature)
                .map_err(|_| Error("Malformed reasoning signature"))?;
            if signature.len() != 1 {
                return Err(Error("Unsupported reasoning signature shape"));
            }
            let mut item = signature.remove(0);
            validate_reasoning(&item)?;
            if item.get("summary").is_none() {
                item["summary"] = json!([]);
            }
            if self.output.len() == 1 {
                return Err(Error("Reasoning has no assistant output for replay"));
            }
            if self.output.len() > 2 && !self.compatibility {
                return Err(Error(
                    "Reasoning attachment to multiple assistant items is unverified",
                ));
            }
            // The gateway returns response-scoped signed state, sometimes only
            // after text. Responses history places it before its assistant turn.
            self.output.remove(index);
            self.output.insert(0, item);
        }
        let mut ids = std::collections::HashSet::new();
        if self
            .output
            .iter()
            .any(|item| !ids.insert(item["id"].as_str().unwrap_or_default()))
        {
            return Err(Error("Gateway reused an output item ID"));
        }
        let incomplete = matches!(self.stop, Some(3 | 11));
        let status = if incomplete {
            "incomplete"
        } else {
            "completed"
        };
        if !incomplete {
            for item in &self.output {
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
        let mut events = Vec::new();
        let mut output = std::mem::take(&mut self.output);
        for (index, item) in output.iter_mut().enumerate() {
            let is_reasoning = item["type"] == "reasoning";
            let is_message = item["type"] == "message";
            if !is_reasoning {
                item["status"] = json!(status);
            }
            if is_reasoning {
                events.push(self.event(
                    "response.output_item.added",
                    json!({"output_index":index,"item":item}),
                ));
                for (summary_index, part) in item["summary"]
                    .as_array()
                    .expect("validated summary")
                    .iter()
                    .enumerate()
                {
                    events.push(self.event("response.reasoning_summary_part.added", json!({"output_index":index,"item_id":item["id"],"summary_index":summary_index,"part":part})));
                    events.push(self.event("response.reasoning_summary_text.done", json!({"output_index":index,"item_id":item["id"],"summary_index":summary_index,"text":part["text"]})));
                }
            } else if is_message {
                let part = &item["content"][0];
                let initial = initial_item(item, "content", json!([]));
                events.push(self.event(
                    "response.output_item.added",
                    json!({"output_index":index,"item":initial}),
                ));
                events.push(self.event("response.content_part.added", json!({"output_index":index,"item_id":item["id"],"content_index":0,"part":{"type":"output_text","text":"","annotations":[]}})));
                events.push(self.event("response.output_text.delta", json!({"output_index":index,"item_id":item["id"],"content_index":0,"delta":part["text"]})));
                events.push(self.event("response.output_text.done", json!({"output_index":index,"item_id":item["id"],"content_index":0,"text":part["text"]})));
                events.push(self.event("response.content_part.done", json!({"output_index":index,"item_id":item["id"],"content_index":0,"part":part})));
            } else if !incomplete {
                let custom = item["type"] == "custom_tool_call";
                let key = if custom { "input" } else { "arguments" };
                let initial = initial_item(item, key, json!(""));
                events.push(self.event(
                    "response.output_item.added",
                    json!({"output_index":index,"item":initial}),
                ));
                events.push(self.event(if custom {"response.custom_tool_call_input.delta"} else {"response.function_call_arguments.delta"}, json!({"output_index":index,"item_id":item["id"],"call_id":item["call_id"],"delta":item[key]})));
                let mut data = json!({"output_index":index,"item_id":item["id"]});
                data[key] = item[key].clone();
                events.push(self.event(
                    if custom {
                        "response.custom_tool_call_input.done"
                    } else {
                        "response.function_call_arguments.done"
                    },
                    data,
                ));
            }
            // Codex executes tool output_item.done even without response.completed.
            if !incomplete || is_reasoning || is_message {
                events.push(self.event(
                    "response.output_item.done",
                    json!({"output_index":index,"item":item}),
                ));
            }
        }
        self.output = output;
        let mut response = self.response(status);
        if incomplete {
            response["incomplete_details"] = json!({"reason":if self.stop == Some(3) {"max_output_tokens"} else {"content_filter"}});
        }
        events.push(self.event(
            if incomplete {
                "response.incomplete"
            } else {
                "response.completed"
            },
            json!({"response":response}),
        ));
        Ok(events)
    }

    pub fn failed_event(&mut self) -> Value {
        self.terminal = true;
        let mut response = self.response("failed");
        response["error"] = json!({"type":"server_error","code":"upstream_protocol_error","message":"Upstream inference stream failed"});
        self.event("response.failed", json!({"response":response}))
    }
}

fn initial_item(item: &Value, payload_key: &str, empty: Value) -> Value {
    let mut initial: Value = item
        .as_object()
        .expect("output item is an object")
        .iter()
        .filter(|(key, _)| key.as_str() != payload_key)
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    initial["status"] = json!("in_progress");
    initial[payload_key] = empty;
    initial
}

fn validate_reasoning(item: &Value) -> Result<(), Error> {
    let object = item.as_object().ok_or(Error("Invalid reasoning item"))?;
    if object.keys().any(|key| {
        !matches!(
            key.as_str(),
            "type" | "id" | "summary" | "content" | "encrypted_content" | "status"
        )
    }) {
        return Err(Error("Unsupported reasoning properties"));
    }
    if item["type"] != "reasoning"
        || item["id"].as_str().is_none_or(str::is_empty)
        || item["encrypted_content"].as_str().is_none_or(str::is_empty)
    {
        return Err(Error("Invalid replayable reasoning identity"));
    }
    if let Some(status) = item.get("status")
        && !matches!(status.as_str(), Some("" | "completed"))
    {
        return Err(Error("Unsupported reasoning status"));
    }
    if let Some(content) = item.get("content")
        && content.as_array().is_none_or(|parts| !parts.is_empty())
    {
        return Err(Error("Unexpected plaintext reasoning content"));
    }
    if let Some(summary) = item.get("summary") {
        let parts = summary
            .as_array()
            .ok_or(Error("Invalid reasoning summary"))?;
        for part in parts {
            if part.as_object().is_none_or(|object| object.len() != 2)
                || part["type"] != "summary_text"
                || !part["text"].is_string()
            {
                return Err(Error("Unsupported reasoning summary properties"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::{ToolCall, Usage};

    fn translator() -> Translator {
        Translator::new(
            "gpt-6-astra".into(),
            HashMap::from([
                ("exec".into(), ToolKind::Custom),
                ("run".into(), ToolKind::Function),
            ]),
            true,
        )
    }

    fn call(custom: bool) -> ChatResponse {
        ChatResponse {
            delta_tool_calls: vec![ToolCall {
                id: "a".into(),
                name: if custom { "exec" } else { "run" }.into(),
                is_custom_tool_call: Some(custom),
                arguments_json: if custom { "text(" } else { "{" }.into(),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn reasoning() -> Value {
        json!({"type":"reasoning","id":"rs_native","summary":[],"content":[],"status":"","encrypted_content":"opaque"})
    }

    #[test]
    fn text_phase_sequences_and_success_without_stop_enum() {
        let mut t = translator();
        let mut events = t.created_events();
        t.push(ChatResponse {
            delta_text: "hello".into(),
            phase: "final_answer".into(),
            ..Default::default()
        })
        .unwrap();
        events.extend(t.finish().unwrap());
        for (index, event) in events.iter().enumerate() {
            assert_eq!(event["sequence_number"], index);
        }
        assert_eq!(
            events.last().unwrap()["response"]["output"][0]["phase"],
            "final_answer"
        );
    }

    #[test]
    fn custom_identity_only_on_first_delta() {
        let mut t = translator();
        t.push(call(true)).unwrap();
        t.push(ChatResponse {
            delta_tool_calls: vec![ToolCall {
                arguments_json: "42)".into(),
                ..Default::default()
            }],
            ..Default::default()
        })
        .unwrap();
        let events = t.finish().unwrap();
        let item = &events.last().unwrap()["response"]["output"][0];
        assert_eq!(item["input"], "text(42)");
        assert!(item.get("arguments").is_none());
    }

    #[test]
    fn errors_and_limits_never_finalize_partial_calls() {
        let mut t = translator();
        t.push(call(false)).unwrap();
        let failed = t.failed_event();
        assert_eq!(failed["type"], "response.failed");
        assert!(t.finish().is_err());
        let mut t = translator();
        t.push(call(false)).unwrap();
        t.push(ChatResponse {
            stop_reason: Some(3),
            ..Default::default()
        })
        .unwrap();
        let events = t.finish().unwrap();
        assert!(
            !events
                .iter()
                .any(|e| e["type"] == "response.output_item.done")
        );
        assert_eq!(events.last().unwrap()["type"], "response.incomplete");
    }

    #[test]
    fn reject_unknown_disallowed_or_multiple_calls() {
        let mut t = Translator::new("model".into(), HashMap::new(), true);
        assert!(t.push(call(true)).is_err());
        let mut t = translator();
        t.allow_tools = false;
        assert!(t.push(call(true)).is_err());
        let mut t = translator();
        t.push(call(true)).unwrap();
        let mut second = call(true);
        second.delta_tool_calls[0].id = "b".into();
        assert!(t.push(second).is_err());
    }

    #[test]
    fn encrypted_reasoning_keeps_native_identity_and_hides_raw_thought() {
        let mut t = translator();
        let signature = json!([reasoning()]).to_string();
        t.push(ChatResponse {
            delta_thinking: "PRIVATE".into(),
            delta_signature: signature[..10].into(),
            ..Default::default()
        })
        .unwrap();
        t.push(ChatResponse {
            delta_signature: signature[10..].into(),
            delta_signature_type: "openai".into(),
            delta_text: "answer".into(),
            ..Default::default()
        })
        .unwrap();
        let events = t.finish().unwrap();
        assert_eq!(events.last().unwrap()["response"]["output"][0], reasoning());
        assert!(!serde_json::to_string(&events).unwrap().contains("PRIVATE"));
    }

    #[test]
    fn reject_unreplayable_reasoning() {
        for patch in [
            json!({"encrypted_content":""}),
            json!({"unknown":true}),
            json!({"summary":[{"type":"summary_text","text":"x","annotations":[]}]}),
        ] {
            let mut item = reasoning();
            for (key, value) in patch.as_object().unwrap() {
                item[key] = value.clone();
            }
            assert!(validate_reasoning(&item).is_err());
        }
        let mut t = translator();
        t.push(ChatResponse {
            delta_signature: json!([reasoning()]).to_string(),
            delta_text: "answer".into(),
            ..Default::default()
        })
        .unwrap();
        assert!(t.finish().is_err());
        let mut t = translator();
        t.push(ChatResponse {
            delta_text: "answer".into(),
            ..Default::default()
        })
        .unwrap();
        t.push(ChatResponse {
            delta_signature: json!([reasoning()]).to_string(),
            delta_signature_type: "openai".into(),
            ..Default::default()
        })
        .unwrap();
        let events = t.finish().unwrap();
        assert_eq!(events.last().unwrap()["response"]["output"][0], reasoning());
        let replay =
            json!({"model":"gpt-6-astra","input":events.last().unwrap()["response"]["output"]});
        assert!(crate::request::audit_request(&replay).is_empty());
    }

    #[test]
    fn raw_cache_counters_do_not_invent_total_usage() {
        let mut t = translator();
        t.push(ChatResponse {
            usage: Some(Usage {
                input_tokens: Some(100),
                output_tokens: Some(5),
                cache_read_tokens: Some(80),
                ..Default::default()
            }),
            ..Default::default()
        })
        .unwrap();
        let events = t.finish().unwrap();
        let response = &events.last().unwrap()["response"];
        assert!(response["usage"].is_null());
        assert_eq!(response["x_devin_usage"]["cache_read_tokens"], 80);
    }

    #[test]
    fn accumulated_private_thinking_is_bounded() {
        let mut t = translator();
        t.received_bytes = MAX_BYTES;
        assert!(
            t.push(ChatResponse {
                delta_thinking: "x".into(),
                ..Default::default()
            })
            .is_err()
        );
    }

    #[test]
    fn text_after_tool_stays_after_tool() {
        let mut t = translator();
        t.push(ChatResponse {
            delta_text: "before".into(),
            ..Default::default()
        })
        .unwrap();
        t.push(call(true)).unwrap();
        t.push(ChatResponse {
            delta_text: "after".into(),
            ..Default::default()
        })
        .unwrap();
        let events = t.finish().unwrap();
        let output = events.last().unwrap()["response"]["output"]
            .as_array()
            .unwrap();
        assert_eq!(output.len(), 3);
        assert_eq!(output[0]["content"][0]["text"], "before");
        assert_eq!(output[1]["type"], "custom_tool_call");
        assert_eq!(output[2]["content"][0]["text"], "after");
    }

    #[test]
    fn multi_item_reasoning_attachment_is_rejected() {
        let mut t = translator();
        t.push(ChatResponse {
            delta_text: "commentary".into(),
            ..Default::default()
        })
        .unwrap();
        t.push(call(true)).unwrap();
        t.push(ChatResponse {
            delta_signature: json!([reasoning()]).to_string(),
            delta_signature_type: "openai".into(),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(
            t.finish().unwrap_err().0,
            "Reasoning attachment to multiple assistant items is unverified"
        );
    }

    #[test]
    fn compatibility_attaches_signed_reasoning_before_assistant_turn() {
        let mut t = translator();
        t.compatibility = true;
        t.push(ChatResponse {
            delta_text: "commentary".into(),
            phase: "commentary".into(),
            ..Default::default()
        })
        .unwrap();
        t.push(call(true)).unwrap();
        t.push(ChatResponse {
            delta_signature: json!([reasoning()]).to_string(),
            delta_signature_type: "openai".into(),
            ..Default::default()
        })
        .unwrap();
        let events = t.finish().unwrap();
        let output = &events.last().unwrap()["response"]["output"];
        assert_eq!(output[0], reasoning());
        assert_eq!(output[1]["phase"], "commentary");
        assert_eq!(output[2]["name"], "exec");
    }

    #[test]
    fn reported_model_family_must_match() {
        for uid in ["different-model", "gpt-6-astral-medium"] {
            let mut t = translator();
            assert!(
                t.push(ChatResponse {
                    actual_model_uid: uid.into(),
                    ..Default::default()
                })
                .is_err()
            );
            assert!(t.finish().is_err());
        }
        let mut t = translator();
        t.push(ChatResponse {
            actual_model_uid: "gpt-6-astra-medium".into(),
            ..Default::default()
        })
        .unwrap();
        let events = t.finish().unwrap();
        assert_eq!(
            events.last().unwrap()["response"]["x_devin_usage"]["actual_model_uid"],
            "gpt-6-astra-medium"
        );
    }
}
