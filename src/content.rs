//! The gateway carries text and an ordered image list on each message.
use crate::proto::ImageData;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Deserialize;
use serde_json::Value;

#[derive(Default)]
pub(crate) struct Content {
    pub text: String,
    pub images: Vec<ImageData>,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Part<'a> {
    InputText {
        text: &'a str,
    },
    OutputText {
        text: &'a str,
        #[serde(default)]
        annotations: Vec<Value>,
        #[serde(default)]
        logprobs: Vec<Value>,
    },
    InputImage {
        image_url: &'a str,
        #[serde(default)]
        detail: Option<Detail>,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Detail {
    Auto,
    Low,
    High,
    Original,
}

impl Content {
    pub fn parse(value: &Value, assistant: bool) -> Result<Self, &'static str> {
        if let Some(text) = value.as_str() {
            return Ok(Self {
                text: text.into(),
                ..Self::default()
            });
        }
        let parts: Vec<Part<'_>> = Vec::deserialize(value)
            .map_err(|_| "Expected text or supported text/image parts with no unknown fields.")?;
        let mut content = Self::default();
        let mut text = Vec::new();
        for part in parts {
            match part {
                Part::InputText { text: value } => text.push(value),
                Part::OutputText {
                    text: value,
                    annotations,
                    logprobs,
                } => {
                    if !annotations.is_empty() || !logprobs.is_empty() {
                        return Err("Only empty annotation/logprob arrays are supported.");
                    }
                    text.push(value);
                }
                Part::InputImage { image_url, detail } if !assistant => {
                    // Detail is an OpenAI processing hint; the gateway has no equivalent.
                    let _ = detail;
                    content.images.push(image(image_url)?);
                }
                _ => return Err("Content part does not match the message role."),
            }
        }
        content.text = text.join("\n\n");
        Ok(content)
    }

    pub fn text_only(value: &Value) -> Result<String, &'static str> {
        let content = Self::parse(value, false)?;
        if !content.images.is_empty() {
            return Err("Images are only supported in user messages and tool results.");
        }
        Ok(content.text)
    }
}

fn image(url: &str) -> Result<ImageData, &'static str> {
    let (mime, data) = url
        .strip_prefix("data:")
        .and_then(|s| s.split_once(";base64,"))
        .ok_or("Images must be base64 data URLs; remote URLs and file IDs are unsupported.")?;
    if !matches!(
        mime,
        "image/png" | "image/jpeg" | "image/webp" | "image/gif"
    ) {
        return Err("Supported image types are PNG, JPEG, WebP and GIF.");
    }
    if STANDARD
        .decode(data)
        .map_err(|_| "Invalid base64 image data.")?
        .is_empty()
    {
        return Err("Image data must not be empty.");
    }
    Ok(ImageData {
        base64_data: data.into(),
        mime_type: mime.into(),
        caption: String::new(),
    })
}
