//! Account-specific Devin variants used by the Codex selector and request router.
use serde_json::Value;
use std::{collections::HashMap, path::Path};
use tokio::process::Command;

#[cfg(test)]
#[path = "../tests/unit/models.rs"]
mod tests;

#[derive(Clone, Default)]
pub struct Models {
    entries: Vec<Model>,
    by_alias: HashMap<String, String>,
}

#[derive(Clone)]
pub struct Model {
    pub alias: String,
    pub uid: String,
    pub label: String,
    pub context: u64,
    pub output: u64,
}

impl Models {
    pub fn parse(value: &Value) -> Result<Self, &'static str> {
        let families = value["families"]
            .as_array()
            .ok_or("Cannot read Devin model families.")?;
        let mut result = Self::default();
        for family in families {
            let variants = family["variants"]
                .as_array()
                .ok_or("Cannot read Devin model variants.")?;
            for variant in variants {
                let uid = variant["model_uid"]
                    .as_str()
                    .ok_or("Devin model has no UID.")?;
                if uid.is_empty()
                    || uid.len() > 200
                    || !uid
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                {
                    return Err("Devin model UID is invalid.");
                }
                let alias = format!("devin-{uid}");
                if result.by_alias.contains_key(&alias) {
                    return Err("Devin model UID is duplicated.");
                }
                let label = variant["label"].as_str().unwrap_or(uid);
                result.by_alias.insert(alias.clone(), uid.into());
                result.entries.push(Model {
                    alias,
                    uid: uid.into(),
                    label: format!("{label} (Devin)"),
                    context: variant["max_context_tokens"].as_u64().unwrap_or(272000),
                    output: variant["max_output_tokens"].as_u64().unwrap_or(128000),
                });
            }
        }
        if result.entries.is_empty() {
            return Err("Devin returned no model variants.");
        }
        Ok(result)
    }

    pub async fn load(binary: &Path) -> Result<Self, &'static str> {
        let output = tokio::time::timeout(
            std::time::Duration::from_secs(20),
            Command::new(binary)
                .args(["models", "list", "--format", "json"])
                .kill_on_drop(true)
                .output(),
        )
        .await
        .map_err(|_| "Timed out reading Devin models.")?
        .map_err(|_| "Cannot start Devin CLI. Install it on PATH or use --devin.")?;
        if !output.status.success() || output.stdout.len() > 4 * 1024 * 1024 {
            return Err("Cannot list Devin models; check your Devin login.");
        }
        let value: Value =
            serde_json::from_slice(&output.stdout).map_err(|_| "Invalid Devin model list.")?;
        Self::parse(&value)
    }

    pub fn entries(&self) -> &[Model] {
        &self.entries
    }

    pub fn uid(&self, alias: &str) -> Option<&str> {
        self.by_alias.get(alias).map(String::as_str)
    }

    pub fn contains_uid(&self, uid: &str) -> bool {
        self.by_alias.values().any(|value| value == uid)
    }

    pub fn max_output(&self, uid: &str) -> Option<u64> {
        self.entries
            .iter()
            .find(|entry| entry.uid == uid)
            .map(|entry| entry.output)
    }
}
