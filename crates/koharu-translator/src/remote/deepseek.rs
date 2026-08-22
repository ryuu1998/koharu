use anyhow::Context;
use koharu_secrets::ExposeSecret;
use reqwest::Client;

use super::openai_compatible::{ChatBackend, ResponseMode};
use crate::{Error, GenerationConfig, Model, Provider, Result, TranslationRequest};

const URL: &str = "https://api.deepseek.com/chat/completions";
const JSON_EXAMPLE: &str = concat!(
    " Example JSON output: ",
    r#"{"translations":[{"id":0,"text":"translated text"}]}"#,
    "."
);

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(default)]
pub struct DeepSeekConfig {}

pub(super) static MODELS: &[(&str, &str)] = &[
    ("deepseek-v4-flash", "DeepSeek V4 Flash"),
    ("deepseek-v4-pro", "DeepSeek V4 Pro"),
];

pub(super) async fn models() -> Result<Vec<Model>> {
    Ok(if koharu_secrets::get("deepseek")?.is_some() {
        Model::catalog(Provider::DeepSeek, MODELS, false)
    } else {
        Vec::new()
    })
}

pub(super) async fn translate(
    client: &Client,
    _config: &DeepSeekConfig,
    model: &str,
    generation: &GenerationConfig,
    request: &TranslationRequest,
) -> Result<Vec<String>> {
    let api_key = koharu_secrets::get("deepseek")?.context("deepseek API key is not configured")?;
    let mut backend = ChatBackend::new(
        "deepseek",
        URL,
        Some(api_key.expose_secret()),
        model,
        generation,
        ResponseMode::JsonObject,
    );
    backend.temperature = generation.temperature.or(Some(1.3));
    backend.thinking = Some(if generation.thinking {
        "enabled"
    } else {
        "disabled"
    });
    backend.system_prompt_suffix = Some(JSON_EXAMPLE);
    match super::openai_compatible::translate(client, backend.clone(), request).await {
        Err(error) if retry_invalid_json(&error) => {
            tracing::warn!(model, "retrying DeepSeek translation after invalid JSON");
            if generation.temperature.is_none() {
                backend.temperature = Some(0.2);
            }
            super::openai_compatible::translate(client, backend, request).await
        }
        result => result,
    }
}

fn retry_invalid_json(error: &Error) -> bool {
    matches!(error, Error::InvalidTranslationJson { .. })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supplies_the_json_shape_required_by_deepseek_json_mode() {
        let example = JSON_EXAMPLE
            .split_once(':')
            .map(|(_, json)| json.trim().trim_end_matches('.'))
            .unwrap();
        let value: serde_json::Value = serde_json::from_str(example).unwrap();
        assert!(value["translations"].is_array());
    }

    #[test]
    fn retries_only_invalid_translation_json() {
        assert!(retry_invalid_json(&Error::InvalidTranslationJson {
            provider: "deepseek",
            source: anyhow::anyhow!("missing field `translations`"),
        }));
        assert!(!retry_invalid_json(&Error::QuotaExceeded {
            provider: "deepseek",
        }));
    }
}
