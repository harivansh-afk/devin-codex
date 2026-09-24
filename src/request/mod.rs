use crate::{config::Credentials, models::Models, proto, tools::ToolKind};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use uuid::Uuid;

mod audit;
mod history;
mod tools;

pub use audit::audit_request;

#[cfg(test)]
#[path = "../../tests/unit/request.rs"]
mod tests;

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

fn text(value: &Value) -> &str {
    value.as_str().expect("validated string")
}

fn optional_text(value: &Value) -> &str {
    value.as_str().unwrap_or_default()
}

pub fn translate_request(
    body: &Value,
    credentials: &Credentials,
) -> Result<Translation, Vec<Problem>> {
    translate_with_models(body, credentials, &Models::default())
}

pub fn translate_with_models(
    body: &Value,
    credentials: &Credentials,
    models: &Models,
) -> Result<Translation, Vec<Problem>> {
    let messages = audit::parse(body, models)?;
    let (tools, tool_map) = tools::translate(body.get("tools"));
    let effort = body
        .get("reasoning")
        .and_then(|r| r.get("effort"))
        .and_then(Value::as_str)
        .unwrap_or_else(|| {
            models
                .by_slug(text(&body["model"]))
                .map_or("medium", |m| m.default_effort())
        });
    let selected = models
        .by_slug(text(&body["model"]))
        .and_then(|model| model.variants.get(effort));
    let mode = body["tool_choice"].as_str().unwrap_or("auto");
    let request = proto::ChatRequest {
        metadata: Some(metadata(credentials)),
        system_prompt: optional_text(&body["instructions"]).into(),
        messages,
        request_type: 5,
        completion_config: Some(proto::CompletionConfig {
            num_completions: 1,
            max_tokens: body
                .get("max_output_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(128000)
                .min(selected.map_or(128000, |variant| variant.output)),
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
        model: selected
            .map(|variant| variant.uid.clone())
            .unwrap_or_else(|| format!("gpt-6-astra-{effort}")),
    };
    Ok(Translation {
        request,
        model: text(&body["model"]).into(),
        allow_tools: mode != "none",
        tool_map,
    })
}

fn metadata(credentials: &Credentials) -> proto::Metadata {
    let version = &credentials.client_version;
    proto::Metadata {
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
    }
}
