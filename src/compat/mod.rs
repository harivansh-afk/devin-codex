//! Explicitly lossy Codex normalization before strict request validation.
use crate::request::Problem;
use serde_json::{Map, Value, json};

mod tools;
use tools::ToolNames;

#[cfg(test)]
#[path = "../../tests/unit/compat.rs"]
mod tests;

pub const MODEL: &str = "gpt-6-astra-devin";
pub const EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max"];

#[derive(Default)]
pub struct Compatibility {
    tools: ToolNames,
    model_alias: bool,
}

fn problem(reason: &str) -> Vec<Problem> {
    vec![Problem {
        path: "compatibility".into(),
        reason: reason.into(),
    }]
}

fn text(value: &Value) -> Result<String, Vec<Problem>> {
    if let Some(text) = value.as_str() {
        return Ok(text.into());
    }
    value
        .as_array()
        .ok_or_else(|| problem("Expected text content."))?
        .iter()
        .map(|p| {
            if !matches!(p["type"].as_str(), Some("input_text" | "output_text")) {
                return Err(problem(
                    "Images and non-text content are not supported yet.",
                ));
            }
            p["text"]
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| problem("Expected text part."))
        })
        .collect::<Result<Vec<_>, _>>()
        .map(|parts| parts.join("\n\n"))
}

fn close_reasoning_turn(input: &mut Vec<Value>) {
    if input.last().is_some_and(|item| item["type"] == "reasoning") {
        input.push(json!({"type":"message","role":"assistant","content":""}));
    }
}

impl Compatibility {
    pub fn prepare(body: &Value) -> Result<(Value, Self), Vec<Problem>> {
        let mut body = body.clone();
        let obj = body
            .as_object_mut()
            .ok_or_else(|| problem("Expected request object."))?;
        prepare_options(obj)?;
        let mut this = Self {
            model_alias: obj.get("model").and_then(Value::as_str) == Some(MODEL),
            ..Self::default()
        };
        if this.model_alias {
            obj.insert("model".into(), json!("gpt-6-astra"));
        }
        let mut definitions = Vec::new();
        if let Some(tools) = obj.get("tools") {
            this.tools.flatten(
                tools
                    .as_array()
                    .ok_or_else(|| problem("Expected tools array."))?,
                None,
                &mut definitions,
            )?;
        }
        let items = match obj.remove("input") {
            Some(Value::String(s)) => vec![json!({"role":"user", "content":s})],
            Some(Value::Array(items)) => items,
            _ => return Err(problem("Expected input text or array.")),
        };
        for item in &items {
            if item["type"] == "additional_tools" {
                this.tools.flatten(
                    item["tools"]
                        .as_array()
                        .ok_or_else(|| problem("Expected additional tools array."))?,
                    None,
                    &mut definitions,
                )?;
            }
        }
        if obj.get("instructions").is_some_and(|v| !v.is_string()) {
            return Err(problem("Expected instruction string."));
        }
        let mut instructions = obj
            .get("instructions")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let mut input = Vec::new();
        for (index, mut item) in items.into_iter().enumerate() {
            match item["type"].as_str().unwrap_or("message") {
                "additional_tools" => continue,
                "agent_message" => item = agent_message(&item)?,
                "message" => {
                    let content = text(&item["content"])?;
                    if matches!(item["role"].as_str(), Some("developer" | "system")) {
                        let role = item["role"].as_str().unwrap();
                        instructions.push_str(&format!(
                            "\n\n<{role} message_index=\"{index}\">\n{content}\n</{role}>"
                        ));
                        continue;
                    }
                    item["content"] = json!(content);
                }
                "function_call" | "custom_tool_call" => this.tools.prepare_call(&mut item)?,
                "function_call_output" | "custom_tool_call_output" if item["output"].is_array() => {
                    item["output"] = json!(text(&item["output"])?);
                }
                _ => {}
            }
            if item["role"] == "user" {
                close_reasoning_turn(&mut input);
            }
            input.push(item);
        }
        close_reasoning_turn(&mut input);
        this.tools.append_instructions(&mut instructions);
        obj.insert("input".into(), json!(input));
        obj.insert("instructions".into(), json!(instructions));
        obj.insert("tools".into(), json!(definitions));
        Ok((body, this))
    }

    pub fn restore(&self, event: &mut Value) {
        if self.model_alias
            && let Some(response) = event.get_mut("response").and_then(Value::as_object_mut)
        {
            response.insert("model".into(), json!(MODEL));
        }
        if let Some(item) = event.get_mut("item") {
            self.tools.restore_call(item);
        }
        if let Some(output) = event
            .get_mut("response")
            .and_then(|r| r.get_mut("output"))
            .and_then(Value::as_array_mut)
        {
            for item in output {
                self.tools.restore_call(item);
            }
        }
    }
}

fn prepare_options(obj: &mut Map<String, Value>) -> Result<(), Vec<Problem>> {
    // Routing/cache/UI hints and settings the gateway cannot express.
    for key in ["client_metadata", "prompt_cache_key", "service_tier"] {
        obj.remove(key);
    }
    if let Some(settings) = obj.remove("text")
        && settings
            .as_object()
            .is_none_or(|o| o.keys().any(|k| k != "verbosity"))
    {
        return Err(problem("Structured Responses output is not supported."));
    }
    if let Some(reasoning) = obj.get_mut("reasoning").and_then(Value::as_object_mut) {
        reasoning.remove("context");
        reasoning.remove("summary");
    }
    // Serial gateway calls; concurrency inside Codex exec remains available.
    obj.insert("parallel_tool_calls".into(), json!(false));
    Ok(())
}

fn agent_message(item: &Value) -> Result<Value, Vec<Problem>> {
    if item["content"]
        .as_array()
        .is_some_and(|parts| parts.iter().any(|p| p["type"] == "encrypted_content"))
    {
        return Err(problem(
            "Encrypted Codex agent messages are unsupported by the Devin gateway adapter.",
        ));
    }
    let content = text(&item["content"])?;
    let author = item["author"]
        .as_str()
        .ok_or_else(|| problem("Agent message author is missing."))?;
    let recipient = item["recipient"]
        .as_str()
        .ok_or_else(|| problem("Agent message recipient is missing."))?;
    Ok(json!({"type":"message","role":"user","content":format!(
        "Agent message from {author} to {recipient}:\n{content}"
    )}))
}
