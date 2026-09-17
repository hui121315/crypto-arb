//! 交易所层错误类型。
//!
//! 业务库内部使用 [`ExchangeError`]；进入 API 层后通过 `From` 转换为
//! [`common::AppError`]，再由 axum 中间件序列化为 HTTP 响应。

use serde_json::{json, Value};
use shared_types::{problem::codes, ExchangeProblem};
use thiserror::Error;

#[derive(Debug, Clone, Error)]
pub enum ExchangeError {
    #[error("network error: {0}")]
    Network(String),

    #[error("timeout after {seconds}s")]
    Timeout { seconds: u64 },

    #[error("rate limited; retry after {retry_after_secs}s")]
    RateLimited { retry_after_secs: u64 },

    #[error("auth failed: {0}")]
    Auth(String),

    #[error("http {status}: {body}")]
    Http { status: u16, body: String },

    #[error("parse error: {0}")]
    Parse(String),

    #[error("api error: {exchange} code={code} msg={message}")]
    Api {
        exchange: String,
        code: String,
        message: String,
    },

    #[error("ws closed: {0}")]
    WsClosed(String),

    #[error("circuit breaker open: {exchange}")]
    CircuitBreaker { exchange: String },

    #[error("unsupported symbol: {0}")]
    UnsupportedSymbol(String),

    #[error("unsupported capability: {0}")]
    UnsupportedCapability(&'static str),

    #[error("not implemented: {0}")]
    NotImplemented(&'static str),
}

pub type ExchangeResult<T> = Result<T, ExchangeError>;

impl ExchangeError {
    pub fn retry_after_ms(&self) -> Option<u64> {
        match self {
            Self::RateLimited { retry_after_secs } => Some(retry_after_secs.saturating_mul(1_000)),
            _ => None,
        }
    }

    pub fn status_code(&self) -> Option<u16> {
        match self {
            Self::Http { status, .. } => Some(*status),
            _ => None,
        }
    }

    pub fn exchange_code(&self) -> Option<String> {
        match self {
            Self::Api { code, .. } => Some(code.clone()),
            _ => None,
        }
    }

    pub fn to_problem(&self, venue: &str, operation: &str) -> ExchangeProblem {
        ExchangeProblem::new(venue, operation, self.to_string())
            .with_status(self.status_code())
            .with_exchange_code(self.exchange_code())
            .with_retry_after_ms(self.retry_after_ms())
            .with_source("exchange")
    }

    fn detail_context(&self) -> Value {
        let mut details = json!({
            "errorKind": self.kind_label(),
        });
        if let Some(status) = self.status_code() {
            details["upstreamStatus"] = json!(status);
        }
        if let Some(retry_after_ms) = self.retry_after_ms() {
            details["retryAfterMs"] = json!(retry_after_ms);
        }
        if let Some(code) = self.exchange_code() {
            details["venueCode"] = json!(code);
        }
        self.extend_variant_details(&mut details);
        details
    }

    const fn kind_label(&self) -> &'static str {
        match self {
            Self::Network(_) => "network",
            Self::Timeout { .. } => "timeout",
            Self::RateLimited { .. } => "rate_limited",
            Self::Auth(_) => "auth",
            Self::Http { .. } => "http",
            Self::Parse(_) => "parse",
            Self::Api { .. } => "api",
            Self::WsClosed(_) => "ws_closed",
            Self::CircuitBreaker { .. } => "circuit_breaker",
            Self::UnsupportedSymbol(_) => "unsupported_symbol",
            Self::UnsupportedCapability(_) => "unsupported_capability",
            Self::NotImplemented(_) => "not_implemented",
        }
    }

    fn extend_variant_details(&self, details: &mut Value) {
        match self {
            Self::Api { exchange, .. } | Self::CircuitBreaker { exchange } => {
                details["exchange"] = json!(exchange);
            }
            Self::UnsupportedSymbol(symbol) => {
                details["symbol"] = json!(symbol);
            }
            Self::UnsupportedCapability(capability) => {
                details["capability"] = json!(capability);
            }
            Self::NotImplemented(feature) => {
                details["feature"] = json!(feature);
            }
            Self::Timeout { seconds } => {
                details["timeoutSeconds"] = json!(seconds);
            }
            _ => {}
        }
    }
}

impl From<ExchangeError> for common::AppError {
    fn from(err: ExchangeError) -> Self {
        let details = err.detail_context();
        match err {
            ExchangeError::RateLimited { retry_after_secs } => {
                Self::RateLimited { retry_after_secs }
            }
            ExchangeError::Timeout { seconds } => Self::Timeout {
                seconds,
                context: "exchange".to_owned(),
            },
            ExchangeError::Auth(message) => Self::Unauthorized(message),
            ExchangeError::Network(message) => {
                Self::upstream(codes::UPSTREAM_NETWORK, message).with_details(details)
            }
            ExchangeError::Http { status, body } => {
                Self::upstream(codes::UPSTREAM_HTTP, format!("http {status}: {body}"))
                    .with_details(details)
            }
            ExchangeError::Parse(message) => {
                Self::upstream(codes::UPSTREAM_PARSE, message).with_details(details)
            }
            ExchangeError::Api {
                exchange,
                code,
                message,
            } => Self::upstream(
                codes::UPSTREAM_API,
                format!("{exchange} api error: code={code} msg={message}"),
            )
            .with_details(details),
            ExchangeError::WsClosed(message) => {
                Self::upstream(codes::UPSTREAM_WS_CLOSED, message).with_details(details)
            }
            ExchangeError::CircuitBreaker { exchange } => Self::upstream(
                codes::CIRCUIT_BREAKER_OPEN,
                format!("circuit breaker open: {exchange}"),
            )
            .with_details(details),
            ExchangeError::UnsupportedSymbol(symbol) => Self::upstream(
                codes::UNSUPPORTED_SYMBOL,
                format!("unsupported symbol: {symbol}"),
            )
            .with_details(details),
            ExchangeError::UnsupportedCapability(capability) => Self::upstream(
                codes::UNSUPPORTED_CAPABILITY,
                format!("unsupported capability: {capability}"),
            )
            .with_details(details),
            ExchangeError::NotImplemented(feature) => Self::upstream(
                codes::NOT_IMPLEMENTED,
                format!("not implemented: {feature}"),
            )
            .with_details(details),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::panic)]

    use super::*;

    #[test]
    fn http_maps_to_upstream_http_with_status_detail() {
        let err: common::AppError = ExchangeError::Http {
            status: 503,
            body: "venue down".into(),
        }
        .into();
        assert_eq!(err.code(), "UPSTREAM_HTTP");
        assert_eq!(err.status().as_u16(), 502);
        let common::AppError::Domain {
            details: Some(details),
            ..
        } = err
        else {
            panic!("expected domain error");
        };
        assert_eq!(details["upstreamStatus"], 503);
        assert_eq!(details["errorKind"], "http");
    }

    #[test]
    fn api_error_preserves_exchange_and_venue_code() {
        let err: common::AppError = ExchangeError::Api {
            exchange: "gate".into(),
            code: "30001".into(),
            message: "bad symbol".into(),
        }
        .into();
        assert_eq!(err.code(), "UPSTREAM_API");
        let common::AppError::Domain {
            details: Some(details),
            ..
        } = err
        else {
            panic!("expected domain error");
        };
        assert_eq!(details["exchange"], "gate");
        assert_eq!(details["venueCode"], "30001");
        assert_eq!(details["errorKind"], "api");
    }

    #[test]
    fn unsupported_symbol_maps_to_distinct_code_with_symbol() {
        let err: common::AppError = ExchangeError::UnsupportedSymbol("FOOUSDT".into()).into();
        assert_eq!(err.code(), "UNSUPPORTED_SYMBOL");
        let common::AppError::Domain {
            details: Some(details),
            ..
        } = err
        else {
            panic!("expected domain error");
        };
        assert_eq!(details["symbol"], "FOOUSDT");
        assert_eq!(details["errorKind"], "unsupported_symbol");
    }

    #[test]
    fn upstream_network_keeps_error_kind_detail() {
        let err: common::AppError = ExchangeError::Network("dns failed".into()).into();
        assert_eq!(err.code(), "UPSTREAM_NETWORK");
        let common::AppError::Domain {
            details: Some(details),
            ..
        } = err
        else {
            panic!("expected domain error");
        };
        assert_eq!(details["errorKind"], "network");
    }

    #[test]
    fn rate_limited_and_timeout_keep_dedicated_variants() {
        let rate: common::AppError = ExchangeError::RateLimited {
            retry_after_secs: 3,
        }
        .into();
        assert_eq!(rate.code(), "RATE_LIMITED");
        let timeout: common::AppError = ExchangeError::Timeout { seconds: 5 }.into();
        assert_eq!(timeout.code(), "TIMEOUT");
    }

    #[test]
    fn exchange_error_problem_keeps_runtime_context() {
        let problem = ExchangeError::Api {
            exchange: "bybit".into(),
            code: "10006".into(),
            message: "too many requests".into(),
        }
        .to_problem("bybit", "perp_tickers");

        assert_eq!(problem.venue, "bybit");
        assert_eq!(problem.operation, "perp_tickers");
        assert_eq!(problem.exchange_code.as_deref(), Some("10006"));
        assert_eq!(problem.source.as_deref(), Some("exchange"));
    }
}
