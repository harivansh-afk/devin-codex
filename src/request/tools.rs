use super::{Problem, audit::*, optional_text, text};
use crate::{proto, tools::ToolKind};
use serde_json::{Value, json};
use std::collections::HashMap;

pub(super) fn audit<'a>(
    value: Option<&'a Value>,
    out: &mut Vec<Problem>,
) -> HashMap<&'a str, ToolKind> {
    let mut tool_types = HashMap::new();
    let Some(value) = value else {
        return tool_types;
    };
    let Some(tools) = value.as_array() else {
        bad(out, "tools", "Expected array.");
        return tool_types;
    };
    for (i, tool) in tools.iter().enumerate() {
        let path = format!("tools[{i}]");
        let Some(tool) = tool.as_object() else {
            bad(out, path, "Expected an object.");
            continue;
        };
        if field_str(tool, "type") == Some("namespace") {
            bad(
                out,
                path,
                "Gateway namespace identity is not established; flattening is disabled.",
            );
            continue;
        }
        let Some(kind) = field_str(tool, "type").and_then(ToolKind::from_definition) else {
            bad(
                out,
                path,
                "Only flat function/custom definitions have a verified wire representation.",
            );
            continue;
        };
        unknown(
            out,
            tool,
            match kind {
                ToolKind::Function => &["type", "name", "description", "parameters", "strict"],
                ToolKind::Custom => &["type", "name", "description", "format"],
            },
            &path,
        );
        if let Some(name) = field_str(tool, "name").filter(|s| name_valid(s)) {
            if tool_types.insert(name, kind).is_some() {
                bad(out, format!("{path}.name"), "Duplicate name.");
            }
        } else {
            bad(
                out,
                format!("{path}.name"),
                "Expected an unqualified tool name.",
            );
        }
        if tool.get("description").is_some_and(|v| !v.is_string()) {
            bad(out, format!("{path}.description"), "Expected string.");
        }
        if kind == ToolKind::Function {
            if !tool.get("parameters").is_some_and(Value::is_object) {
                bad(
                    out,
                    format!("{path}.parameters"),
                    "Expected JSON Schema object.",
                );
            }
            if tool.get("strict").is_some_and(|v| !v.is_boolean()) {
                bad(out, format!("{path}.strict"), "Expected boolean.");
            }
        } else if let Some(fmt) = tool.get("format") {
            match fmt.as_object() {
                Some(fmt) if field_str(fmt, "type") == Some("grammar") => {
                    if fmt.len() != 3
                        || field_str(fmt, "syntax") != Some("lark")
                        || !fmt.get("definition").is_some_and(Value::is_string)
                    {
                        bad(
                            out,
                            format!("{path}.format"),
                            "Only an unchanged Lark grammar is verified.",
                        );
                    }
                }
                Some(_) if fmt == &json!({"type":"text"}) => {}
                Some(_) => bad(
                    out,
                    format!("{path}.format"),
                    "Unsupported custom-tool format.",
                ),
                None => bad(out, format!("{path}.format"), "Expected object."),
            }
        }
    }
    tool_types
}

pub(super) fn translate(
    definitions: Option<&Value>,
) -> (Vec<proto::ToolDefinition>, HashMap<String, ToolKind>) {
    let mut tools = Vec::new();
    let mut tool_map = HashMap::new();
    for tool in definitions.and_then(Value::as_array).into_iter().flatten() {
        let kind = ToolKind::from_definition(text(&tool["type"])).expect("validated tool kind");
        let name = text(&tool["name"]).to_owned();
        let mut definition = proto::ToolDefinition {
            name: name.clone(),
            description: optional_text(&tool["description"]).into(),
            is_custom: kind == ToolKind::Custom,
            ..Default::default()
        };
        match kind {
            ToolKind::Custom if tool["format"]["type"].as_str() == Some("grammar") => {
                definition.grammar = text(&tool["format"]["definition"]).into();
                definition.grammar_syntax = text(&tool["format"]["syntax"]).into();
            }
            ToolKind::Custom => {}
            ToolKind::Function => {
                definition.parameters_json =
                    serde_json::to_string(&tool["parameters"]).expect("JSON schema serializes");
                definition.strict = tool.get("strict").and_then(Value::as_bool);
            }
        }
        tool_map.insert(name, kind);
        tools.push(definition);
    }
    (tools, tool_map)
}
