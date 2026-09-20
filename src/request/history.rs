use super::{Problem, audit::*, optional_text, text};
use crate::{proto, tools::ToolKind};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

fn text_part(value: &Value, kind: &str) -> bool {
    value["type"].as_str() == Some(kind) && value["text"].is_string()
}

fn audit_content(content: Option<&Value>, role: Option<&str>, path: &str, out: &mut Vec<Problem>) {
    match content {
        Some(Value::String(_)) => {}
        Some(Value::Array(parts)) => {
            let expected = if role == Some("assistant") {
                "output_text"
            } else {
                "input_text"
            };
            if parts.len() != 1 || !text_part(&parts[0], expected) {
                bad(
                    out,
                    format!("{path}.content"),
                    "Only one text part is supported; no flattening or image substitution.",
                );
            } else {
                let part = parts[0].as_object().unwrap();
                unknown(
                    out,
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
                            out,
                            format!("{path}.content[0].{key}"),
                            "Only empty annotation/logprob arrays are supported.",
                        );
                    }
                }
            }
        }
        _ => bad(out, format!("{path}.content"), "Expected text."),
    }
}

pub(super) fn audit(items: &[Value], tool_types: &HashMap<&str, ToolKind>, out: &mut Vec<Problem>) {
    let mut calls = HashMap::new();
    let mut all_call_ids = HashSet::new();
    let mut pending_reasoning = false;
    for (i, item) in items.iter().enumerate() {
        let path = format!("input[{i}]");
        let Some(item) = item.as_object() else {
            bad(out, path, "Expected object.");
            continue;
        };
        if item.contains_key("id") && !nonempty(item.get("id")) {
            bad(out, format!("{path}.id"), "Expected nonempty string ID.");
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
                    out,
                    format!("{path}.status"),
                    "Only completed history or an observed empty reasoning status is supported.",
                );
            }
        }
        let allowed: &[&str] = match kind {
            Some("additional_tools") => {
                bad(
                    out,
                    path,
                    "Responses Lite AdditionalTools placement and namespaces are unverified.",
                );
                continue;
            }
            Some("message") => {
                let role = field_str(item, "role");
                if !matches!(role, Some("user" | "assistant")) {
                    bad(
                        out,
                        format!("{path}.role"),
                        "Developer/system message boundaries cannot be preserved by concatenation.",
                    );
                }
                audit_content(item.get("content"), role, &path, out);
                if item.contains_key("phase")
                    && (role != Some("assistant")
                        || !str_is(item.get("phase"), &["commentary", "final_answer"]))
                {
                    bad(out, format!("{path}.phase"), "Unknown phase.");
                }
                if role == Some("assistant") {
                    pending_reasoning = false;
                } else if pending_reasoning {
                    bad(
                        out,
                        &path,
                        "Reasoning must attach to an assistant turn before user input.",
                    );
                }
                &["type", "id", "role", "content", "status", "phase"]
            }
            Some(kind @ ("function_call" | "custom_tool_call")) => {
                let kind = ToolKind::from_call(kind).expect("matched tool call");
                if let Some(id) = field_str(item, "call_id").filter(|s| !s.is_empty()) {
                    if !all_call_ids.insert(id) {
                        bad(out, format!("{path}.call_id"), "Duplicate call ID.");
                    }
                    calls.insert(id, kind);
                } else {
                    bad(out, format!("{path}.call_id"), "Missing call ID.");
                }
                match field_str(item, "name").filter(|name| name_valid(name)) {
                    Some(name) if tool_types.get(name).is_some_and(|found| *found != kind) => {
                        bad(
                            out,
                            format!("{path}.type"),
                            "Call kind does not match its declared tool.",
                        );
                    }
                    None => bad(
                        out,
                        format!("{path}.name"),
                        "Expected an unqualified historical tool name.",
                    ),
                    _ => {}
                }
                if !item.get(kind.input_key()).is_some_and(Value::is_string) {
                    bad(out, &path, "Tool input must be the original string.");
                }
                pending_reasoning = false;
                kind.call_fields()
            }
            Some(kind @ ("function_call_output" | "custom_tool_call_output")) => {
                let expected = ToolKind::from_output(kind).expect("matched tool result");
                match field_str(item, "call_id").and_then(|id| calls.remove(id)) {
                    Some(found) if found != expected => {
                        bad(
                            out,
                            format!("{path}.type"),
                            "Tool result kind does not match its call.",
                        );
                    }
                    None => bad(
                        out,
                        format!("{path}.call_id"),
                        "Orphan or already answered tool result.",
                    ),
                    _ => {}
                }
                if !item.get("output").is_some_and(Value::is_string) {
                    bad(
                        out,
                        format!("{path}.output"),
                        "Only text tool results are supported.",
                    );
                }
                if pending_reasoning {
                    bad(out, &path, "Reasoning cannot attach to a tool result.");
                }
                &["type", "id", "call_id", "output", "status"]
            }
            Some("reasoning") => {
                if !nonempty(item.get("encrypted_content")) {
                    bad(
                        out,
                        &path,
                        "Opaque reasoning replay requires encrypted_content.",
                    );
                }
                for (key, part_type) in [("summary", "summary_text"), ("content", "reasoning_text")]
                {
                    if let Some(parts) = item.get(key) {
                        if key == "content" && parts.is_null() {
                            continue;
                        }
                        if !parts.as_array().is_some_and(|parts| {
                            parts.iter().all(|part| {
                                part.as_object().is_some_and(|p| p.len() == 2)
                                    && text_part(part, part_type)
                            })
                        }) {
                            bad(
                                out,
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
                    out,
                    path,
                    "Unsupported input item; encrypted agents and compaction are not translated.",
                );
                continue;
            }
        };
        unknown(out, item, allowed, &path);
    }
    if pending_reasoning {
        bad(
            out,
            "input",
            "Trailing reasoning has no assistant message to attach to.",
        );
    }
    if !calls.is_empty() {
        bad(
            out,
            "input",
            "Unanswered tool calls remain in input history.",
        );
    }
}

pub(super) fn translate(input: &Value) -> Vec<proto::ChatMessage> {
    let owned;
    let items = if let Some(text) = input.as_str() {
        owned = vec![json!({"role":"user", "content":text})];
        &owned
    } else {
        input.as_array().expect("validated input")
    };
    let mut messages = Vec::new();
    let mut reasoning = Vec::new();
    for item in items {
        let kind = item["type"].as_str().unwrap_or("message");
        if kind == "reasoning" {
            reasoning.push(item.clone());
            continue;
        }
        let mut message = proto::ChatMessage {
            id: item["id"]
                .as_str()
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
                message.phase = optional_text(&item["phase"]).into();
            }
            "function_call" | "custom_tool_call" => {
                let kind = ToolKind::from_call(kind).expect("validated tool call");
                message.role = 2;
                message.tool_calls.push(proto::ToolCall {
                    id: text(&item["call_id"]).into(),
                    name: text(&item["name"]).into(),
                    arguments_json: text(&item[kind.input_key()]).into(),
                    is_custom_tool_call: (kind == ToolKind::Custom).then_some(true),
                    ..Default::default()
                });
            }
            "function_call_output" | "custom_tool_call_output" => {
                message.role = 4;
                message.content = text(&item["output"]).into();
                message.tool_call_id = text(&item["call_id"]).into();
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
    messages
}
