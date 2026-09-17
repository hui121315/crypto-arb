use axum::http::StatusCode;
use common::AppError;
use exchange::ExchangeError;
use shared_types::{problem::codes, ApiProblem, ApiRecoveryAction};
use trading::TradingError;

const BINANCE_PERMISSION_DENIED_CODE: &str = "-2015";
const BINANCE_PERMISSION_DOC_URL: &str =
    "https://academy.binance.com/en/articles/understanding-the-binance-invalid-api-key-2015-error";
const BINANCE_API_SECURITY_DOC_URL: &str =
    "https://developers.binance.com/en/docs/products/derivatives-trading-usds-futures/general-info";

pub(crate) fn map_trading_error(error: TradingError) -> AppError {
    match error {
        TradingError::RiskBlocked(reasons) => AppError::domain(
            StatusCode::BAD_REQUEST,
            codes::RISK_BLOCKED,
            format!("risk blocked order: {reasons:?}"),
        )
        .with_details(serde_json::json!({ "reasons": reasons })),
        TradingError::InsufficientMargin {
            exchange,
            required,
            available,
        } => AppError::domain(
            StatusCode::BAD_REQUEST,
            codes::INSUFFICIENT_MARGIN,
            format!(
                "insufficient margin on {exchange}: required {required:.4}, available {available:.4}"
            ),
        )
        .with_details(serde_json::json!({
            "exchange": exchange,
            "required": required,
            "available": available,
        })),
        TradingError::OrderNotFound(id) => AppError::NotFound(format!("order: {id}")),
        TradingError::SubmissionInFlight(client_order_id) => AppError::domain(
            StatusCode::CONFLICT,
            "SUBMISSION_IN_FLIGHT",
            format!(
                "submission already in flight for client_order_id {client_order_id}; retry after it settles"
            ),
        )
        .with_details(serde_json::json!({ "clientOrderId": client_order_id })),
        TradingError::AuditLogUnavailable { reason } => AppError::domain(
            StatusCode::SERVICE_UNAVAILABLE,
            codes::LIVE_AUDIT_TRAIL_UNAVAILABLE,
            format!("live order refused: audit trail unavailable: {reason}"),
        )
        .with_details(serde_json::json!({ "reason": reason })),
        TradingError::Exchange(inner) => map_exchange_error(inner),
    }
}

pub(crate) fn trading_error_problem(
    error: &TradingError,
    venue: &str,
    operation: &str,
) -> ApiProblem {
    match error {
        TradingError::RiskBlocked(reasons) => {
            let mut problem = ApiProblem::new(
                codes::RISK_BLOCKED,
                format!("risk blocked order: {reasons:?}"),
            )
            .with_status(StatusCode::BAD_REQUEST.as_u16())
            .with_source(operation)
            .with_request_id(common::request_id::current());
            problem.details = Some(serde_json::json!({ "reasons": reasons }));
            problem
        }
        TradingError::InsufficientMargin {
            exchange,
            required,
            available,
        } => {
            let mut problem = ApiProblem::new(
                codes::INSUFFICIENT_MARGIN,
                format!(
                    "insufficient margin on {exchange}: required {required:.4}, available {available:.4}"
                ),
            )
            .with_status(StatusCode::BAD_REQUEST.as_u16())
            .with_source(operation)
            .with_request_id(common::request_id::current());
            problem.details = Some(serde_json::json!({
                "exchange": exchange,
                "required": required,
                "available": available,
            }));
            problem
        }
        TradingError::OrderNotFound(id) => ApiProblem::new("NOT_FOUND", format!("order: {id}"))
            .with_status(StatusCode::NOT_FOUND.as_u16())
            .with_source(operation)
            .with_request_id(common::request_id::current()),
        TradingError::SubmissionInFlight(client_order_id) => ApiProblem::new(
            "SUBMISSION_IN_FLIGHT",
            format!(
                "submission already in flight for client_order_id {client_order_id}; retry after it settles"
            ),
        )
        .with_status(StatusCode::CONFLICT.as_u16())
        .with_retry_after_ms(Some(1_000))
        .with_source(operation)
        .with_request_id(common::request_id::current()),
        TradingError::AuditLogUnavailable { reason } => {
            let mut problem = ApiProblem::new(
                codes::LIVE_AUDIT_TRAIL_UNAVAILABLE,
                format!("live order refused: audit trail unavailable: {reason}"),
            )
            .with_status(StatusCode::SERVICE_UNAVAILABLE.as_u16())
            .with_source(operation)
            .with_request_id(common::request_id::current());
            problem.details = Some(serde_json::json!({ "reason": reason }));
            problem
        }
        TradingError::Exchange(inner) => exchange_error_problem(inner, venue, operation),
    }
}

fn map_exchange_error(error: ExchangeError) -> AppError {
    if binance_permission_denied(&error) {
        return AppError::domain(
            StatusCode::FORBIDDEN,
            codes::CREDENTIAL_PERMISSION_DENIED,
            binance_permission_message(),
        )
        .with_details(binance_permission_details());
    }
    error.into()
}

fn exchange_error_problem(error: &ExchangeError, venue: &str, operation: &str) -> ApiProblem {
    if binance_permission_denied(error) {
        let mut problem = ApiProblem::new(
            codes::CREDENTIAL_PERMISSION_DENIED,
            binance_permission_message(),
        )
        .with_status(StatusCode::FORBIDDEN.as_u16())
        .with_source(operation)
        .with_request_id(common::request_id::current())
        .with_recovery_action(ApiRecoveryAction::CheckPermissions);
        problem.details = Some(binance_permission_details());
        return problem;
    }
    let exchange_problem = error
        .to_problem(venue, operation)
        .with_status(Some(exchange_problem_status(error)))
        .with_request_id(common::request_id::current());
    exchange_problem.to_api_problem(exchange_problem_code(error))
}

fn binance_permission_denied(error: &ExchangeError) -> bool {
    matches!(
        error,
        ExchangeError::Api { exchange, code, .. }
            if exchange.eq_ignore_ascii_case("binance")
                && code.trim() == BINANCE_PERMISSION_DENIED_CODE
    )
}

fn binance_permission_message() -> &'static str {
    "Binance 实盘写权限被拒绝（-2015）。请在 API Management 检查：使用主网 API Key、启用 Futures 交易权限，并将运行后端的公网 IP 加入白名单；无需开启提现权限。"
}

fn binance_permission_details() -> serde_json::Value {
    serde_json::json!({
        "venue": "binance",
        "operation": "order_write",
        "exchangeCode": BINANCE_PERMISSION_DENIED_CODE,
        "requiredPermission": "futures_trade",
        "configurationChecks": [
            "mainnet_api_key",
            "enable_futures",
            "backend_public_ip_allowlist"
        ],
        "withdrawalPermissionRequired": false,
        "docUrls": [BINANCE_PERMISSION_DOC_URL, BINANCE_API_SECURITY_DOC_URL],
    })
}

fn exchange_problem_status(error: &ExchangeError) -> u16 {
    match error {
        ExchangeError::RateLimited { .. } => StatusCode::TOO_MANY_REQUESTS.as_u16(),
        ExchangeError::Timeout { .. } => StatusCode::GATEWAY_TIMEOUT.as_u16(),
        ExchangeError::Auth(_) => StatusCode::UNAUTHORIZED.as_u16(),
        ExchangeError::Http { status, .. } => *status,
        ExchangeError::Network(_)
        | ExchangeError::Parse(_)
        | ExchangeError::Api { .. }
        | ExchangeError::WsClosed(_)
        | ExchangeError::CircuitBreaker { .. }
        | ExchangeError::UnsupportedSymbol(_)
        | ExchangeError::UnsupportedCapability(_)
        | ExchangeError::NotImplemented(_) => StatusCode::BAD_GATEWAY.as_u16(),
    }
}

fn exchange_problem_code(error: &ExchangeError) -> &'static str {
    match error {
        ExchangeError::Network(_) => codes::UPSTREAM_NETWORK,
        ExchangeError::Http { .. } => codes::UPSTREAM_HTTP,
        ExchangeError::Parse(_) => codes::UPSTREAM_PARSE,
        ExchangeError::Api { .. } => codes::UPSTREAM_API,
        ExchangeError::WsClosed(_) => codes::UPSTREAM_WS_CLOSED,
        ExchangeError::CircuitBreaker { .. } => codes::CIRCUIT_BREAKER_OPEN,
        ExchangeError::UnsupportedSymbol(_) => codes::UNSUPPORTED_SYMBOL,
        ExchangeError::UnsupportedCapability(_) => codes::UNSUPPORTED_CAPABILITY,
        ExchangeError::NotImplemented(_) => codes::NOT_IMPLEMENTED,
        ExchangeError::RateLimited { .. } => "RATE_LIMITED",
        ExchangeError::Timeout { .. } => "TIMEOUT",
        ExchangeError::Auth(_) => "UNAUTHORIZED",
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::panic)]

    use super::*;
    use axum::body::to_bytes;
    use axum::response::IntoResponse;
    use shared_types::RiskBlockReason;

    #[tokio::test]
    async fn risk_blocked_maps_to_domain_code_with_reasons() {
        let err = map_trading_error(TradingError::RiskBlocked(vec![
            RiskBlockReason::KillSwitchActive,
            RiskBlockReason::LiveTradingDisabled,
        ]));
        assert_eq!(err.status(), StatusCode::BAD_REQUEST);
        assert_eq!(err.code(), "RISK_BLOCKED");
        let text = body_string(err.into_response()).await;
        assert!(text.contains("\"code\":\"RISK_BLOCKED\""), "body: {text}");
        assert!(text.contains("kill_switch_active"), "body: {text}");
        assert!(text.contains("live_trading_disabled"), "body: {text}");
    }

    #[tokio::test]
    async fn insufficient_margin_maps_to_domain_code_with_numbers() {
        let err = map_trading_error(TradingError::InsufficientMargin {
            exchange: "binance".into(),
            required: 100.0,
            available: 40.0,
        });
        assert_eq!(err.status(), StatusCode::BAD_REQUEST);
        assert_eq!(err.code(), "INSUFFICIENT_MARGIN");
        let text = body_string(err.into_response()).await;
        assert!(
            text.contains("\"code\":\"INSUFFICIENT_MARGIN\""),
            "body: {text}"
        );
        assert!(text.contains("\"exchange\":\"binance\""), "body: {text}");
        assert!(text.contains("\"required\":100.0"), "body: {text}");
        assert!(text.contains("\"available\":40.0"), "body: {text}");
    }

    #[tokio::test]
    async fn audit_log_unavailable_maps_to_service_unavailable() {
        let err = map_trading_error(TradingError::AuditLogUnavailable {
            reason: "audit log configured but sink is not open: open_failed".into(),
        });
        assert_eq!(err.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(err.code(), "LIVE_AUDIT_TRAIL_UNAVAILABLE");
        let text = body_string(err.into_response()).await;
        assert!(
            text.contains("\"code\":\"LIVE_AUDIT_TRAIL_UNAVAILABLE\""),
            "body: {text}"
        );
        assert!(text.contains("audit trail unavailable"), "body: {text}");
        assert!(text.contains("\"reason\""), "body: {text}");
    }

    #[tokio::test]
    async fn binance_2015_maps_to_actionable_permission_denial() {
        let err = map_trading_error(TradingError::Exchange(ExchangeError::Api {
            exchange: "binance".into(),
            code: "-2015".into(),
            message: "Invalid API-key, IP, or permissions for action".into(),
        }));

        assert_eq!(err.status(), StatusCode::FORBIDDEN);
        assert_eq!(err.code(), codes::CREDENTIAL_PERMISSION_DENIED);
        let text = body_string(err.into_response()).await;
        assert!(
            text.contains("\"code\":\"CREDENTIAL_PERMISSION_DENIED\""),
            "body: {text}"
        );
        assert!(text.contains("enable_futures"), "body: {text}");
        assert!(text.contains("backend_public_ip_allowlist"), "body: {text}");
        assert!(
            text.contains("\"recoveryAction\":\"check_permissions\""),
            "body: {text}"
        );
    }

    async fn body_string(response: axum::response::Response) -> String {
        let body = match to_bytes(response.into_body(), usize::MAX).await {
            Ok(body) => body,
            Err(error) => panic!("body bytes failed: {error}"),
        };
        match String::from_utf8(body.to_vec()) {
            Ok(text) => text,
            Err(error) => panic!("utf8 body failed: {error}"),
        }
    }

    #[tokio::test]
    async fn trading_error_problem_preserves_rate_limit_request_id_retry_after() {
        let problem = common::request_id::scope("req-live-submit-429".to_owned(), async {
            trading_error_problem(
                &TradingError::Exchange(ExchangeError::RateLimited {
                    retry_after_secs: 9,
                }),
                "okx",
                "submit_order",
            )
        })
        .await;

        assert_eq!(problem.code, "RATE_LIMITED");
        assert_eq!(problem.status, Some(429));
        assert_eq!(problem.request_id.as_deref(), Some("req-live-submit-429"));
        assert_eq!(problem.retry_after_ms, Some(9_000));
        assert_eq!(problem.source.as_deref(), Some("exchange"));
        assert_eq!(
            problem
                .details
                .as_ref()
                .and_then(|details| details.get("venue"))
                .and_then(|value| value.as_str()),
            Some("okx")
        );
    }
}
