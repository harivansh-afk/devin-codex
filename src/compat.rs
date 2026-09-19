//! Explicitly lossy conversion for stock Codex. Strict API callers bypass this.
use crate::request::Problem;
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub const MODEL: &str = "gpt-6-astra-devin";
pub const EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max"];

type Identity = (Option<String>, String);

fn plaintext_agent_tool(namespace: Option<&str>, name: &str) -> bool {
    namespace == Some("collaboration")
        && matches!(name, "spawn_agent" | "send_message" | "followup_task")
}

fn close_reasoning_turn(input: &mut Vec<Value>) {
    if input.last().is_some_and(|item| item["type"] == "reasoning") {
        input.push(json!({"type":"message","role":"assistant","content":""}));
    }
}

#[derive(Default)]
pub struct Compatibility {
    names: BTreeMap<String, Identity>,
    descriptions: BTreeMap<String, String>,
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
            if tool["type"] == "function" && plaintext_agent_tool(namespace, name) {
                let message = tool
                    .pointer_mut("/parameters/properties/message")
                    .and_then(Value::as_object_mut)
                    .ok_or_else(|| problem("Expected collaboration message schema."))?;
                if message.get("type").and_then(Value::as_str) != Some("string") {
                    return Err(problem("Expected a string collaboration message."));
                }
                message.remove("encrypted");
            }
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
        let mut this = Self {
            model_alias: obj.get("model").and_then(Value::as_str) == Some(MODEL),
            ..Self::default()
        };
        if this.model_alias {
            obj.insert("model".into(), json!("gpt-6-astra"));
        }
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
                "agent_message" => {
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
                    item = json!({"type":"message","role":"user","content":format!(
                        "Agent message from {author} to {recipient}:\n{content}"
                    )});
                }
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
                    let identity = (ns.map(str::to_owned), name.to_owned());
                    if this
                        .names
                        .get(&alias)
                        .is_some_and(|existing| existing != &identity)
                    {
                        return Err(problem("Flattened historical tool names collide."));
                    }
                    this.names.insert(alias.clone(), identity);
                    if kind == "function_call"
                        && plaintext_agent_tool(ns, name)
                        && let Some(encrypted) = item.get("encrypted_function_args")
                    {
                        if encrypted != &json!([]) {
                            return Err(problem(
                                "Encrypted collaboration arguments cannot be replayed.",
                            ));
                        }
                        item.as_object_mut()
                            .unwrap()
                            .remove("encrypted_function_args");
                    }
                    item["name"] = json!(alias);
                    item.as_object_mut().unwrap().remove("namespace");
                }
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
        if self.model_alias
            && let Some(response) = event.get_mut("response").and_then(Value::as_object_mut)
        {
            response.insert("model".into(), json!(MODEL));
        }
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
                if item["type"] == "function_call"
                    && plaintext_agent_tool(namespace.as_deref(), name)
                {
                    item["encrypted_function_args"] = json!([]);
                }
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
    fn devin_model_alias_preserves_effort_and_response_identity() {
        let credentials = Credentials {
            api_key: "test".into(),
            endpoint: "https://example.invalid".into(),
            device_id: String::new(),
            client_version: String::new(),
        };
        for effort in ["low", "medium", "high", "xhigh", "max"] {
            let body = json!({"model":"gpt-6-astra-devin", "input":"hello", "reasoning":{"effort":effort}});
            let (prepared, mapping) = Compatibility::prepare(&body).unwrap();
            let translated = translate_request(&prepared, &credentials).unwrap();
            assert_eq!(translated.request.model, format!("gpt-6-astra-{effort}"));
            assert_eq!(translated.model, "gpt-6-astra");
            for kind in ["response.created", "response.completed", "response.failed"] {
                let mut event = json!({"type":kind,"response":{"model":"gpt-6-astra","output":[]}});
                mapping.restore(&mut event);
                assert_eq!(event["response"]["model"], "gpt-6-astra-devin");
            }
        }
        for model in ["other", "gpt-6-astra-devin-other"] {
            let (prepared, _) =
                Compatibility::prepare(&json!({"model":model,"input":"hello"})).unwrap();
            assert!(translate_request(&prepared, &credentials).is_err());
        }
    }

    #[test]
    fn collaboration_uses_codex_plaintext_dispatch_without_changing_other_tools() {
        for name in ["spawn_agent", "send_message", "followup_task"] {
            for namespace in ["collaboration", "other"] {
                let parameters = json!({"type":"object","properties":{"message":{"type":"string","description":"Task text","encrypted":true}},"required":["message"]});
                let body = json!({"model":MODEL,"input":"hello","tools":[{
                    "type":"namespace","name":namespace,"tools":[{
                        "type":"function","name":name,"parameters":parameters
                    }]
                }]});
                let (prepared, mapping) = Compatibility::prepare(&body).unwrap();
                let actual = &prepared["tools"][0]["parameters"];
                if namespace == "collaboration" {
                    assert!(actual["properties"]["message"].get("encrypted").is_none());
                    assert_eq!(actual["properties"]["message"]["description"], "Task text");
                } else {
                    assert_eq!(actual, &parameters);
                }
                let call = json!({"type":"function_call","name":format!("{namespace}__{name}"),"call_id":"c1","arguments":"{\"message\":\"hello\"}"});
                let mut event = json!({"type":"response.output_item.done","item":call,"response":{"output":[call]}});
                mapping.restore(&mut event);
                let item = &event["item"];
                assert_eq!(item["namespace"], namespace);
                assert_eq!(item["name"], name);
                assert_eq!(item["arguments"], "{\"message\":\"hello\"}");
                assert_eq!(event["response"]["output"][0], *item);
                if namespace == "collaboration" {
                    assert_eq!(item["encrypted_function_args"], json!([]));
                    let mut followup = body.clone();
                    followup["input"] =
                        json!([item, {"type":"function_call_output","call_id":"c1","output":"ok"}]);
                    let (followup, _) = Compatibility::prepare(&followup).unwrap();
                    assert!(crate::request::audit_request(&followup).is_empty());
                } else {
                    assert!(item.get("encrypted_function_args").is_none());
                }
            }
        }
    }

    #[test]
    fn interrupted_reasoning_keeps_an_empty_assistant_boundary() {
        let reasoning = json!({"type":"reasoning","encrypted_content":"opaque","summary":[]});
        for tail in [
            vec![],
            vec![json!({"role":"user","content":"interrupt"})],
            vec![
                json!({"type":"agent_message","author":"/root/child","recipient":"/root","content":[{"type":"input_text","text":"done"}]}),
            ],
        ] {
            let mut input = vec![reasoning.clone()];
            input.extend(tail);
            let (body, _) = Compatibility::prepare(&json!({"model":MODEL,"input":input})).unwrap();
            assert_eq!(body["input"][0], reasoning);
            assert_eq!(
                body["input"][1],
                json!({"type":"message","role":"assistant","content":""})
            );
            assert!(crate::request::audit_request(&body).is_empty());
        }
    }

    #[test]
    fn compaction_keeps_historical_calls_without_readvertising_tools() {
        let body = json!({"model":MODEL,"tools":[],"tool_choice":"none","input":[
            {"type":"custom_tool_call","namespace":"functions","name":"exec","call_id":"c1","input":"text(42)"},
            {"type":"custom_tool_call_output","call_id":"c1","output":"42"},
            {"type":"function_call","namespace":"collaboration","name":"wait_agent","call_id":"c2","arguments":"{}"},
            {"type":"function_call_output","call_id":"c2","output":"done"}
        ]});
        let (prepared, _) = Compatibility::prepare(&body).unwrap();
        assert_eq!(prepared["tools"], json!([]));
        assert_eq!(prepared["input"][0]["name"], "functions__exec");
        assert_eq!(prepared["input"][0]["input"], "text(42)");
        assert!(crate::request::audit_request(&prepared).is_empty());
        let mut collision = body.clone();
        collision["tools"] = json!([{"type":"function","name":"functions__exec","parameters":{}}]);
        assert!(Compatibility::prepare(&collision).is_err());
    }

    #[test]
    fn plaintext_agent_messages_translate_but_encrypted_messages_fail() {
        let mut body = json!({"model":"gpt-6-astra","input":[{
            "type":"agent_message","author":"/root/probe","recipient":"/root",
            "content":[{"type":"input_text","text":"Message Type: FINAL_ANSWER\nPayload:\nCHILD_OK"}]
        }]});
        let (prepared, _) = Compatibility::prepare(&body).unwrap();
        assert_eq!(prepared["input"][0]["role"], "user");
        let content = prepared["input"][0]["content"].as_str().unwrap();
        assert!(content.contains("/root/probe"));
        assert!(content.contains("CHILD_OK"));
        assert!(crate::request::audit_request(&prepared).is_empty());
        body["input"][0]["content"] =
            json!([{"type":"encrypted_content","encrypted_content":"opaque"}]);
        assert!(Compatibility::prepare(&body).is_err());
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
