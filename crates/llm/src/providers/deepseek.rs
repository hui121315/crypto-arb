//! `DeepSeek` provider using its own documented OpenAI-compatible endpoint.

use crate::error::LlmResult;
use crate::evidence::{ProviderApiEvidence, DEEPSEEK_EVIDENCE};
use crate::provider::LlmProvider;
use crate::providers::openai::OpenAiProvider;
use crate::types::{ChatRequest, ChatResponse};
use async_trait::async_trait;

/// `DeepSeek` shares the OpenAI-compatible body format, but not OpenAI's
/// `/v1/chat/completions` path. Its evidence selects `/chat/completions`.
#[derive(Debug, Clone)]
pub struct DeepSeekProvider {
    inner: OpenAiProvider,
}

impl DeepSeekProvider {
    pub fn new(api_key: impl Into<String>) -> LlmResult<Self> {
        Self::with_options(api_key, None, DEEPSEEK_EVIDENCE.default_model)
    }

    pub fn with_options(
        api_key: impl Into<String>,
        base_url: Option<String>,
        default_model: impl Into<String>,
    ) -> LlmResult<Self> {
        let inner =
            OpenAiProvider::with_evidence(api_key, base_url, default_model, &DEEPSEEK_EVIDENCE)?;
        Ok(Self { inner })
    }
}

#[async_trait]
impl LlmProvider for DeepSeekProvider {
    fn name(&self) -> &'static str {
        DEEPSEEK_EVIDENCE.provider
    }

    fn evidence(&self) -> &'static ProviderApiEvidence {
        &DEEPSEEK_EVIDENCE
    }

    fn default_model(&self) -> &str {
        self.inner.default_model()
    }

    async fn chat(&self, req: ChatRequest) -> LlmResult<ChatResponse> {
        self.inner.chat(req).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn provider_uses_deepseek_default_model() {
        let p = DeepSeekProvider::new("test").unwrap();
        assert_eq!(p.name(), "deepseek");
        assert_eq!(p.default_model(), "deepseek-v4-flash");
        assert_eq!(p.evidence().endpoint_path, "/chat/completions");
    }
}
