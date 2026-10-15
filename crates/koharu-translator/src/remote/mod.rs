mod caiyun;
mod claude;
mod deepl;
mod deepseek;
mod gemini;
mod google_cloud;
mod grok;
mod lm_studio;
mod minimax;
mod openai;
mod openai_compatible;
mod openrouter;
mod scheduler;
pub(crate) mod transport;

use anyhow::Context;
use futures::{FutureExt, future::BoxFuture, future::join_all};
use reqwest::Client;
pub(crate) use scheduler::Scheduler;
pub(super) use transport::{Disposition, http_disposition, send_json};

pub use caiyun::CaiyunConfig;
pub use claude::ClaudeConfig;
pub use deepl::DeepLConfig;
pub use deepseek::DeepSeekConfig;
pub use gemini::GeminiConfig;
pub use google_cloud::GoogleCloudConfig;
pub use grok::GrokConfig;
pub use lm_studio::LmStudioConfig;
pub use minimax::MiniMaxConfig;
pub use openai::OpenAiConfig;
pub use openai_compatible::OpenAiCompatibleConfig;
pub use openrouter::OpenRouterConfig;

use crate::{
    GenerationConfig, Model, ModelSelection, Provider, ProvidersConfig, Result, TranslationRequest,
};

pub(crate) async fn translate(
    client: &Client,
    providers: &ProvidersConfig,
    selection: &ModelSelection,
    generation: &GenerationConfig,
    request: &TranslationRequest,
) -> Result<Vec<String>> {
    let model = || {
        selection
            .model
            .as_deref()
            .with_context(|| format!("{} requires a selected model", selection.provider))
    };
    match selection.provider {
        Provider::OpenAi => {
            openai::translate(client, &providers.openai, model()?, generation, request).await
        }
        Provider::Gemini => {
            gemini::translate(client, &providers.gemini, model()?, generation, request).await
        }
        Provider::Claude => {
            claude::translate(client, &providers.claude, model()?, generation, request).await
        }
        Provider::Grok => {
            grok::translate(client, &providers.grok, model()?, generation, request).await
        }
        Provider::MiniMax => {
            minimax::translate(client, &providers.minimax, model()?, generation, request).await
        }
        Provider::DeepSeek => {
            deepseek::translate(client, &providers.deepseek, model()?, generation, request).await
        }
        Provider::OpenAiCompatible => {
            openai_compatible::translate(
                client,
                &providers.openai_compatible,
                model()?,
                generation,
                request,
            )
            .await
        }
        Provider::OpenRouter => {
            openrouter::translate(client, &providers.openrouter, model()?, generation, request)
                .await
        }
        Provider::LmStudio => {
            lm_studio::translate(client, &providers.lm_studio, model()?, generation, request).await
        }
        Provider::DeepL => deepl::translate(client, &providers.deepl, request).await,
        Provider::GoogleCloudTranslation => {
            google_cloud::translate(client, &providers.google_cloud_translation, request).await
        }
        Provider::Caiyun => caiyun::translate(client, &providers.caiyun, request).await,
        Provider::Local => unreachable!("local translation has its own backend"),
    }
}

pub(crate) async fn models(client: &Client, providers: &ProvidersConfig) -> Vec<Model> {
    let mut models = Vec::new();
    let pending: Vec<BoxFuture<'_, Result<Vec<Model>>>> = vec![
        openai::models(client).boxed(),
        gemini::models(client).boxed(),
        claude::models(client).boxed(),
        grok::models(client).boxed(),
        minimax::models(client).boxed(),
        deepseek::models(client).boxed(),
        openai_compatible::models(client, &providers.openai_compatible).boxed(),
        openrouter::models(client).boxed(),
        lm_studio::models(client, &providers.lm_studio).boxed(),
        deepl::models().boxed(),
        google_cloud::models().boxed(),
        caiyun::models().boxed(),
    ];
    for result in join_all(pending).await {
        append_models(&mut models, result);
    }
    models.sort_by(|left, right| {
        left.provider
            .to_string()
            .cmp(&right.provider.to_string())
            .then_with(|| left.model.cmp(&right.model))
    });
    models.dedup_by(|left, right| left.provider == right.provider && left.model == right.model);
    models
}

fn append_models(models: &mut Vec<Model>, result: Result<Vec<Model>>) {
    match result {
        Ok(mut provider_models) => models.append(&mut provider_models),
        Err(error) => tracing::warn!(%error, "failed to list translation models"),
    }
}

#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize, specta::Type)]
#[serde(default)]
pub struct RemoteConfig {
    pub concurrency: u8,
}

impl Default for RemoteConfig {
    fn default() -> Self {
        Self { concurrency: 6 }
    }
}

impl RemoteConfig {
    #[must_use]
    pub fn parallelism(&self) -> usize {
        usize::from(self.concurrency.clamp(1, 16))
    }
}

#[cfg(test)]
mod config_tests {
    use super::*;
    #[test]
    fn bounds_cloud_concurrency() {
        assert_eq!(RemoteConfig::default().parallelism(), 6);
        assert_eq!(RemoteConfig { concurrency: 0 }.parallelism(), 1);
        assert_eq!(RemoteConfig { concurrency: 255 }.parallelism(), 16);
    }
}
