//! Small, account-checked model menu and exact Devin gateway routing.
use crate::compat::{EFFORTS, MODEL};
use serde_json::Value;
use std::{collections::HashMap, path::Path};
use tokio::process::Command;

#[cfg(test)]
#[path = "../tests/unit/models.rs"]
mod tests;

const FABLE_UID: &str = "fusion-claude-fable-5-1-high-sidekick-swe-2-medium";
const CHOICES: &[(&str, &str, &str, &str)] = &[
    (
        "gpt-6-sol",
        "gpt-6-sol",
        "devin-gpt-6-sol",
        "GPT-6 Sol (Devin)",
    ),
    ("gpt-6-astra", "gpt-6-astra", MODEL, "GPT-6 Astra (Devin)"),
    (
        "claude-opus-5.5",
        "claude-opus-5-5",
        "devin-claude-opus-5-5",
        "Claude Opus 5.5 (Devin)",
    ),
];

#[derive(Clone, Default)]
pub struct Models {
    entries: Vec<Model>,
}

#[derive(Clone)]
pub struct Model {
    pub alias: String,
    pub slug: String,
    pub label: String,
    pub context: u64,
    pub variants: HashMap<String, Variant>,
}

#[derive(Clone)]
pub struct Variant {
    pub uid: String,
    pub output: u64,
}

impl Model {
    pub fn default_effort(&self) -> &str {
        if self.variants.contains_key("medium") {
            "medium"
        } else {
            EFFORTS
                .iter()
                .find(|effort| self.variants.contains_key(**effort))
                .expect("catalog has a supported variant")
        }
    }
}

impl Models {
    pub fn parse(value: &Value) -> Result<Self, &'static str> {
        let families = value["families"]
            .as_array()
            .ok_or("Cannot read Devin model families.")?;
        let mut entries = Vec::new();
        for (family_slug, uid_prefix, alias, label) in CHOICES {
            let Some(family) = families.iter().find(|f| f["slug"] == *family_slug) else {
                continue;
            };
            let variants = family["variants"]
                .as_array()
                .ok_or("Cannot read Devin model variants.")?;
            let mut model = Model {
                alias: (*alias).into(),
                slug: (*uid_prefix).into(),
                label: (*label).into(),
                context: 272000,
                variants: HashMap::new(),
            };
            for effort in EFFORTS {
                let uid = format!("{uid_prefix}-{effort}");
                if let Some(variant) = variants.iter().find(|v| v["model_uid"] == uid) {
                    model.context = variant["max_context_tokens"]
                        .as_u64()
                        .unwrap_or(model.context);
                    model.variants.insert(
                        (*effort).into(),
                        Variant {
                            uid,
                            output: variant["max_output_tokens"].as_u64().unwrap_or(128000),
                        },
                    );
                }
            }
            if !model.variants.is_empty() {
                entries.push(model);
            }
        }
        if let Some(variant) = families
            .iter()
            .find(|family| family["slug"] == "fusion")
            .and_then(|family| family["variants"].as_array())
            .and_then(|variants| variants.iter().find(|v| v["model_uid"] == FABLE_UID))
        {
            entries.push(Model {
                alias: "devin-fable-5-1-fusion".into(),
                slug: "fable-5-1-fusion".into(),
                label: "Fable 5.1 High + SWE-2 (Devin Fusion)".into(),
                context: variant["max_context_tokens"].as_u64().unwrap_or(272000),
                variants: HashMap::from([(
                    "medium".into(),
                    Variant {
                        uid: FABLE_UID.into(),
                        output: variant["max_output_tokens"].as_u64().unwrap_or(128000),
                    },
                )]),
            });
        }
        if entries.is_empty() {
            return Err("None of the selected Devin models are available to this account.");
        }
        Ok(Self { entries })
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

    pub fn model(&self, alias_or_slug: &str) -> Option<&Model> {
        self.entries
            .iter()
            .find(|entry| entry.alias == alias_or_slug || entry.slug == alias_or_slug)
    }

    pub fn by_slug(&self, slug: &str) -> Option<&Model> {
        self.entries.iter().find(|entry| entry.slug == slug)
    }
}
