use super::Error;
use serde_json::{Value, json};

#[derive(Clone, Default)]
pub(super) struct ReasoningSignature {
    raw: String,
    kind: String,
}

impl ReasoningSignature {
    pub(super) fn push(&mut self, delta: &str, kind: &str) -> Result<(), Error> {
        self.raw.push_str(delta);
        if !kind.is_empty() {
            if kind != "openai" {
                return Err(Error("Unsupported gateway reasoning signature type"));
            }
            self.kind = kind.to_owned();
        }
        Ok(())
    }

    pub(super) fn items(&self, available: usize) -> Result<Vec<Value>, Error> {
        if self.kind != "openai" || self.raw.is_empty() {
            return Err(Error("Reasoning lacks replayable OpenAI signature"));
        }
        let mut items: Vec<Value> =
            serde_json::from_str(&self.raw).map_err(|_| Error("Malformed reasoning signature"))?;
        if items.is_empty() {
            return Err(Error("Empty reasoning signature cannot be replayed"));
        }
        if items.len() > available {
            return Err(Error("Gateway response exceeds output item limit"));
        }
        for item in &mut items {
            validate(item)?;
            if item.get("summary").is_none() {
                item["summary"] = json!([]);
            }
        }
        Ok(items)
    }

    pub(super) fn shape(&self) -> Option<String> {
        let items: Vec<Value> = serde_json::from_str(&self.raw).ok()?;
        let reasoning = items
            .iter()
            .filter(|item| item["type"] == "reasoning")
            .count();
        Some(format!(
            "signature_items={}, reasoning_items={reasoning}, other_items={}",
            items.len(),
            items.len() - reasoning,
        ))
    }
}

fn validate(item: &Value) -> Result<(), Error> {
    let object = item.as_object().ok_or(Error("Invalid reasoning item"))?;
    if object.keys().any(|key| {
        !matches!(
            key.as_str(),
            "type" | "id" | "summary" | "content" | "encrypted_content" | "status"
        )
    }) {
        return Err(Error("Unsupported reasoning properties"));
    }
    if item["type"] != "reasoning"
        || item["id"].as_str().is_none_or(str::is_empty)
        || item["encrypted_content"].as_str().is_none_or(str::is_empty)
    {
        return Err(Error("Invalid replayable reasoning identity"));
    }
    if let Some(status) = item.get("status")
        && !matches!(status.as_str(), Some("" | "completed"))
    {
        return Err(Error("Unsupported reasoning status"));
    }
    if let Some(content) = item.get("content")
        && content.as_array().is_none_or(|parts| !parts.is_empty())
    {
        return Err(Error("Unexpected plaintext reasoning content"));
    }
    if let Some(summary) = item.get("summary") {
        for part in summary
            .as_array()
            .ok_or(Error("Invalid reasoning summary"))?
        {
            if part.as_object().is_none_or(|object| object.len() != 2)
                || part["type"] != "summary_text"
                || !part["text"].is_string()
            {
                return Err(Error("Unsupported reasoning summary properties"));
            }
        }
    }
    Ok(())
}
