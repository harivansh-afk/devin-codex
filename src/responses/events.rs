use crate::{proto::Usage, tools::ToolKind};
use serde_json::{Value, json};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone)]
pub(super) struct Response {
    pub model: String,
    id: String,
    created_at: u64,
    sequence: u64,
    pub output: Vec<Value>,
    usage: Value,
    pub gateway_usage: Value,
}

impl Response {
    pub fn new(model: String) -> Self {
        Self {
            model,
            id: format!("resp_{}", uuid::Uuid::new_v4().simple()),
            created_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            sequence: 0,
            output: Vec::new(),
            usage: Value::Null,
            gateway_usage: json!({}),
        }
    }

    pub fn item_id(&self, prefix: &str, index: usize) -> String {
        format!("{prefix}_{}_{index}", self.id.trim_start_matches("resp_"))
    }

    fn event(&mut self, kind: &str, mut data: Value) -> Value {
        data["type"] = json!(kind);
        data["sequence_number"] = json!(self.sequence);
        self.sequence += 1;
        data
    }

    fn envelope(&self, status: &str) -> Value {
        json!({"id":self.id,"object":"response","created_at":self.created_at,
            "model":self.model,"status":status,"output":self.output,"usage":self.usage,
            "x_devin_usage":self.gateway_usage,"error":null,"incomplete_details":null})
    }

    pub fn record_usage(&mut self, usage: Usage) {
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
            self.usage = json!({"input_tokens":input,"output_tokens":output,"total_tokens":total});
        }
    }

    pub fn created_events(&mut self) -> Vec<Value> {
        let response = self.envelope("in_progress");
        vec![
            self.event("response.created", json!({"response":response})),
            self.event("response.in_progress", json!({"response":response})),
        ]
    }

    pub fn finished_events(&mut self, stop: Option<i32>) -> Vec<Value> {
        let incomplete = matches!(stop, Some(3 | 11));
        let status = if incomplete {
            "incomplete"
        } else {
            "completed"
        };
        let mut events = Vec::new();
        let mut output = std::mem::take(&mut self.output);
        for (index, item) in output.iter_mut().enumerate() {
            if item["type"] != "reasoning" {
                item["status"] = json!(status);
            }
            match item["type"].as_str() {
                Some("reasoning") => self.reasoning_events(index, item, &mut events),
                Some("message") => self.message_events(index, item, &mut events),
                Some(kind) if !incomplete => {
                    let kind = ToolKind::from_call(kind).expect("validated output item");
                    self.call_events(index, item, kind, &mut events);
                }
                _ => {}
            }
            // Codex executes tool output_item.done even without response.completed.
            if !incomplete || matches!(item["type"].as_str(), Some("reasoning" | "message")) {
                events.push(self.event(
                    "response.output_item.done",
                    json!({"output_index":index,"item":item}),
                ));
            }
        }
        self.output = output;
        let mut response = self.envelope(status);
        if incomplete {
            response["incomplete_details"] = json!({
                "reason":if stop == Some(3) {"max_output_tokens"} else {"content_filter"}
            });
        }
        events.push(self.event(
            if incomplete {
                "response.incomplete"
            } else {
                "response.completed"
            },
            json!({"response":response}),
        ));
        events
    }

    fn reasoning_events(&mut self, index: usize, item: &Value, events: &mut Vec<Value>) {
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
            events.push(self.event("response.reasoning_summary_part.added",
                json!({"output_index":index,"item_id":item["id"],"summary_index":summary_index,"part":part})));
            events.push(self.event("response.reasoning_summary_text.done",
                json!({"output_index":index,"item_id":item["id"],"summary_index":summary_index,"text":part["text"]})));
        }
    }

    fn message_events(&mut self, index: usize, item: &Value, events: &mut Vec<Value>) {
        let part = &item["content"][0];
        let initial = initial_item(item, "content", json!([]));
        events.push(self.event(
            "response.output_item.added",
            json!({"output_index":index,"item":initial}),
        ));
        events.push(self.event("response.content_part.added",
            json!({"output_index":index,"item_id":item["id"],"content_index":0,"part":{"type":"output_text","text":"","annotations":[]}})));
        events.push(self.event("response.output_text.delta",
            json!({"output_index":index,"item_id":item["id"],"content_index":0,"delta":part["text"]})));
        events.push(self.event("response.output_text.done",
            json!({"output_index":index,"item_id":item["id"],"content_index":0,"text":part["text"]})));
        events.push(self.event(
            "response.content_part.done",
            json!({"output_index":index,"item_id":item["id"],"content_index":0,"part":part}),
        ));
    }

    fn call_events(&mut self, index: usize, item: &Value, kind: ToolKind, events: &mut Vec<Value>) {
        let key = kind.input_key();
        let initial = initial_item(item, key, json!(""));
        events.push(self.event(
            "response.output_item.added",
            json!({"output_index":index,"item":initial}),
        ));
        events.push(self.event(&kind.input_event("delta"),
            json!({"output_index":index,"item_id":item["id"],"call_id":item["call_id"],"delta":item[key]})));
        let mut data = json!({"output_index":index,"item_id":item["id"]});
        data[key] = item[key].clone();
        events.push(self.event(&kind.input_event("done"), data));
    }

    pub fn failed_event(&mut self, code: &str, message: &str) -> Value {
        self.output.clear();
        let mut response = self.envelope("failed");
        response["error"] = json!({"type":"server_error","code":code,"message":message});
        self.event("response.failed", json!({"response":response}))
    }
}

fn initial_item(item: &Value, payload_key: &str, empty: Value) -> Value {
    let mut initial = item.clone();
    initial["status"] = json!("in_progress");
    initial[payload_key] = empty;
    initial
}
