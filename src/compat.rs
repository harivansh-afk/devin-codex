//! Explicitly lossy conversion for stock Codex. Strict API callers bypass this.
use crate::request::Problem;
use serde_json::{Value, json};
use std::collections::BTreeMap;

type Identity = (Option<String>, String);

#[derive(Default)]
pub struct Compatibility {
    names: BTreeMap<String, Identity>,
    descriptions: BTreeMap<String, String>,
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
    let parts = value
        .as_array()
        .ok_or_else(|| problem("Expected text content."))?;
    parts
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

impl Compatibility {
    fn tools(
        &mut self,
        tools: &[Value],
        namespace: Option<&str>,
        out: &mut Vec<Value>,
    ) -> Result<(), Vec<Problem>> {
        for tool in tools {
            let name = tool["name"]
                .as_str()
                .ok_or_else(|| problem("Tool name is missing."))?;
            if tool["type"] == "namespace" {
                if namespace.is_some() {
                    return Err(problem("Nested tool namespaces are unsupported."));
                }
                if let Some(description) = tool["description"].as_str().filter(|s| !s.is_empty()) {
                    self.descriptions.insert(name.into(), description.into());
                }
                self.tools(
                    tool["tools"]
                        .as_array()
                        .ok_or_else(|| problem("Expected namespace tools."))?,
                    Some(name),
                    out,
                )?;
                continue;
            }
            if !matches!(tool["type"].as_str(), Some("function" | "custom")) {
                return Err(problem(
                    "Only client-executed function and custom tools are supported.",
                ));
            }
            let alias = namespace.map_or_else(|| name.to_owned(), |ns| format!("{ns}__{name}"));
            let identity = (namespace.map(str::to_owned), name.to_owned());
            if self
                .names
                .get(&alias)
                .is_some_and(|existing| existing != &identity)
            {
                return Err(problem("Flattened tool names collide."));
            }
            self.names.insert(alias.clone(), identity);
            let mut tool = tool.clone();
            tool["name"] = json!(alias);
            if let Some(existing) = out.iter_mut().find(|t| t["name"] == alias) {
                *existing = tool;
            } else {
                out.push(tool);
            }
        }
        Ok(())
    }

    pub fn prepare(body: &Value) -> Result<(Value, Self), Vec<Problem>> {
        let mut body = body.clone();
        let obj = body
            .as_object_mut()
            .ok_or_else(|| problem("Expected request object."))?;
        // These are routing/cache/UI hints, or settings the gateway cannot express.
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
        // Serial gateway tool calls; concurrency inside Codex exec remains available.
        obj.insert("parallel_tool_calls".into(), json!(false));
        let mut this = Self::default();
        let mut definitions = Vec::new();
        if let Some(tools) = obj.get("tools") {
            this.tools(
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
                this.tools(
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
            let kind = item["type"].as_str().unwrap_or("message").to_owned();
            match kind.as_str() {
                "additional_tools" => continue,
                "message" => {
                    let content = text(&item["content"])?;
                    if matches!(item["role"].as_str(), Some("developer" | "system")) {
                        instructions.push_str(&format!(
                            "\n\n<{} message_index=\"{}\">\n{}\n</{}>",
                            item["role"].as_str().unwrap(),
                            index,
                            content,
                            item["role"].as_str().unwrap()
                        ));
                        continue;
                    }
                    item["content"] = json!(content);
                }
                "function_call" | "custom_tool_call" => {
                    let name = item["name"]
                        .as_str()
                        .ok_or_else(|| problem("History tool name is missing."))?;
                    let ns = item["namespace"].as_str();
                    let alias = ns.map_or_else(|| name.to_owned(), |ns| format!("{ns}__{name}"));
                    if !this.names.contains_key(&alias) {
                        return Err(problem(
                            "Historical tool is absent from the supplied definitions.",
                        ));
                    }
                    item["name"] = json!(alias);
                    item.as_object_mut().unwrap().remove("namespace");
                }
                "function_call_output" | "custom_tool_call_output" if item["output"].is_array() => {
                    item["output"] = json!(text(&item["output"])?);
                }
                _ => {}
            }
            input.push(item);
        }
        for (namespace, description) in &this.descriptions {
            instructions.push_str(&format!("\n\nNamespace {namespace}:\n{description}"));
        }
        if this.names.values().any(|(ns, _)| ns.is_some()) {
            instructions.push_str("\n\nTool transport names: use the flat name below when calling a tool. Names inside JavaScript passed to exec are unchanged.\n");
            for (alias, (ns, name)) in &this.names {
                if let Some(ns) = ns {
                    instructions.push_str(&format!("{ns}.{name} = {alias}\n"));
                }
            }
        }
        obj.insert("input".into(), json!(input));
        obj.insert("instructions".into(), json!(instructions));
        obj.insert("tools".into(), json!(definitions));
        Ok((body, this))
    }

    pub fn restore(&self, event: &mut Value) {
        fn item(names: &BTreeMap<String, Identity>, item: &mut Value) {
            if !matches!(
                item["type"].as_str(),
                Some("function_call" | "custom_tool_call")
            ) {
                return;
            }
            if let Some((namespace, name)) = item["name"].as_str().and_then(|name| names.get(name))
            {
                item["name"] = json!(name);
                if let Some(namespace) = namespace {
                    item["namespace"] = json!(namespace);
                }
            }
        }
        if let Some(value) = event.get_mut("item") {
            item(&self.names, value);
        }
        if let Some(output) = event
            .get_mut("response")
            .and_then(|r| r.get_mut("output"))
            .and_then(Value::as_array_mut)
        {
            for value in output {
                item(&self.names, value);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::Credentials, request::translate_request};

    fn request() -> Value {
        json!({"model":"gpt-6-astra", "reasoning":{"effort":"medium","context":"all_turns"},
        "text":{"verbosity":"low"}, "client_metadata":{}, "prompt_cache_key":"test", "service_tier":null,
        "input":[
            {"type":"additional_tools","role":"developer","tools":[
                {"type":"namespace","name":"functions","tools":[
                    {"type":"custom","name":"exec","description":"Original description", "format":{"type":"grammar","syntax":"lark","definition":"start: /.+/"}}
                ]},
                {"type":"namespace","name":"other","tools":[
                    {"type":"function","name":"exec","parameters":{"type":"object"},"strict":false}
                ]}
            ]},
            {"role":"developer","content":[{"type":"input_text","text":"First instruction"},{"type":"input_text","text":"Second instruction"}]},
            {"role":"user","content":"Do the task"},
            {"type":"reasoning","id":"rs_1","summary":[],"encrypted_content":"opaque"},
            {"type":"custom_tool_call","namespace":"functions","name":"exec","call_id":"c1","input":"text(42)"},
            {"type":"custom_tool_call_output","call_id":"c1","output":[{"type":"input_text","text":"42"}]}
        ]})
    }

    #[test]
    fn lite_history_roundtrip_preserves_payloads_and_tool_identity() {
        let (body, mapping) = Compatibility::prepare(&request()).unwrap();
        let credentials = Credentials {
            api_key: "test".into(),
            endpoint: "https://example.invalid".into(),
            device_id: String::new(),
            client_version: String::new(),
        };
        let translated = translate_request(&body, &credentials).unwrap_or_else(|e| panic!("{e:?}"));
        assert!(
            translated
                .request
                .system_prompt
                .contains("First instruction\n\nSecond instruction")
        );
        assert_eq!(translated.request.tools[0].name, "functions__exec");
        assert_eq!(
            translated.request.tools[0].description,
            "Original description"
        );
        assert_eq!(translated.request.tools[0].grammar, "start: /.+/");
        assert_eq!(translated.request.tools[1].name, "other__exec");
        assert_eq!(
            translated.request.messages[1].tool_calls[0].arguments_json,
            "text(42)"
        );
        assert!(translated.request.messages[1].reasoning.contains("opaque"));
        assert_eq!(translated.request.messages[2].content, "42");
        let call = json!({"type":"custom_tool_call","name":"functions__exec","call_id":"c2","input":"text(7)"});
        let mut event = json!({"type":"response.output_item.done","item":call});
        mapping.restore(&mut event);
        assert_eq!(event["item"]["namespace"], "functions");
        assert_eq!(event["item"]["name"], "exec");
        assert_eq!(event["item"]["input"], "text(7)");
        assert!(event.get("response").is_none());
        let mut final_event = json!({"type":"response.completed","response":{"output":[call]}});
        mapping.restore(&mut final_event);
        assert_eq!(final_event["response"]["output"][0], event["item"]);
    }

    #[test]
    fn unsupported_content_and_name_collisions_fail_before_inference() {
        let mut body = request();
        body["input"][2]["content"] = json!([{"type":"input_image","image_url":"data:private"}]);
        assert!(Compatibility::prepare(&body).is_err());
        let mut body = request();
        body["tools"] = json!([{"type":"function","name":"functions__exec","parameters":{}}]);
        assert!(Compatibility::prepare(&body).is_err());
        for body in [
            Value::Null,
            json!({"input":null}),
            json!({"input":[1]}),
            json!({"input":"hello","tools":[null]}),
        ] {
            assert!(Compatibility::prepare(&body).is_err());
        }
        let mut body = request();
        body["tools"] = json!([{"type":"web_search"}]);
        assert!(Compatibility::prepare(&body).is_err());
    }
}
