use crate::{
    compat::{EFFORTS, MODEL},
    config::random_token,
    models::Models,
};
use serde_json::{Value, json};
use std::{
    io::Write,
    path::{Path, PathBuf},
};
use tokio::process::Command;

#[cfg(test)]
#[path = "../../tests/unit/catalog.rs"]
mod tests;

pub(super) struct Catalog(PathBuf);

impl Catalog {
    fn prepare(value: &mut Value, available: &Models) -> Result<(), &'static str> {
        let models = value
            .get_mut("models")
            .and_then(Value::as_array_mut)
            .ok_or("Cannot read Codex model catalog.")?;
        let mut model = models
            .iter()
            .find(|m| m["slug"] == "gpt-6-astra")
            .cloned()
            .ok_or("Installed Codex does not provide the gpt-6-astra model catalog.")?;
        model["slug"] = json!(MODEL);
        model["display_name"] = json!("GPT-6 Astra (Devin)");
        model["description"] =
            json!("GPT-6 Astra through Devin inference; translated Codex compatibility mode.");
        model["supported_reasoning_levels"]
            .as_array_mut()
            .ok_or("Codex model catalog has no reasoning levels.")?
            .retain(|level| {
                level["effort"]
                    .as_str()
                    .is_some_and(|e| EFFORTS.contains(&e))
            });
        if model["supported_reasoning_levels"]
            .as_array()
            .unwrap()
            .is_empty()
        {
            return Err("Codex model catalog has no supported Devin reasoning levels.");
        }
        if !model["default_reasoning_level"]
            .as_str()
            .is_some_and(|e| EFFORTS.contains(&e))
        {
            model["default_reasoning_level"] =
                model["supported_reasoning_levels"][0]["effort"].clone();
        }
        model["additional_speed_tiers"] = json!([]);
        model["service_tiers"] = json!([]);
        model["input_modalities"] = json!(["text"]);
        model["supports_image_detail_original"] = json!(false);
        model["supports_search_tool"] = json!(false);
        model["upgrade"] = Value::Null;
        let template = model.clone();
        models.clear();
        if available.entries().is_empty() || available.contains_uid("gpt-6-astra-medium") {
            models.push(model);
        }
        for entry in available.entries() {
            let mut variant = template.clone();
            variant["slug"] = json!(entry.alias);
            variant["display_name"] = json!(entry.label);
            variant["description"] = json!(format!("{} via Devin inference.", entry.uid));
            variant["context_window"] = json!(entry.context);
            variant["default_reasoning_level"] = json!("medium");
            variant["supported_reasoning_levels"] = json!([{
                "effort":"medium", "description":"Reasoning is selected by the Devin model variant"
            }]);
            variant["priority"] = json!(100);
            models.push(variant);
        }
        Ok(())
    }

    pub async fn load(binary: &Path, available: &Models) -> Result<Self, &'static str> {
        let output = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            Command::new(binary)
                .args(["debug", "models", "--bundled"])
                .kill_on_drop(true)
                .output(),
        )
        .await
        .map_err(|_| "Timed out reading Codex's bundled model catalog.")?
        .map_err(|_| "Cannot start Codex. Install it on PATH or use --codex.")?;
        let mut value: Value = serde_json::from_slice(&output.stdout).map_err(
            |_| "Cannot read Codex model catalog. Codex 0.155.1 or compatible is required.",
        )?;
        if !output.status.success() {
            return Err("Cannot read Codex model catalog.");
        }
        Self::prepare(&mut value, available)?;
        let bytes = serde_json::to_vec(&value).map_err(|_| "Cannot encode model catalog.")?;
        let path = std::env::temp_dir().join(format!("devin-codex-models-{}.json", random_token()));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&path)
            .map_err(|_| "Cannot create temporary model catalog.")?;
        let catalog = Self(path);
        file.write_all(&bytes)
            .map_err(|_| "Cannot write temporary model catalog.")?;
        Ok(catalog)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Catalog {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
