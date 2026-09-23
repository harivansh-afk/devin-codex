use super::{Problem, history, tools};
use crate::models::Models;
use serde_json::{Map, Value, json};

pub(super) fn bad(out: &mut Vec<Problem>, path: impl Into<String>, reason: &str) {
    out.push(Problem {
        path: path.into(),
        reason: reason.into(),
    });
}

pub(super) fn unknown(
    out: &mut Vec<Problem>,
    obj: &Map<String, Value>,
    allowed: &[&str],
    path: &str,
) {
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

pub(super) fn name_valid(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
}

pub(super) fn field_str<'a>(obj: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    obj.get(key).and_then(Value::as_str)
}

pub(super) fn str_is(value: Option<&Value>, choices: &[&str]) -> bool {
    value
        .and_then(Value::as_str)
        .is_some_and(|v| choices.contains(&v))
}

pub(super) fn nonempty(value: Option<&Value>) -> bool {
    value.and_then(Value::as_str).is_some_and(|v| !v.is_empty())
}

pub fn audit_request(body: &Value) -> Vec<Problem> {
    audit_with_models(body, &Models::default())
}

pub(super) fn audit_with_models(body: &Value, models: &Models) -> Vec<Problem> {
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
    if !body["model"]
        .as_str()
        .is_some_and(|model| model == "gpt-6-astra" || models.by_slug(model).is_some())
    {
        bad(
            &mut out,
            "model",
            "Model is not in the selected Devin catalog.",
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
                && !reasoning["effort"].as_str().is_some_and(|effort| {
                    if let Some(model) =
                        body["model"].as_str().and_then(|slug| models.by_slug(slug))
                    {
                        model.variants.contains_key(effort)
                    } else {
                        crate::compat::EFFORTS.contains(&effort)
                    }
                })
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
    let tool_types = tools::audit(obj.get("tools"), &mut out);
    match obj.get("input") {
        Some(Value::String(_)) => {}
        Some(Value::Array(items)) => history::audit(items, &tool_types, &mut out),
        _ => bad(&mut out, "input", "Expected text or an input array."),
    }
    out
}
