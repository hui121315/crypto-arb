use super::*;
use axum::http::{header, HeaderValue};
use axum::response::{IntoResponse, Response};
use axum::Json;
use shared_types::ApiProblemEnvelope;

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status();
        let retry_after_ms = self.retry_after_ms();
        if self.is_internal_redacted() {
            tracing::error!(
                code = self.code(),
                request_id = crate::request_id::current().as_deref().unwrap_or(""),
                error = %self,
                "internal error redacted from client response"
            );
        }
        let body = Json(ApiProblemEnvelope {
            error: self.to_api_problem(),
        });
        let mut response = (status, body).into_response();
        if let Some(retry_after_ms) = retry_after_ms {
            insert_retry_after_header(&mut response, retry_after_ms);
        }
        if status == StatusCode::UNAUTHORIZED {
            response.headers_mut().insert(
                header::WWW_AUTHENTICATE,
                HeaderValue::from_static("Bearer realm=\"crossline-api\""),
            );
        }
        response
    }
}

fn insert_retry_after_header(response: &mut Response, retry_after_ms: u64) {
    let retry_after_secs = retry_after_ms.saturating_add(999) / 1_000;
    if let Ok(value) = HeaderValue::from_str(&retry_after_secs.to_string()) {
        response.headers_mut().insert(header::RETRY_AFTER, value);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::panic)]

    use super::*;
    use axum::body::to_bytes;
    use pretty_assertions::assert_eq;

    #[tokio::test]
    async fn app_error_response_uses_shared_problem_contract() {
        let response = AppError::BadRequest("missing symbol".into()).into_response();
        let status = response.status();
        let text = response_body(response).await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(text.contains("\"code\":\"BAD_REQUEST\""));
        assert!(text.contains("\"status\":400"));
        assert!(text.contains("\"message\":\"invalid request: missing symbol\""));
        assert!(text.contains("\"recoveryAction\":\"review_request\""));
    }

    #[tokio::test]
    async fn rate_limit_response_carries_retry_after() {
        let response = AppError::RateLimited {
            retry_after_secs: 2,
        }
        .into_response();
        let retry_after = response
            .headers()
            .get(header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let text = response_body(response).await;

        assert_eq!(retry_after, "2");
        assert!(text.contains("\"retryAfterMs\":2000"));
        assert!(text.contains("\"recoveryAction\":\"retry_after_delay\""));
    }

    #[tokio::test]
    async fn rate_limit_retry_after_saturates_instead_of_overflowing() {
        let response = AppError::RateLimited {
            retry_after_secs: u64::MAX,
        }
        .into_response();
        let text = response_body(response).await;

        assert!(
            text.contains("\"retryAfterMs\":18446744073709551615"),
            "body: {text}"
        );
    }

    #[tokio::test]
    async fn unauthorized_response_is_stable_typed_and_recoverable() {
        let response = crate::request_id::scope("req-auth".to_owned(), async {
            AppError::Unauthorized("token mismatch".to_owned()).into_response()
        })
        .await;
        let authenticate = response
            .headers()
            .get(header::WWW_AUTHENTICATE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let text = response_body(response).await;

        assert_eq!(authenticate, "Bearer realm=\"crossline-api\"");
        assert!(text.contains("\"code\":\"UNAUTHORIZED\""), "body: {text}");
        assert!(text.contains("\"status\":401"), "body: {text}");
        assert!(text.contains("\"source\":\"api.auth\""), "body: {text}");
        assert!(text.contains("\"requestId\":\"req-auth\""), "body: {text}");
        assert!(
            text.contains("\"recoveryAction\":\"authenticate\""),
            "body: {text}"
        );
        assert!(
            text.contains("\"recoveryAction\":\"provide_valid_bearer_token\""),
            "body: {text}"
        );
        assert!(
            text.contains("provide a valid Bearer token"),
            "body: {text}"
        );
        assert!(
            !text.contains("token mismatch"),
            "auth detail leaked: {text}"
        );
    }

    #[tokio::test]
    async fn problem_body_carries_request_id_in_scope() {
        let response = crate::request_id::scope("req-42".to_owned(), async {
            AppError::NotFound("x".into()).into_response()
        })
        .await;
        let text = response_body(response).await;

        assert!(text.contains("\"requestId\":\"req-42\""), "body: {text}");
    }

    #[tokio::test]
    async fn domain_error_serializes_code_status_and_details() {
        let response = AppError::domain(StatusCode::BAD_REQUEST, "RISK_BLOCKED", "risk blocked")
            .with_details(serde_json::json!({ "reasons": ["kill_switch_active"] }))
            .into_response();
        let status = response.status();
        let text = response_body(response).await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(text.contains("\"code\":\"RISK_BLOCKED\""), "body: {text}");
        assert!(text.contains("\"status\":400"), "body: {text}");
        assert!(text.contains("kill_switch_active"), "body: {text}");
    }

    #[tokio::test]
    async fn upstream_error_uses_exchange_as_problem_source() {
        let response = AppError::Upstream {
            exchange: "okx".into(),
            message: "temporarily unavailable".into(),
        }
        .into_response();
        let status = response.status();
        let text = response_body(response).await;

        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert!(text.contains("\"code\":\"UPSTREAM\""), "body: {text}");
        assert!(text.contains("\"source\":\"okx\""), "body: {text}");
        assert!(
            text.contains("\"recoveryAction\":\"check_runtime_health\""),
            "body: {text}"
        );
    }

    #[test]
    fn domain_statuses_map_to_stable_recovery_taxonomy() {
        let cases = [
            (StatusCode::FORBIDDEN, ApiRecoveryAction::CheckPermissions),
            (StatusCode::CONFLICT, ApiRecoveryAction::RefreshState),
            (StatusCode::REQUEST_TIMEOUT, ApiRecoveryAction::Retry),
            (
                StatusCode::SERVICE_UNAVAILABLE,
                ApiRecoveryAction::CheckRuntimeHealth,
            ),
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                ApiRecoveryAction::ContactOperator,
            ),
        ];

        for (status, expected) in cases {
            let problem = AppError::domain(status, "DOMAIN_TEST", "failed").to_api_problem();
            assert_eq!(problem.recovery_action, Some(expected));
        }
    }

    #[tokio::test]
    async fn internal_io_error_is_redacted_from_body() {
        let leak = "/srv/secret/keys/api.key not found";
        let response =
            AppError::Io(std::io::Error::new(std::io::ErrorKind::NotFound, leak)).into_response();
        let status = response.status();
        let text = response_body(response).await;

        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(text.contains("\"code\":\"IO\""), "body: {text}");
        assert!(text.contains("\"status\":500"), "body: {text}");
        assert!(
            text.contains("\"message\":\"internal server error\""),
            "body: {text}"
        );
        assert!(!text.contains("api.key"), "internal path leaked: {text}");
        assert!(
            !text.contains("/srv/secret"),
            "internal path leaked: {text}"
        );
    }

    #[tokio::test]
    async fn internal_other_error_is_redacted_but_keeps_code_and_request_id() {
        let response = crate::request_id::scope("req-internal".to_owned(), async {
            AppError::Other(anyhow::anyhow!(
                "db dsn postgres://user:pw@host/db unreachable"
            ))
            .into_response()
        })
        .await;
        let status = response.status();
        let text = response_body(response).await;

        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(text.contains("\"code\":\"INTERNAL\""), "body: {text}");
        assert!(
            text.contains("\"message\":\"internal server error\""),
            "body: {text}"
        );
        assert!(
            text.contains("\"requestId\":\"req-internal\""),
            "body: {text}"
        );
        assert!(!text.contains("postgres://"), "internal dsn leaked: {text}");
        assert!(!text.contains("user:pw"), "credentials leaked: {text}");
    }

    #[tokio::test]
    async fn config_error_is_redacted_from_body() {
        let response =
            AppError::Config("missing APP_SECURITY__AUTH_TOKEN=topsecret".into()).into_response();
        let status = response.status();
        let text = response_body(response).await;

        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(text.contains("\"code\":\"CONFIG\""), "body: {text}");
        assert!(
            text.contains("\"message\":\"internal server error\""),
            "body: {text}"
        );
        assert!(!text.contains("topsecret"), "config secret leaked: {text}");
    }

    async fn response_body(response: Response) -> String {
        let body = match to_bytes(response.into_body(), usize::MAX).await {
            Ok(body) => body,
            Err(error) => panic!("body bytes failed: {error}"),
        };
        match String::from_utf8(body.to_vec()) {
            Ok(text) => text,
            Err(error) => panic!("utf8 body failed: {error}"),
        }
    }
}
