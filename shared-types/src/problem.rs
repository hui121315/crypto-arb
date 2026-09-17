//! API problem contract shared by backend and frontend.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiProblemEnvelope {
    pub error: ApiProblem,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiProblem {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub status: Option<u16>,
    #[serde(default)]
    pub request_id: Option<String>,
    #[serde(default)]
    pub retry_after_ms: Option<u64>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery_action: Option<ApiRecoveryAction>,
    #[serde(default)]
    pub details: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiRecoveryAction {
    ReviewRequest,
    Authenticate,
    CheckPermissions,
    Retry,
    RetryAfterDelay,
    RefreshState,
    CheckRuntimeHealth,
    ContactOperator,
    ManualReview,
}

impl ApiRecoveryAction {
    #[must_use]
    pub const fn for_http_status(status: u16) -> Option<Self> {
        match status {
            400 | 405 | 406 | 411 | 413 | 415 | 422 => Some(Self::ReviewRequest),
            401 => Some(Self::Authenticate),
            403 => Some(Self::CheckPermissions),
            404 | 409 | 410 | 412 => Some(Self::RefreshState),
            408 => Some(Self::Retry),
            425 | 429 => Some(Self::RetryAfterDelay),
            500 => Some(Self::ContactOperator),
            502..=504 => Some(Self::CheckRuntimeHealth),
            400..=599 => Some(Self::ManualReview),
            _ => None,
        }
    }

    fn from_legacy(value: &str) -> Option<Self> {
        match value {
            "review_request" | "check_request" => Some(Self::ReviewRequest),
            "authenticate" | "provide_valid_bearer_token" => Some(Self::Authenticate),
            "check_permissions" => Some(Self::CheckPermissions),
            "retry" => Some(Self::Retry),
            "retry_after_delay" => Some(Self::RetryAfterDelay),
            "refresh_state" => Some(Self::RefreshState),
            "check_runtime_health" => Some(Self::CheckRuntimeHealth),
            "contact_operator" => Some(Self::ContactOperator),
            "manual_review" => Some(Self::ManualReview),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExchangeProblem {
    pub venue: String,
    pub operation: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exchange_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc_url: Option<String>,
}

impl ApiProblem {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            status: None,
            request_id: None,
            retry_after_ms: None,
            source: None,
            recovery_action: None,
            details: None,
        }
    }

    pub fn with_status(mut self, status: u16) -> Self {
        self.status = Some(status);
        self
    }

    pub fn with_request_id(mut self, request_id: Option<String>) -> Self {
        self.request_id = request_id;
        self
    }

    pub fn with_retry_after_ms(mut self, retry_after_ms: Option<u64>) -> Self {
        self.retry_after_ms = retry_after_ms;
        self
    }

    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    pub fn with_recovery_action(mut self, recovery_action: ApiRecoveryAction) -> Self {
        self.recovery_action = Some(recovery_action);
        self
    }

    #[must_use]
    pub fn effective_recovery_action(&self) -> Option<ApiRecoveryAction> {
        self.recovery_action.or_else(|| {
            self.details
                .as_ref()
                .and_then(|details| details.get("recoveryAction"))
                .and_then(serde_json::Value::as_str)
                .and_then(ApiRecoveryAction::from_legacy)
        })
    }
}

impl ExchangeProblem {
    pub fn new(
        venue: impl Into<String>,
        operation: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            venue: venue.into(),
            operation: operation.into(),
            message: message.into(),
            method: None,
            path: None,
            symbol: None,
            status: None,
            exchange_code: None,
            retry_after_ms: None,
            latency_ms: None,
            source: None,
            request_id: None,
            doc_url: None,
        }
    }

    pub fn with_method(mut self, method: impl Into<String>) -> Self {
        self.method = Some(method.into());
        self
    }

    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }

    pub fn with_symbol(mut self, symbol: impl Into<String>) -> Self {
        self.symbol = Some(symbol.into());
        self
    }

    pub fn with_status(mut self, status: Option<u16>) -> Self {
        self.status = status;
        self
    }

    pub fn with_exchange_code(mut self, code: Option<String>) -> Self {
        self.exchange_code = code;
        self
    }

    pub fn with_retry_after_ms(mut self, retry_after_ms: Option<u64>) -> Self {
        self.retry_after_ms = retry_after_ms;
        self
    }

    pub fn with_latency_ms(mut self, latency_ms: Option<u64>) -> Self {
        self.latency_ms = latency_ms;
        self
    }

    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    pub fn with_request_id(mut self, request_id: Option<String>) -> Self {
        self.request_id = request_id;
        self
    }

    pub fn with_doc_url(mut self, doc_url: Option<String>) -> Self {
        self.doc_url = doc_url;
        self
    }

    pub fn to_api_problem(&self, code: impl Into<String>) -> ApiProblem {
        let mut problem = ApiProblem::new(code, self.message.clone())
            .with_retry_after_ms(self.retry_after_ms)
            .with_source(self.source.clone().unwrap_or_else(|| self.venue.clone()))
            .with_recovery_action(
                self.status
                    .and_then(ApiRecoveryAction::for_http_status)
                    .unwrap_or(ApiRecoveryAction::CheckRuntimeHealth),
            );
        if let Some(status) = self.status {
            problem = problem.with_status(status);
        }
        problem.request_id = self.request_id.clone();
        problem.details = Some(exchange_problem_details(self));
        problem
    }
}

fn exchange_problem_details(problem: &ExchangeProblem) -> serde_json::Value {
    serde_json::to_value(problem).unwrap_or_else(|error| {
        serde_json::json!({
            "venue": problem.venue.as_str(),
            "operation": problem.operation.as_str(),
            "status": problem.status,
            "detailEncodeError": error.to_string(),
        })
    })
}

pub mod codes;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn problem_serializes_camel_case_retry_after() {
        let envelope = ApiProblemEnvelope {
            error: ApiProblem::new("RATE_LIMITED", "slow down")
                .with_status(429)
                .with_request_id(Some("req-1".into()))
                .with_retry_after_ms(Some(2_000))
                .with_recovery_action(ApiRecoveryAction::RetryAfterDelay),
        };

        let text = serde_json::to_string(&envelope).expect("serialize");

        assert!(text.contains("\"retryAfterMs\":2000"));
        assert!(text.contains("\"requestId\":\"req-1\""));
        assert!(text.contains("\"recoveryAction\":\"retry_after_delay\""));
    }

    #[test]
    fn problem_decodes_legacy_payload_and_derives_recovery() {
        let problem: ApiProblem = serde_json::from_value(serde_json::json!({
            "code": "UNAUTHORIZED",
            "message": "authenticate",
            "status": 401,
            "details": { "recoveryAction": "provide_valid_bearer_token" }
        }))
        .expect("decode legacy problem");

        assert_eq!(problem.recovery_action, None);
        assert_eq!(
            problem.effective_recovery_action(),
            Some(ApiRecoveryAction::Authenticate)
        );
    }

    #[test]
    fn exchange_problem_serializes_context_and_builds_api_problem() {
        let problem = ExchangeProblem::new("bybit", "perp_tickers", "rate limited")
            .with_method("GET")
            .with_path("/v5/market/tickers")
            .with_status(Some(429))
            .with_retry_after_ms(Some(2_000))
            .with_source("exchange-fanout");

        let text = serde_json::to_string(&problem).expect("serialize exchange problem");
        let api_problem = problem.to_api_problem("MARKET_DATA_RATE_LIMITED");

        assert!(text.contains("\"retryAfterMs\":2000"));
        assert!(text.contains("\"operation\":\"perp_tickers\""));
        assert_eq!(api_problem.status, Some(429));
        assert_eq!(api_problem.retry_after_ms, Some(2_000));
        assert_eq!(api_problem.source.as_deref(), Some("exchange-fanout"));
        assert_eq!(
            api_problem.recovery_action,
            Some(ApiRecoveryAction::RetryAfterDelay)
        );
        assert_eq!(
            api_problem
                .details
                .as_ref()
                .and_then(|details| details.get("venue"))
                .and_then(|value| value.as_str()),
            Some("bybit")
        );
    }
}
