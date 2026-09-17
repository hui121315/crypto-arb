//! 顶层应用错误类型。
//!
//! 业务库使用 `thiserror` 派生；HTTP 响应适配由可选 `http` feature 提供。

use http::StatusCode;
#[cfg(feature = "http")]
use shared_types::{ApiProblem, ApiRecoveryAction};
use thiserror::Error;

#[cfg(feature = "http")]
mod response;

pub type AppResult<T> = Result<T, AppError>;

/// 对外脱敏后返回给客户端的内部错误文案，真实错误仅落到服务端日志。
#[cfg(feature = "http")]
const REDACTED_INTERNAL_MESSAGE: &str = "internal server error";

#[cfg(feature = "http")]
const AUTHENTICATION_REQUIRED_MESSAGE: &str =
    "authentication required; provide a valid Bearer token and retry";

/// 应用层错误。所有业务/系统错误最终归一到此枚举，便于统一处理。
#[derive(Debug, Error)]
pub enum AppError {
    #[error("invalid request: {0}")]
    BadRequest(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("unauthorized: {0}")]
    Unauthorized(String),

    #[error("rate limited: retry after {retry_after_secs}s")]
    RateLimited { retry_after_secs: u64 },

    #[error("upstream {exchange} error: {message}")]
    Upstream { exchange: String, message: String },

    #[error("timeout after {seconds}s ({context})")]
    Timeout { seconds: u64, context: String },

    #[error("config error: {0}")]
    Config(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error(transparent)]
    Other(#[from] anyhow::Error),

    #[error("{message}")]
    Domain {
        status: StatusCode,
        code: &'static str,
        message: String,
        details: Option<serde_json::Value>,
    },
}

impl AppError {
    pub fn status(&self) -> StatusCode {
        match self {
            AppError::BadRequest(_) => StatusCode::BAD_REQUEST,
            AppError::NotFound(_) => StatusCode::NOT_FOUND,
            AppError::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            AppError::RateLimited { .. } => StatusCode::TOO_MANY_REQUESTS,
            AppError::Upstream { .. } => StatusCode::BAD_GATEWAY,
            AppError::Timeout { .. } => StatusCode::GATEWAY_TIMEOUT,
            AppError::Config(_) => StatusCode::INTERNAL_SERVER_ERROR,
            AppError::Io(_) | AppError::Json(_) | AppError::Other(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
            AppError::Domain { status, .. } => *status,
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            AppError::BadRequest(_) => "BAD_REQUEST",
            AppError::NotFound(_) => "NOT_FOUND",
            AppError::Unauthorized(_) => "UNAUTHORIZED",
            AppError::RateLimited { .. } => "RATE_LIMITED",
            AppError::Upstream { .. } => "UPSTREAM",
            AppError::Timeout { .. } => "TIMEOUT",
            AppError::Config(_) => "CONFIG",
            AppError::Io(_) => "IO",
            AppError::Json(_) => "JSON",
            AppError::Other(_) => "INTERNAL",
            AppError::Domain { code, .. } => code,
        }
    }

    pub fn retry_after_ms(&self) -> Option<u64> {
        match self {
            AppError::RateLimited { retry_after_secs } => {
                Some(retry_after_secs.saturating_mul(1_000))
            }
            _ => None,
        }
    }

    pub fn domain(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        AppError::Domain {
            status,
            code,
            message: message.into(),
            details: None,
        }
    }

    pub fn upstream(code: &'static str, message: impl Into<String>) -> Self {
        AppError::domain(StatusCode::BAD_GATEWAY, code, message)
    }

    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        if let AppError::Domain { details: slot, .. } = &mut self {
            *slot = Some(details);
        }
        self
    }

    /// 内部错误（IO/JSON/anyhow/config）对外脱敏：这些 5xx 错误的原始文案可能
    /// 包含文件路径、序列化细节或 anyhow 错误链，不应直接回流给客户端。
    #[cfg(feature = "http")]
    fn is_internal_redacted(&self) -> bool {
        matches!(
            self,
            AppError::Io(_) | AppError::Json(_) | AppError::Other(_) | AppError::Config(_)
        )
    }

    /// 返回给客户端的错误文案；内部错误统一脱敏，其余沿用 `Display`。
    #[cfg(feature = "http")]
    fn client_message(&self) -> String {
        match self {
            AppError::Unauthorized(_) => AUTHENTICATION_REQUIRED_MESSAGE.to_owned(),
            error if error.is_internal_redacted() => REDACTED_INTERNAL_MESSAGE.to_owned(),
            error => error.to_string(),
        }
    }

    #[cfg(feature = "http")]
    pub fn to_api_problem(&self) -> ApiProblem {
        let mut problem = ApiProblem::new(self.code(), self.client_message())
            .with_status(self.status().as_u16())
            .with_recovery_action(self.recovery_action());
        if let Some(retry_after_ms) = self.retry_after_ms() {
            problem = problem.with_retry_after_ms(Some(retry_after_ms));
        }
        if let AppError::Domain { details, .. } = self {
            problem.details = details.clone();
        }
        if let AppError::Upstream { exchange, .. } = self {
            problem = problem.with_source(exchange.clone());
        }
        if matches!(self, AppError::Unauthorized(_)) {
            problem = problem.with_source("api.auth");
            problem.details = Some(serde_json::json!({
                "recoveryAction": "provide_valid_bearer_token",
            }));
        }
        problem.with_request_id(crate::request_id::current())
    }

    #[cfg(feature = "http")]
    fn recovery_action(&self) -> ApiRecoveryAction {
        match self {
            AppError::BadRequest(_) => ApiRecoveryAction::ReviewRequest,
            AppError::NotFound(_) => ApiRecoveryAction::RefreshState,
            AppError::Unauthorized(_) => ApiRecoveryAction::Authenticate,
            AppError::RateLimited { .. } => ApiRecoveryAction::RetryAfterDelay,
            AppError::Upstream { .. } => ApiRecoveryAction::CheckRuntimeHealth,
            AppError::Timeout { .. } => ApiRecoveryAction::Retry,
            AppError::Config(_) | AppError::Io(_) | AppError::Json(_) | AppError::Other(_) => {
                ApiRecoveryAction::ContactOperator
            }
            AppError::Domain { status, .. } => ApiRecoveryAction::for_http_status(status.as_u16())
                .unwrap_or(ApiRecoveryAction::ManualReview),
        }
    }
}
