//! Google Gemini provider（generativelanguage.googleapis.com）。
//!
//! 参考：<https://ai.google.dev/api/generate-content>

use crate::error::{response_request_id, retry_after_secs, LlmError, LlmResult};
use crate::evidence::{ProviderApiEvidence, GEMINI_EVIDENCE};
use crate::provider::LlmProvider;
use crate::types::{ChatRequest, ChatResponse, Message, Role, Usage};
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

const BASE_TIMEOUT_SECS: u64 = 60;
const EXTRA_TIMEOUT_PER_1024_TOKENS_SECS: u64 = 15;
const MAX_TIMEOUT_SECS: u64 = 180;

#[derive(Debug, Clone)]
pub struct GeminiProvider {
    pub api_key: String,
    pub base_url: String,
    pub default_model: String,
    evidence: &'static ProviderApiEvidence,
    client: Client,
}

impl GeminiProvider {
    pub fn new(api_key: impl Into<String>) -> LlmResult<Self> {
        Self::with_options(api_key, None, GEMINI_EVIDENCE.default_model)
    }

    pub fn with_options(
        api_key: impl Into<String>,
        base_url: Option<String>,
        default_model: impl Into<String>,
    ) -> LlmResult<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(MAX_TIMEOUT_SECS))
            .build()
            .map_err(|e| LlmError::Network {
                provider: GEMINI_EVIDENCE.provider.to_owned(),
                path: GEMINI_EVIDENCE.endpoint_path.to_owned(),
                message: format!("client build: {e}"),
            })?;
        Ok(Self {
            api_key: api_key.into(),
            base_url: base_url.unwrap_or_else(|| GEMINI_EVIDENCE.base_url.to_owned()),
            default_model: default_model.into(),
            evidence: &GEMINI_EVIDENCE,
            client,
        })
    }
}

// ===== 请求 =====

#[derive(Serialize)]
struct ApiPart {
    text: String,
}

#[derive(Serialize)]
struct ApiContent {
    role: String,
    parts: Vec<ApiPart>,
}

#[derive(Serialize)]
struct GenerationConfig {
    temperature: f64,
    #[serde(rename = "maxOutputTokens")]
    max_output_tokens: u32,
}

#[derive(Serialize)]
struct ApiRequest {
    contents: Vec<ApiContent>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "systemInstruction")]
    system_instruction: Option<ApiContent>,
    #[serde(rename = "generationConfig")]
    generation_config: GenerationConfig,
}

// ===== 响应 =====

#[derive(Deserialize)]
struct ApiResponse {
    #[serde(default)]
    candidates: Vec<Candidate>,
    #[serde(default, rename = "usageMetadata")]
    usage_metadata: UsageMetadata,
    #[serde(default, rename = "modelVersion")]
    model_version: String,
}

#[derive(Deserialize)]
struct Candidate {
    #[serde(default)]
    content: Option<CandidateContent>,
}

#[derive(Deserialize)]
struct CandidateContent {
    #[serde(default)]
    parts: Vec<CandidatePart>,
}

#[derive(Deserialize)]
struct CandidatePart {
    #[serde(default)]
    text: String,
}

#[derive(Deserialize, Default)]
struct UsageMetadata {
    #[serde(default, rename = "promptTokenCount")]
    prompt_token_count: u32,
    #[serde(default, rename = "candidatesTokenCount")]
    candidates_token_count: u32,
    #[serde(default, rename = "totalTokenCount")]
    total_token_count: u32,
}

fn role_str(r: Role) -> &'static str {
    match r {
        Role::Assistant => "model",
        _ => "user",
    }
}

fn map_message(m: &Message) -> ApiContent {
    ApiContent {
        role: role_str(m.role).into(),
        parts: vec![ApiPart {
            text: m.content.clone(),
        }],
    }
}

#[async_trait]
impl LlmProvider for GeminiProvider {
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
        let (system, rest) = req.split_system();
        let contents: Vec<ApiContent> = rest.iter().map(map_message).collect();
        let system_instruction = system.map(|s| ApiContent {
            role: "system".into(),
            parts: vec![ApiPart { text: s }],
        });
        let body = ApiRequest {
            contents,
            system_instruction,
            generation_config: GenerationConfig {
                temperature: req.temperature,
                max_output_tokens: req.max_tokens,
            },
        };
        let url = self.evidence.endpoint_url(&self.base_url, &model);
        let timeout = timeout_for_tokens(req.max_tokens);
        let resp = self
            .client
            .post(&url)
            .timeout(timeout)
            .header("x-goog-api-key", &self.api_key)
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
            .candidates
            .first()
            .and_then(|c| c.content.as_ref())
            .map(|c| {
                c.parts
                    .iter()
                    .map(|p| p.text.clone())
                    .collect::<Vec<_>>()
                    .join("")
            })
            .unwrap_or_default();
        Ok(ChatResponse {
            content,
            model: if parsed.model_version.is_empty() {
                model
            } else {
                parsed.model_version
            },
            provider: self.name().to_owned(),
            usage: Usage {
                prompt_tokens: parsed.usage_metadata.prompt_token_count,
                completion_tokens: parsed.usage_metadata.candidates_token_count,
                total_tokens: parsed.usage_metadata.total_token_count,
            },
        })
    }
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
    fn provider_name_and_default_model() {
        let p = GeminiProvider::new("k").unwrap();
        assert_eq!(p.name(), "gemini");
        assert_eq!(p.default_model(), "gemini-1.5-flash");
    }

    #[test]
    fn role_assistant_mapped_to_model() {
        assert_eq!(role_str(Role::Assistant), "model");
        assert_eq!(role_str(Role::User), "user");
        assert_eq!(role_str(Role::System), "user");
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
