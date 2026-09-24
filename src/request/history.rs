use super::{Problem, audit::*};
use crate::{content::Content, proto, tools::ToolKind};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

fn text_part(value: &Value, kind: &str) -> bool {
    value["type"].as_str() == Some(kind) && value["text"].is_string()
}

fn content(value: Option<&Value>, assistant: bool, path: &str, out: &mut Vec<Problem>) -> Content {
    match Content::parse(value.unwrap_or(&Value::Null), assistant) {
        Ok(content) => content,
        Err(reason) => {
            bad(out, path, reason);
            Content::default()
        }
    }
}

pub(super) fn parse(
    items: &[Value],
    tool_types: &HashMap<&str, ToolKind>,
    out: &mut Vec<Problem>,
) -> Vec<proto::ChatMessage> {
    let mut calls = HashMap::new();
    let mut all_call_ids = HashSet::new();
    let mut reasoning = Vec::new();
    let mut messages = Vec::new();
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
        let mut message = proto::ChatMessage {
            id: field_str(item, "id")
                .map(str::to_owned)
                .unwrap_or_else(|| Uuid::new_v4().to_string()),
            ..Default::default()
        };
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
                let content = content(
                    item.get("content"),
                    role == Some("assistant"),
                    &format!("{path}.content"),
                    out,
                );
                message.role = if role == Some("assistant") { 2 } else { 1 };
                message.content = content.text;
                message.images = content.images;
                message.phase = field_str(item, "phase").unwrap_or_default().into();
                if item.contains_key("phase")
                    && (role != Some("assistant")
                        || !str_is(item.get("phase"), &["commentary", "final_answer"]))
                {
                    bad(out, format!("{path}.phase"), "Unknown phase.");
                }
                if role != Some("assistant") && !reasoning.is_empty() {
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
                message.role = 2;
                message.tool_calls.push(proto::ToolCall {
                    id: field_str(item, "call_id").unwrap_or_default().into(),
                    name: field_str(item, "name").unwrap_or_default().into(),
                    arguments_json: field_str(item, kind.input_key()).unwrap_or_default().into(),
                    is_custom_tool_call: (kind == ToolKind::Custom).then_some(true),
                    ..Default::default()
                });
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
                let content = content(item.get("output"), false, &format!("{path}.output"), out);
                message.role = 4;
                message.content = content.text;
                message.images = content.images;
                message.tool_call_id = field_str(item, "call_id").unwrap_or_default().into();
                if !reasoning.is_empty() {
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
        if kind == Some("reasoning") {
            reasoning.push(items[i].clone());
        } else {
            if message.role == 2 && !reasoning.is_empty() {
                message.reasoning =
                    serde_json::to_string(&reasoning).expect("JSON values serialize");
                message.reasoning_type = "openai".into();
                reasoning.clear();
            }
            messages.push(message);
        }
    }
    if !reasoning.is_empty() {
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
    messages
}
