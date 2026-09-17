//! LLM layer errors preserve provider response evidence without retaining prompts.

use reqwest::header::HeaderMap;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum LlmError {
    #[error("provider {provider} authentication failed at {path} with HTTP {status}")]
    Auth {
        provider: String,
        path: String,
        status: u16,
        request_id: Option<String>,
    },
    #[error("provider {provider} network failure at {path}: {message}")]
    Network {
        provider: String,
        path: String,
        message: String,
    },
    #[error("provider {provider} rate limited at {path}; retry after {retry_after_secs:?}s")]
    RateLimited {
        provider: String,
        path: String,
        retry_after_secs: Option<u64>,
        request_id: Option<String>,
    },
    #[error("provider not found: {0}")]
    ProviderNotFound(String),
    #[error("provider {provider} returned an invalid response at {path}: {message}")]
    InvalidResponse {
        provider: String,
        path: String,
        request_id: Option<String>,
        message: String,
    },
    #[error("provider {provider} upstream error at {path} with HTTP {status}: {message}")]
    Upstream {
        provider: String,
        path: String,
        status: u16,
        request_id: Option<String>,
        message: String,
    },
    #[error("provider {provider} timed out at {path} after {seconds}s")]
    Timeout {
        provider: String,
        path: String,
        seconds: u64,
    },
    #[error("not implemented: {0}")]
    NotImplemented(&'static str),
}

pub type LlmResult<T> = Result<T, LlmError>;

pub fn response_request_id(headers: &HeaderMap) -> Option<String> {
    ["x-request-id", "request-id", "x-goog-request-id"]
        .iter()
        .find_map(|name| headers.get(*name))
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

pub fn retry_after_secs(headers: &HeaderMap) -> Option<u64> {
    headers
        .get("retry-after")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
}
