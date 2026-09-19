use crate::{config::Credentials, proto, responses::ToolKind};
use serde::Serialize;
use serde_json::{Map, Value, json};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Problem {
    pub path: String,
    pub reason: String,
}

pub struct Translation {
    pub request: proto::ChatRequest,
    pub model: String,
    pub tool_map: HashMap<String, ToolKind>,
    pub allow_tools: bool,
}

fn bad(out: &mut Vec<Problem>, path: impl Into<String>, reason: &str) {
    out.push(Problem {
        path: path.into(),
        reason: reason.into(),
    });
}

fn unknown(out: &mut Vec<Problem>, obj: &Map<String, Value>, allowed: &[&str], path: &str) {
    for key in obj.keys().filter(|key| !allowed.contains(&key.as_str())) {
        let key = if path.is_empty() {
            key.clone()
        } else {
            format!("{path}.{key}")
        };
        bad(
            out,
            key,
            "No verified gateway mapping; field will not be discarded.",
        );
    }
}

fn name_valid(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
}

fn str_is(value: Option<&Value>, choices: &[&str]) -> bool {
    value
        .and_then(Value::as_str)
        .is_some_and(|v| choices.contains(&v))
}

fn nonempty(value: Option<&Value>) -> bool {
    value.and_then(Value::as_str).is_some_and(|v| !v.is_empty())
}

pub fn audit_request(body: &Value) -> Vec<Problem> {
    let mut out = Vec::new();
    let Some(obj) = body.as_object() else {
        bad(&mut out, "$", "Expected a JSON object.");
        return out;
    };
    unknown(
        &mut out,
        obj,
        &[
            "model",
            "input",
            "instructions",
            "tools",
            "tool_choice",
            "parallel_tool_calls",
            "stream",
            "store",
            "reasoning",
            "include",
            "max_output_tokens",
            "temperature",
            "top_p",
        ],
        "",
    );
    if body.get("model").and_then(Value::as_str) != Some("gpt-6-astra") {
        bad(
            &mut out,
            "model",
            "This release verifies only Codex-facing gpt-6-astra.",
        );
    }
    if obj.get("store").is_some_and(|v| v != &Value::Bool(false)) {
        bad(
            &mut out,
            "store",
            "Stateful response storage is not implemented; send full history with store=false.",
        );
    }
    for key in ["stream", "parallel_tool_calls"] {
        if obj.get(key).is_some_and(|v| !v.is_boolean()) {
            bad(&mut out, key, "Expected boolean.");
        }
    }
    if obj.get("parallel_tool_calls") == Some(&Value::Bool(true)) {
        bad(
            &mut out,
            "parallel_tool_calls",
            "Parallel-call stream routing is not yet verified.",
        );
    }
    if obj.get("instructions").is_some_and(|v| !v.is_string()) {
        bad(&mut out, "instructions", "Expected a string.");
    }
    if let Some(reasoning) = obj.get("reasoning") {
        if let Some(reasoning) = reasoning.as_object() {
            unknown(&mut out, reasoning, &["effort"], "reasoning");
            if reasoning.contains_key("effort")
                && !str_is(
                    reasoning.get("effort"),
                    &["low", "medium", "high", "xhigh", "max"],
                )
            {
                bad(
                    &mut out,
                    "reasoning.effort",
                    "Requested effort is not in the observed Astra gateway catalog.",
                );
            }
        } else {
            bad(&mut out, "reasoning", "Expected an object.");
        }
    }
    if obj.contains_key("tool_choice") && !str_is(obj.get("tool_choice"), &["auto", "none"]) {
        bad(
            &mut out,
            "tool_choice",
            "Only auto and none have a defined encoding here.",
        );
    }
    if obj
        .get("include")
        .is_some_and(|v| v != &json!([]) && v != &json!(["reasoning.encrypted_content"]))
    {
        bad(
            &mut out,
            "include",
            "Only encrypted reasoning inclusion is supported.",
        );
    }
    for (key, max) in [("temperature", 2.0), ("top_p", 1.0)] {
        if let Some(value) = obj.get(key)
            && !value
                .as_f64()
                .is_some_and(|v| v.is_finite() && v > 0.0 && v <= max)
        {
            bad(
                &mut out,
                key,
                "Out of supported range; values are never clamped.",
            );
        }
    }
    if obj
        .get("max_output_tokens")
        .is_some_and(|v| !v.as_u64().is_some_and(|n| (1..=128000).contains(&n)))
    {
        bad(
            &mut out,
            "max_output_tokens",
            "Expected integer in 1..128000.",
        );
    }
    let mut tool_types: HashMap<&str, &str> = HashMap::new();
    if let Some(tools) = obj.get("tools") {
        if let Some(tools) = tools.as_array() {
            for (i, tool) in tools.iter().enumerate() {
                let path = format!("tools[{i}]");
                let Some(tool) = tool.as_object() else {
                    bad(&mut out, path, "Expected an object.");
                    continue;
                };
                let kind = tool.get("type").and_then(Value::as_str);
                if kind == Some("namespace") {
                    bad(
                        &mut out,
                        path,
                        "Gateway namespace identity is not established; flattening is disabled.",
                    );
                    continue;
                }
                if !matches!(kind, Some("function" | "custom")) {
                    bad(
                        &mut out,
                        path,
                        "Only flat function/custom definitions have a verified wire representation.",
                    );
                    continue;
                }
                let function = kind == Some("function");
                unknown(
                    &mut out,
                    tool,
                    if function {
                        &["type", "name", "description", "parameters", "strict"]
                    } else {
                        &["type", "name", "description", "format"]
                    },
                    &path,
                );
                if let Some(name) = tool
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|s| name_valid(s))
                {
                    if tool_types.insert(name, kind.unwrap()).is_some() {
                        bad(&mut out, format!("{path}.name"), "Duplicate name.");
                    }
                } else {
                    bad(
                        &mut out,
                        format!("{path}.name"),
                        "Expected an unqualified tool name.",
                    );
                }
                if tool.get("description").is_some_and(|v| !v.is_string()) {
                    bad(&mut out, format!("{path}.description"), "Expected string.");
                }
                if function {
                    if !tool.get("parameters").is_some_and(Value::is_object) {
                        bad(
                            &mut out,
                            format!("{path}.parameters"),
                            "Expected JSON Schema object.",
                        );
                    }
                    if tool.get("strict").is_some_and(|v| !v.is_boolean()) {
                        bad(&mut out, format!("{path}.strict"), "Expected boolean.");
                    }
                } else if let Some(fmt) = tool.get("format") {
                    match fmt.as_object() {
                        Some(fmt) if fmt.get("type").and_then(Value::as_str) == Some("grammar") => {
                            if fmt.len() != 3
                                || fmt.get("syntax").and_then(Value::as_str) != Some("lark")
                                || !fmt.get("definition").is_some_and(Value::is_string)
                            {
                                bad(
                                    &mut out,
                                    format!("{path}.format"),
                                    "Only an unchanged Lark grammar is verified.",
                                );
                            }
                        }
                        Some(_) if fmt == &json!({"type":"text"}) => {}
                        Some(_) => bad(
                            &mut out,
                            format!("{path}.format"),
                            "Unsupported custom-tool format.",
                        ),
                        None => bad(&mut out, format!("{path}.format"), "Expected object."),
                    }
                }
            }
        } else {
            bad(&mut out, "tools", "Expected array.");
        }
    }
    let items = match obj.get("input") {
        Some(Value::String(_)) => return out,
        Some(Value::Array(items)) => items,
        _ => {
            bad(&mut out, "input", "Expected text or an input array.");
            return out;
        }
    };
    let mut calls: HashMap<&str, &str> = HashMap::new();
    let mut all_call_ids = HashSet::new();
    let mut pending_reasoning = false;
    for (i, item) in items.iter().enumerate() {
        let path = format!("input[{i}]");
        let Some(item) = item.as_object() else {
            bad(&mut out, path, "Expected object.");
            continue;
        };
        if item.contains_key("id") && !nonempty(item.get("id")) {
            bad(
                &mut out,
                format!("{path}.id"),
                "Expected nonempty string ID.",
            );
        }
        let kind = item
            .get("type")
            .map(Value::as_str)
            .unwrap_or(Some("message"));
        if let Some(status) = item.get("status") {
            let allowed = if kind == Some("reasoning") {
                &["", "completed"][..]
            } else {
                &["completed"][..]
            };
            if !str_is(Some(status), allowed) {
                bad(
                    &mut out,
                    format!("{path}.status"),
                    "Only completed history or an observed empty reasoning status is supported.",
                );
            }
        }
        let allowed: &[&str] = match kind {
            Some("additional_tools") => {
                bad(
                    &mut out,
                    path,
                    "Responses Lite AdditionalTools placement and namespaces are unverified.",
                );
                continue;
            }
            Some("message") => {
                let role = item.get("role").and_then(Value::as_str);
                if !matches!(role, Some("user" | "assistant")) {
                    bad(
                        &mut out,
                        format!("{path}.role"),
                        "Developer/system message boundaries cannot be preserved by concatenation.",
                    );
                }
                match item.get("content") {
                    Some(Value::String(_)) => {}
                    Some(Value::Array(parts)) => {
                        let expected = if role == Some("assistant") {
                            "output_text"
                        } else {
                            "input_text"
                        };
                        if parts.len() != 1
                            || !parts[0].is_object()
                            || parts[0].get("type").and_then(Value::as_str) != Some(expected)
                            || !parts[0].get("text").is_some_and(Value::is_string)
                        {
                            bad(
                                &mut out,
                                format!("{path}.content"),
                                "Only one text part is supported; no flattening or image substitution.",
                            );
                        } else {
                            let part = parts[0].as_object().unwrap();
                            unknown(
                                &mut out,
                                part,
                                if expected == "output_text" {
                                    &["type", "text", "annotations", "logprobs"]
                                } else {
                                    &["type", "text"]
                                },
                                &format!("{path}.content[0]"),
                            );
                            for key in ["annotations", "logprobs"] {
                                if part.get(key).is_some_and(|v| v != &json!([])) {
                                    bad(
                                        &mut out,
                                        format!("{path}.content[0].{key}"),
                                        "Only empty annotation/logprob arrays are supported.",
                                    );
                                }
                            }
                        }
                    }
                    _ => bad(&mut out, format!("{path}.content"), "Expected text."),
                }
                if item.contains_key("phase")
                    && (role != Some("assistant")
                        || !str_is(item.get("phase"), &["commentary", "final_answer"]))
                {
                    bad(&mut out, format!("{path}.phase"), "Unknown phase.");
                }
                if role == Some("assistant") {
                    pending_reasoning = false;
                } else if pending_reasoning {
                    bad(
                        &mut out,
                        &path,
                        "Reasoning must attach to an assistant turn before user input.",
                    );
                }
                &["type", "id", "role", "content", "status", "phase"]
            }
            Some("function_call" | "custom_tool_call") => {
                let kind = kind.unwrap();
                if let Some(id) = item
                    .get("call_id")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                {
                    if !all_call_ids.insert(id) {
                        bad(&mut out, format!("{path}.call_id"), "Duplicate call ID.");
                    }
                    calls.insert(id, kind);
                } else {
                    bad(&mut out, format!("{path}.call_id"), "Missing call ID.");
                }
                let expected = if kind == "function_call" {
                    "function"
                } else {
                    "custom"
                };
                match item
                    .get("name")
                    .and_then(Value::as_str)
                    .and_then(|name| tool_types.get(name))
                {
                    Some(found) if *found != expected => bad(
                        &mut out,
                        format!("{path}.type"),
                        "Call kind does not match its declared tool.",
                    ),
                    None => bad(
                        &mut out,
                        format!("{path}.name"),
                        "Call has no matching declared tool.",
                    ),
                    _ => {}
                }
                let input = if kind == "function_call" {
                    "arguments"
                } else {
                    "input"
                };
                if !item.get(input).is_some_and(Value::is_string) {
                    bad(&mut out, &path, "Tool input must be the original string.");
                }
                pending_reasoning = false;
                if kind == "function_call" {
                    &["type", "id", "call_id", "name", "arguments", "status"]
                } else {
                    &["type", "id", "call_id", "name", "input", "status"]
                }
            }
            Some("function_call_output" | "custom_tool_call_output") => {
                let expected = if kind == Some("function_call_output") {
                    "function_call"
                } else {
                    "custom_tool_call"
                };
                match item
                    .get("call_id")
                    .and_then(Value::as_str)
                    .and_then(|id| calls.remove(id))
                {
                    Some(found) if found != expected => bad(
                        &mut out,
                        format!("{path}.type"),
                        "Tool result kind does not match its call.",
                    ),
                    None => bad(
                        &mut out,
                        format!("{path}.call_id"),
                        "Orphan or already answered tool result.",
                    ),
                    _ => {}
                }
                if !item.get("output").is_some_and(Value::is_string) {
                    bad(
                        &mut out,
                        format!("{path}.output"),
                        "Only text tool results are supported.",
                    );
                }
                if pending_reasoning {
                    bad(&mut out, &path, "Reasoning cannot attach to a tool result.");
                }
                &["type", "id", "call_id", "output", "status"]
            }
            Some("reasoning") => {
                if !nonempty(item.get("encrypted_content")) {
                    bad(
                        &mut out,
                        &path,
                        "Opaque reasoning replay requires encrypted_content.",
                    );
                }
                for (key, part_type) in [("summary", "summary_text"), ("content", "reasoning_text")]
                {
                    if let Some(parts) = item.get(key) {
                        let valid = parts.as_array().is_some_and(|parts| {
                            parts.iter().all(|part| {
                                part.as_object().is_some_and(|part| {
                                    part.len() == 2
                                        && part.get("type").and_then(Value::as_str)
                                            == Some(part_type)
                                        && part.get("text").is_some_and(Value::is_string)
                                })
                            })
                        });
                        if !valid {
                            bad(
                                &mut out,
                                format!("{path}.{key}"),
                                "Expected reasoning text parts with no additional properties.",
                            );
                        }
                    }
                }
                pending_reasoning = true;
                &[
                    "type",
                    "id",
                    "summary",
                    "content",
                    "encrypted_content",
                    "status",
                ]
            }
            _ => {
                bad(
                    &mut out,
                    path,
                    "Unsupported input item; encrypted agents and compaction are not translated.",
                );
                continue;
            }
        };
        unknown(&mut out, item, allowed, &path);
    }
    if pending_reasoning {
        bad(
            &mut out,
            "input",
            "Trailing reasoning has no assistant message to attach to.",
        );
    }
    if !calls.is_empty() {
        bad(
            &mut out,
            "input",
            "Unanswered tool calls remain in input history.",
        );
    }
    out
}

pub fn translate_request(
    body: &Value,
    credentials: &Credentials,
) -> Result<Translation, Vec<Problem>> {
    let problems = audit_request(body);
    if !problems.is_empty() {
        return Err(problems);
    }
    let version = &credentials.client_version;
    let metadata = proto::Metadata {
        client_name: "devin-cli".into(),
        client_version: version.clone(),
        api_key: credentials.api_key.clone(),
        locale: "en".into(),
        platform: if cfg!(target_os = "macos") {
            "darwin"
        } else {
            "linux"
        }
        .into(),
        ide_version: version.clone(),
        extension_name: "chisel".into(),
        ide_type: "chisel".into(),
        device_id: credentials.device_id.clone(),
    };
    let owned;
    let items = if let Some(text) = body["input"].as_str() {
        owned = vec![json!({"role":"user", "content":text})];
        &owned
    } else {
        body["input"].as_array().expect("validated input")
    };
    let mut messages = Vec::new();
    let mut reasoning = Vec::new();
    for item in items {
        let kind = item
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("message");
        if kind == "reasoning" {
            reasoning.push(item.clone());
            continue;
        }
        let mut message = proto::ChatMessage {
            id: item
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| Uuid::new_v4().to_string()),
            ..Default::default()
        };
        match kind {
            "message" => {
                message.role = if item["role"] == "user" { 1 } else { 2 };
                message.content = item["content"]
                    .as_str()
                    .unwrap_or_else(|| item["content"][0]["text"].as_str().expect("validated text"))
                    .into();
                message.phase = item
                    .get("phase")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .into();
            }
            "function_call" | "custom_tool_call" => {
                let custom = kind == "custom_tool_call";
                message.role = 2;
                message.tool_calls.push(proto::ToolCall {
                    id: item["call_id"].as_str().unwrap().into(),
                    name: item["name"].as_str().unwrap().into(),
                    arguments_json: item[if custom { "input" } else { "arguments" }]
                        .as_str()
                        .unwrap()
                        .into(),
                    is_custom_tool_call: custom.then_some(true),
                    ..Default::default()
                });
            }
            "function_call_output" | "custom_tool_call_output" => {
                message.role = 4;
                message.content = item["output"].as_str().unwrap().into();
                message.tool_call_id = item["call_id"].as_str().unwrap().into();
            }
            _ => unreachable!("audit rejected unknown input kind"),
        }
        if !reasoning.is_empty() {
            message.reasoning = serde_json::to_string(&reasoning).expect("JSON values serialize");
            message.reasoning_type = "openai".into();
            reasoning.clear();
        }
        messages.push(message);
    }
    let mut tools = Vec::new();
    let mut tool_map = HashMap::new();
    if let Some(definitions) = body.get("tools").and_then(Value::as_array) {
        for tool in definitions {
            let custom = tool["type"] == "custom";
            let name = tool["name"].as_str().unwrap().to_owned();
            let mut definition = proto::ToolDefinition {
                name: name.clone(),
                description: tool
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .into(),
                is_custom: custom,
                ..Default::default()
            };
            if custom {
                if tool
                    .get("format")
                    .and_then(|f| f.get("type"))
                    .and_then(Value::as_str)
                    == Some("grammar")
                {
                    definition.grammar = tool["format"]["definition"].as_str().unwrap().into();
                    definition.grammar_syntax = tool["format"]["syntax"].as_str().unwrap().into();
                }
                tool_map.insert(name, ToolKind::Custom);
            } else {
                definition.parameters_json =
                    serde_json::to_string(&tool["parameters"]).expect("JSON schema serializes");
                definition.strict = tool.get("strict").and_then(Value::as_bool);
                tool_map.insert(name, ToolKind::Function);
            }
            tools.push(definition);
        }
    }
    let effort = body
        .get("reasoning")
        .and_then(|r| r.get("effort"))
        .and_then(Value::as_str)
        .unwrap_or("medium");
    let mode = body
        .get("tool_choice")
        .and_then(Value::as_str)
        .unwrap_or("auto");
    let request = proto::ChatRequest {
        metadata: Some(metadata),
        system_prompt: body
            .get("instructions")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .into(),
        messages,
        request_type: 5,
        completion_config: Some(proto::CompletionConfig {
            num_completions: 1,
            max_tokens: body
                .get("max_output_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(128000),
            max_newlines: 400,
            temperature: body
                .get("temperature")
                .and_then(Value::as_f64)
                .unwrap_or(1.0),
            top_k: 40,
            top_p: body
                .get("top_p")
                .and_then(Value::as_f64)
                .unwrap_or(0.9500000476837158),
        }),
        tools,
        disable_parallel_tool_calls: true,
        tool_choice: Some(proto::ToolChoice { mode: mode.into() }),
        trajectory: Some(proto::Trajectory {
            id: Uuid::new_v4().to_string(),
            source: 4,
            node_index: 0,
        }),
        cascade_id: Uuid::new_v4().to_string(),
        planner_mode: 1,
        model: format!("gpt-6-astra-{effort}"),
    };
    Ok(Translation {
        request,
        model: "gpt-6-astra".into(),
        allow_tools: mode != "none",
        tool_map,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn creds() -> Credentials {
        Credentials {
            api_key: "test-key".into(),
            endpoint: "https://example.invalid".into(),
            device_id: "test-device".into(),
            client_version: "3000.10.31".into(),
        }
    }
    fn base() -> Value {
        json!({"model":"gpt-6-astra", "input":"hello"})
    }
    fn history() -> Value {
        json!({"model":"gpt-6-astra", "tools":[{"type":"function", "name":"run", "parameters":{"type":"object"}}], "input":[
            {"type":"function_call", "call_id":"c1", "name":"run", "arguments":"{\n}"},
            {"type":"function_call_output", "call_id":"c1", "output":"done"}
        ]})
    }
    #[test]
    fn minimal_and_history_translate() {
        for body in [base(), history()] {
            assert!(audit_request(&body).is_empty());
            assert!(translate_request(&body, &creds()).is_ok());
        }
        let t = translate_request(&history(), &creds()).ok().unwrap();
        assert_eq!(t.request.messages[0].tool_calls[0].arguments_json, "{\n}");
        assert_eq!(t.request.messages[1].tool_call_id, "c1");
        assert_eq!(t.request.model, "gpt-6-astra-medium");
    }
    #[test]
    fn invalid_json_types_never_panic() {
        for value in [Value::Null, json!([]), json!({}), json!(1), json!(true)] {
            for target in [
                "effort",
                "name",
                "call_id",
                "result_id",
                "item_id",
                "status",
                "type",
            ] {
                let mut body = history();
                match target {
                    "effort" => body["reasoning"] = json!({"effort":value}),
                    "result_id" => body["input"][1]["call_id"] = value.clone(),
                    "item_id" => body["input"][0]["id"] = value.clone(),
                    key => body["input"][0][key] = value.clone(),
                }
                assert!(
                    translate_request(&body, &creds()).is_err(),
                    "{target}: {value}"
                );
            }
        }
    }
    #[test]
    fn reasoning_and_phase_preserved() {
        let reasoning = json!({"type":"reasoning", "id":"r1", "encrypted_content":"opaque", "status":"", "content":[], "summary":[{"type":"summary_text", "text":"summary"}]});
        let mut body = base();
        body["input"] = json!([reasoning, {"role":"assistant", "content":[{"type":"output_text", "text":"hello", "annotations":[], "logprobs":[]}], "phase":"commentary"}]);
        let t = translate_request(&body, &creds()).ok().unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&t.request.messages[0].reasoning).unwrap(),
            json!([reasoning])
        );
        assert_eq!(t.request.messages[0].reasoning_type, "openai");
        assert_eq!(t.request.messages[0].phase, "commentary");
    }
    #[test]
    fn grammar_and_false_strict_preserved() {
        let mut body = history();
        body["tools"][0]["strict"] = json!(false);
        body["tools"].as_array_mut().unwrap().push(json!({"type":"custom", "name":"patch", "description":"Original\nDescription", "format":{"type":"grammar", "syntax":"lark", "definition":"start: \"ok\"\n"}}));
        let t = translate_request(&body, &creds()).ok().unwrap();
        assert_eq!(t.request.tools[0].strict, Some(false));
        assert_eq!(t.request.tools[1].grammar, "start: \"ok\"\n");
        assert_eq!(t.request.tools[1].description, "Original\nDescription");
        assert_eq!(t.tool_map["patch"], ToolKind::Custom);
    }
    #[test]
    fn unsupported_features_fail_closed() {
        for item in [
            json!({"role":"developer", "content":"keep exact"}),
            json!({"role":"system", "content":"keep exact"}),
            json!({"type":"additional_tools", "role":"developer", "tools":[]}),
            json!({"role":"assistant", "content":[{"type":"output_text", "text":"x", "annotations":{}}]}),
            json!({"role":"user", "content":[{"type":"input_text", "text":"x", "annotations":[]}]}),
            json!({"role":"user", "content":"x", "phase":"commentary"}),
            json!({"role":"assistant", "content":"x", "unknown":false}),
            json!({"type":"reasoning", "encrypted_content":"opaque", "summary":[{"type":"summary_text", "text":"x", "other":0}]}),
        ] {
            let mut body = base();
            body["input"] = json!([item]);
            assert!(!audit_request(&body).is_empty(), "{item}");
        }
        let mut body = base();
        body["tools"] = json!([{"type":"namespace", "name":"functions", "tools":[]}]);
        assert!(!audit_request(&body).is_empty());
        body = base();
        body["future_field"] = json!(false);
        assert!(!audit_request(&body).is_empty());
    }
    #[test]
    fn tool_history_integrity_checked() {
        for case in [
            "kind",
            "duplicate_output",
            "orphan",
            "duplicate_call",
            "unanswered",
        ] {
            let mut body = history();
            match case {
                "kind" => body["input"][1]["type"] = json!("custom_tool_call_output"),
                "duplicate_output" => {
                    let x = body["input"][1].clone();
                    body["input"].as_array_mut().unwrap().push(x);
                }
                "orphan" => body["input"][1]["call_id"] = json!("other"),
                "duplicate_call" => {
                    let calls = body["input"].as_array().unwrap().clone();
                    body["input"].as_array_mut().unwrap().extend(calls);
                }
                "unanswered" => {
                    body["input"].as_array_mut().unwrap().pop();
                }
                _ => unreachable!(),
            }
            assert!(!audit_request(&body).is_empty(), "{case}");
        }
    }
    #[test]
    fn reasoning_never_attaches_to_nonassistant() {
        let r = json!({"type":"reasoning", "encrypted_content":"opaque", "summary":[]});
        let mut body = history();
        body["input"].as_array_mut().unwrap().insert(1, r.clone());
        assert!(!audit_request(&body).is_empty());
        body["input"] = json!([r, {"role":"user", "content":"hello"}]);
        assert!(!audit_request(&body).is_empty());
    }
    #[test]
    fn ranges_and_no_tools_choice() {
        for (key, value) in [
            ("temperature", json!(0)),
            ("top_p", json!(2)),
            ("max_output_tokens", json!(true)),
            ("max_output_tokens", json!(128001)),
            ("max_output_tokens", json!(1.0)),
            ("store", json!(true)),
            ("parallel_tool_calls", json!(true)),
        ] {
            let mut b = base();
            b[key] = value;
            assert!(!audit_request(&b).is_empty(), "{key}");
        }
        let mut b = base();
        b["tool_choice"] = json!("none");
        let t = translate_request(&b, &creds()).ok().unwrap();
        assert!(!t.allow_tools);
        assert_eq!(t.request.tool_choice.unwrap().mode, "none");
    }
}
