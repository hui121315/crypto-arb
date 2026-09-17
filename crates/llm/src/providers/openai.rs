//! OpenAI-compatible provider driven by endpoint evidence.

use crate::error::{response_request_id, retry_after_secs, LlmError, LlmResult};
use crate::evidence::{ProviderApiEvidence, OPENAI_EVIDENCE};
use crate::provider::LlmProvider;
use crate::types::{ChatRequest, ChatResponse, Role, Usage};
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

const BASE_TIMEOUT_SECS: u64 = 60;
const EXTRA_TIMEOUT_PER_1024_TOKENS_SECS: u64 = 15;
const MAX_TIMEOUT_SECS: u64 = 180;

#[derive(Debug, Clone)]
pub struct OpenAiProvider {
    pub api_key: String,
    pub base_url: String,
    pub default_model: String,
    evidence: &'static ProviderApiEvidence,
    client: Client,
}

impl OpenAiProvider {
    pub fn new(api_key: impl Into<String>) -> LlmResult<Self> {
        Self::with_options(api_key, None, OPENAI_EVIDENCE.default_model)
    }

    pub fn with_options(
        api_key: impl Into<String>,
        base_url: Option<String>,
        default_model: impl Into<String>,
    ) -> LlmResult<Self> {
        Self::with_evidence(api_key, base_url, default_model, &OPENAI_EVIDENCE)
    }

    pub(crate) fn with_evidence(
        api_key: impl Into<String>,
        base_url: Option<String>,
        default_model: impl Into<String>,
        evidence: &'static ProviderApiEvidence,
    ) -> LlmResult<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(MAX_TIMEOUT_SECS))
            .build()
            .map_err(|e| LlmError::Network {
                provider: evidence.provider.to_owned(),
                path: evidence.endpoint_path.to_owned(),
                message: format!("client build: {e}"),
            })?;
        Ok(Self {
            api_key: api_key.into(),
            base_url: base_url.unwrap_or_else(|| evidence.base_url.to_owned()),
            default_model: default_model.into(),
            evidence,
            client,
        })
    }
}

#[derive(Serialize)]
struct ApiMessage {
    role: String,
    content: String,
}

#[derive(Serialize)]
struct ApiRequest {
    model: String,
    messages: Vec<ApiMessage>,
    temperature: f64,
    max_tokens: u32,
}

#[derive(Deserialize)]
struct ApiResponse {
    #[serde(default)]
    model: String,
    choices: Vec<Choice>,
    #[serde(default)]
    usage: ApiUsage,
}

#[derive(Deserialize)]
struct Choice {
    message: ChoiceMessage,
}

#[derive(Deserialize)]
struct ChoiceMessage {
    #[serde(default)]
    content: String,
}

#[derive(Deserialize, Default)]
struct ApiUsage {
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
    #[serde(default)]
    total_tokens: u32,
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    fn name(&self) -> &'static str {
        self.evidence.provider
    }

    fn evidence(&self) -> &'static ProviderApiEvidence {
        self.evidence
    }

    fn default_model(&self) -> &str {
        &self.default_model
    }

    async fn chat(&self, req: ChatRequest) -> LlmResult<ChatResponse> {
        let model = req
            .model
            .clone()
            .unwrap_or_else(|| self.default_model.clone());
        let body = ApiRequest {
            model: model.clone(),
            messages: req
                .messages
                .iter()
                .map(|m| ApiMessage {
                    role: role_str(m.role).into(),
                    content: m.content.clone(),
                })
                .collect(),
            temperature: req.temperature,
            max_tokens: req.max_tokens,
        };
        let url = self.evidence.endpoint_url(&self.base_url, &model);
        let timeout = timeout_for_tokens(req.max_tokens);
        let resp = self
            .client
            .post(&url)
            .timeout(timeout)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| map_err(&e, timeout, self.evidence))?;
        let status = resp.status();
        let request_id = response_request_id(resp.headers());
        if is_auth_status(status) {
            return Err(LlmError::Auth {
                provider: self.name().to_owned(),
                path: self.evidence.endpoint_path.to_owned(),
                status: status.as_u16(),
                request_id,
            });
        }
        if status.as_u16() == 429 {
            return Err(LlmError::RateLimited {
                provider: self.name().to_owned(),
                path: self.evidence.endpoint_path.to_owned(),
                retry_after_secs: retry_after_secs(resp.headers()),
                request_id,
            });
        }
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(LlmError::Upstream {
                provider: self.name().to_owned(),
                path: self.evidence.endpoint_path.to_owned(),
                status: status.as_u16(),
                request_id,
                message: format!("HTTP {status}: {}", truncate(&body, 200)),
            });
        }
        let parsed: ApiResponse = resp.json().await.map_err(|e| LlmError::InvalidResponse {
            provider: self.name().to_owned(),
            path: self.evidence.endpoint_path.to_owned(),
            request_id,
            message: format!("json: {e}"),
        })?;
        let content = parsed
            .choices
            .first()
            .map(|c| c.message.content.clone())
            .unwrap_or_default();
        Ok(ChatResponse {
            content,
            model: if parsed.model.is_empty() {
                model
            } else {
                parsed.model
            },
            provider: self.name().to_owned(),
            usage: Usage {
                prompt_tokens: parsed.usage.prompt_tokens,
                completion_tokens: parsed.usage.completion_tokens,
                total_tokens: parsed.usage.total_tokens,
            },
        })
    }
}

fn role_str(r: Role) -> &'static str {
    r.as_str()
}

fn timeout_for_tokens(max_tokens: u32) -> Duration {
    let extra = u64::from(max_tokens / 1024) * EXTRA_TIMEOUT_PER_1024_TOKENS_SECS;
    Duration::from_secs((BASE_TIMEOUT_SECS + extra).min(MAX_TIMEOUT_SECS))
}

fn is_auth_status(status: reqwest::StatusCode) -> bool {
    matches!(
        status,
        reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN
    )
}

fn map_err(e: &reqwest::Error, timeout: Duration, evidence: &ProviderApiEvidence) -> LlmError {
    if e.is_timeout() {
        LlmError::Timeout {
            provider: evidence.provider.to_owned(),
            path: evidence.endpoint_path.to_owned(),
            seconds: timeout.as_secs(),
        }
    } else {
        LlmError::Network {
            provider: evidence.provider.to_owned(),
            path: evidence.endpoint_path.to_owned(),
            message: e.to_string(),
        }
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_owned()
    } else {
        s.chars().take(max).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn role_mapping() {
        assert_eq!(role_str(Role::System), "system");
        assert_eq!(role_str(Role::User), "user");
        assert_eq!(role_str(Role::Assistant), "assistant");
    }

    #[test]
    fn provider_name_and_default_model() {
        let p = OpenAiProvider::new("test-key").unwrap();
        assert_eq!(p.name(), "openai");
        assert_eq!(p.default_model(), "gpt-4o-mini");
        assert_eq!(p.evidence().endpoint_path, "/v1/chat/completions");
    }

    #[test]
    fn timeout_scales_with_max_tokens() {
        assert_eq!(timeout_for_tokens(1024), Duration::from_secs(75));
        assert_eq!(timeout_for_tokens(16_384), Duration::from_secs(180));
    }

    #[test]
    fn forbidden_maps_to_auth() {
        assert!(is_auth_status(reqwest::StatusCode::FORBIDDEN));
    }
}
