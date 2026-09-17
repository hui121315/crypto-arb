use common::AppError;
use shared_types::RuntimeProblem;

const KNOWN_VENUES: &[&str] = &[
    "hyperliquid:xyz",
    "hyperliquid:cash",
    "hyperliquid:flx",
    "hyperliquid:km",
    "hyperliquid:vntl",
    "hyperliquid:core",
    "hyperliquid",
    "binance",
    "bitget",
    "bybit",
    "gate_crossex",
    "gate",
    "kraken",
    "kucoin",
    "okx",
];

pub(crate) fn from_app_error(
    scope: &str,
    operation: &str,
    error: &AppError,
    venue_hints: &[String],
) -> RuntimeProblem {
    let message = error.to_string();
    RuntimeProblem {
        scope: scope.to_owned(),
        operation: operation.to_owned(),
        code: error.code().to_owned(),
        venue: venue_from_error(error).or_else(|| venue_from_message(&message, venue_hints)),
        retry_after_ms: retry_after_ms(error),
        message,
        problem: Some(error.to_api_problem()),
        observed_at_ms: common::time::now_ms(),
    }
}

fn venue_from_error(error: &AppError) -> Option<String> {
    match error {
        AppError::Upstream { exchange, .. } if exchange != "exchange" => Some(exchange.clone()),
        AppError::Domain {
            details: Some(details),
            ..
        } => details
            .get("exchange")
            .and_then(|value| value.as_str())
            .map(|venue| venue.to_owned()),
        _ => None,
    }
}

fn venue_from_message(message: &str, venue_hints: &[String]) -> Option<String> {
    let lower = message.to_ascii_lowercase();
    venue_hints
        .iter()
        .find(|venue| lower.contains(&venue.to_ascii_lowercase()))
        .cloned()
        .or_else(|| known_venue_from_message(&lower))
}

fn known_venue_from_message(lower: &str) -> Option<String> {
    KNOWN_VENUES
        .iter()
        .find(|venue| lower.contains(**venue))
        .map(|venue| (*venue).to_owned())
}

fn retry_after_ms(error: &AppError) -> Option<u64> {
    match error {
        AppError::RateLimited { retry_after_secs } => Some(retry_after_secs.saturating_mul(1_000)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upstream_error_keeps_specific_exchange() {
        let error = AppError::Upstream {
            exchange: "gate".into(),
            message: "json parse".into(),
        };

        let problem = from_app_error("portfolio", "positions", &error, &[]);

        assert_eq!(problem.venue.as_deref(), Some("gate"));
        assert_eq!(problem.code, "UPSTREAM");
        assert_eq!(problem.operation, "positions");
    }

    #[test]
    fn generic_upstream_error_infers_venue_from_message() {
        let error = AppError::Upstream {
            exchange: "exchange".into(),
            message: "parse error: gate positions json".into(),
        };

        let problem = from_app_error("system", "positions", &error, &[]);

        assert_eq!(problem.venue.as_deref(), Some("gate"));
    }

    #[test]
    fn generic_upstream_error_keeps_gate_crossex_family() {
        let error = AppError::Upstream {
            exchange: "exchange".into(),
            message: "gate_crossex private order stream disconnected".into(),
        };

        let problem = from_app_error("system", "private_ws_order_stream", &error, &[]);

        assert_eq!(problem.venue.as_deref(), Some("gate_crossex"));
    }

    #[test]
    fn generic_upstream_error_infers_kraken() {
        let error = AppError::Upstream {
            exchange: "exchange".into(),
            message: "kraken futures open_orders stream disconnected".into(),
        };

        let problem = from_app_error("system", "private_ws_order_stream", &error, &[]);

        assert_eq!(problem.venue.as_deref(), Some("kraken"));
    }

    #[test]
    fn rate_limit_exposes_retry_after_ms() {
        let error = AppError::RateLimited {
            retry_after_secs: 2,
        };

        let problem = from_app_error("portfolio", "balances", &error, &[]);

        assert_eq!(problem.retry_after_ms, Some(2_000));
    }

    #[test]
    fn rate_limit_retry_after_saturates() {
        let error = AppError::RateLimited {
            retry_after_secs: u64::MAX,
        };

        let problem = from_app_error("portfolio", "balances", &error, &[]);

        assert_eq!(problem.retry_after_ms, Some(u64::MAX));
    }

    #[test]
    fn domain_error_extracts_venue_from_details() {
        let error = AppError::upstream("UPSTREAM_API", "okx api error")
            .with_details(serde_json::json!({ "exchange": "okx", "venueCode": "1001" }));

        let problem = from_app_error("portfolio", "positions", &error, &[]);

        assert_eq!(problem.venue.as_deref(), Some("okx"));
        assert_eq!(problem.code, "UPSTREAM_API");
    }
}
