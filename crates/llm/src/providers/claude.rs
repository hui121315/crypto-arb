//! Anthropic Claude provider（Messages API v1）。
//!
//! 参考：<https://docs.anthropic.com/en/api/messages>

use crate::error::{response_request_id, retry_after_secs, LlmError, LlmResult};
use crate::evidence::{ProviderApiEvidence, CLAUDE_EVIDENCE};
use crate::provider::LlmProvider;
use crate::types::{ChatRequest, ChatResponse, Message, Role, Usage};
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

const ANTHROPIC_VERSION: &str = "2023-06-01";
const BASE_TIMEOUT_SECS: u64 = 60;
const EXTRA_TIMEOUT_PER_1024_TOKENS_SECS: u64 = 15;
const MAX_TIMEOUT_SECS: u64 = 180;

#[derive(Debug, Clone)]
pub struct ClaudeProvider {
    pub api_key: String,
    pub base_url: String,
    pub default_model: String,
    evidence: &'static ProviderApiEvidence,
    client: Client,
}

impl ClaudeProvider {
    pub fn new(api_key: impl Into<String>) -> LlmResult<Self> {
        Self::with_options(api_key, None, CLAUDE_EVIDENCE.default_model)
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
                provider: CLAUDE_EVIDENCE.provider.to_owned(),
                path: CLAUDE_EVIDENCE.endpoint_path.to_owned(),
                message: format!("client build: {e}"),
            })?;
        Ok(Self {
            api_key: api_key.into(),
            base_url: base_url.unwrap_or_else(|| CLAUDE_EVIDENCE.base_url.to_owned()),
            default_model: default_model.into(),
            evidence: &CLAUDE_EVIDENCE,
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
    max_tokens: u32,
    temperature: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    messages: Vec<ApiMessage>,
}

#[derive(Deserialize)]
struct ApiResponse {
    #[serde(default)]
    model: String,
    #[serde(default)]
    content: Vec<ContentBlock>,
    #[serde(default)]
    usage: ApiUsage,
}

#[derive(Deserialize)]
struct ContentBlock {
    #[serde(default, rename = "type")]
    block_type: String,
    #[serde(default)]
    text: String,
}

#[derive(Deserialize, Default)]
struct ApiUsage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
}

fn map_message(m: &Message) -> ApiMessage {
    // Claude 的 messages 数组只支持 user / assistant；system 走顶层 system 字段
    let role = match m.role {
        Role::Assistant => "assistant",
        Role::User | Role::System => "user",
    };
    ApiMessage {
        role: role.into(),
        content: m.content.clone(),
    }
}

#[async_trait]
impl LlmProvider for ClaudeProvider {
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
        let messages: Vec<ApiMessage> = rest.iter().map(map_message).collect();
        let body = ApiRequest {
            model: model.clone(),
            max_tokens: req.max_tokens,
            temperature: req.temperature,
            system,
            messages,
        };
        let url = self.evidence.endpoint_url(&self.base_url, &model);
        let timeout = timeout_for_tokens(req.max_tokens);
        let resp = self
            .client
            .post(&url)
            .timeout(timeout)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
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
            .content
            .iter()
            .filter(|b| b.block_type == "text")
            .map(|b| b.text.clone())
            .collect::<Vec<_>>()
            .join("");
        let total = parsed.usage.input_tokens + parsed.usage.output_tokens;
        Ok(ChatResponse {
            content,
            model: if parsed.model.is_empty() {
                model
            } else {
                parsed.model
            },
            provider: self.name().to_owned(),
            usage: Usage {
                prompt_tokens: parsed.usage.input_tokens,
                completion_tokens: parsed.usage.output_tokens,
                total_tokens: total,
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
        let p = ClaudeProvider::new("test-key").unwrap();
        assert_eq!(p.name(), "claude");
        assert_eq!(p.default_model(), "claude-3-5-sonnet-latest");
    }

    #[test]
    fn system_role_mapped_to_user_in_messages() {
        let m = Message::system("S");
        let api = map_message(&m);
        assert_eq!(api.role, "user");
    }

    #[test]
    fn assistant_role_preserved() {
        let m = Message::assistant("A");
        let api = map_message(&m);
        assert_eq!(api.role, "assistant");
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
