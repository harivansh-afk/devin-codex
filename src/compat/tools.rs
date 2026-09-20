use super::{Problem, problem};
use crate::tools::ToolKind;
use serde_json::{Value, json};
use std::collections::BTreeMap;

type Identity = (Option<String>, String);

#[derive(Default)]
pub(super) struct ToolNames {
    names: BTreeMap<String, Identity>,
    descriptions: BTreeMap<String, String>,
}

fn plaintext_agent_tool(namespace: Option<&str>, name: &str) -> bool {
    namespace == Some("collaboration")
        && matches!(name, "spawn_agent" | "send_message" | "followup_task")
}

impl ToolNames {
    fn register(
        &mut self,
        namespace: Option<&str>,
        name: &str,
        collision: &str,
    ) -> Result<String, Vec<Problem>> {
        let alias = namespace.map_or_else(|| name.to_owned(), |ns| format!("{ns}__{name}"));
        let identity = (namespace.map(str::to_owned), name.to_owned());
        if self
            .names
            .get(&alias)
            .is_some_and(|existing| existing != &identity)
        {
            return Err(problem(collision));
        }
        self.names.insert(alias.clone(), identity);
        Ok(alias)
    }

    pub fn flatten(
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
                self.flatten(
                    tool["tools"]
                        .as_array()
                        .ok_or_else(|| problem("Expected namespace tools."))?,
                    Some(name),
                    out,
                )?;
                continue;
            }
            let Some(kind) = tool["type"].as_str().and_then(ToolKind::from_definition) else {
                return Err(problem(
                    "Only client-executed function and custom tools are supported.",
                ));
            };
            let alias = self.register(namespace, name, "Flattened tool names collide.")?;
            let mut tool = tool.clone();
            if kind == ToolKind::Function && plaintext_agent_tool(namespace, name) {
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

    pub fn prepare_call(&mut self, item: &mut Value) -> Result<(), Vec<Problem>> {
        let name = item["name"]
            .as_str()
            .ok_or_else(|| problem("History tool name is missing."))?;
        let namespace = item["namespace"].as_str();
        let alias = self.register(namespace, name, "Flattened historical tool names collide.")?;
        if item["type"] == "function_call"
            && plaintext_agent_tool(namespace, name)
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
        Ok(())
    }

    pub fn restore_call(&self, item: &mut Value) {
        let Some(kind) = item["type"].as_str().and_then(ToolKind::from_call) else {
            return;
        };
        if let Some((namespace, name)) = item["name"].as_str().and_then(|name| self.names.get(name))
        {
            item["name"] = json!(name);
            if kind == ToolKind::Function && plaintext_agent_tool(namespace.as_deref(), name) {
                item["encrypted_function_args"] = json!([]);
            }
            if let Some(namespace) = namespace {
                item["namespace"] = json!(namespace);
            }
        }
    }

    pub fn append_instructions(&self, instructions: &mut String) {
        for (namespace, description) in &self.descriptions {
            instructions.push_str(&format!("\n\nNamespace {namespace}:\n{description}"));
        }
        if self.names.values().any(|(ns, _)| ns.is_some()) {
            instructions.push_str("\n\nTool transport names: use the flat name below when calling a tool. Names inside JavaScript passed to exec are unchanged.\n");
            for (alias, (ns, name)) in &self.names {
                if let Some(ns) = ns {
                    instructions.push_str(&format!("{ns}.{name} = {alias}\n"));
                }
            }
        }
    }
}
