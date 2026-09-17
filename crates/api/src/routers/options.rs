//! `/api/options/*`：期权定价、Greeks、隐含波动率。

use crate::state::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use common::AppError;
use options::iv_solver::SIGMA_UPPER_BOUND;
use options::{call_price, greeks_from_days, put_price, solve_iv_from_days};
use serde_json::{json, Value};
use shared_types::{
    problem::codes, OptionGreeks, OptionGreeksRequest, OptionIvRequest, OptionIvResponse,
    OptionPriceRequest, OptionPriceResponse, OptionType,
};

const MAX_ABS_RISK_FREE_RATE: f64 = 10.0;
const MAX_DAYS_TO_EXPIRY: f64 = 36_500.0;
const MAX_PRICE_INPUT: f64 = 1_000_000_000_000.0;

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/options/positions", get(positions))
        .route("/api/options/price", post(price))
        .route("/api/options/greeks", post(greeks))
        .route("/api/options/iv", post(iv))
}

async fn positions(State(_state): State<AppState>) -> Result<(), AppError> {
    Err(positions_unsupported_error())
}

fn positions_unsupported_error() -> AppError {
    AppError::domain(
        StatusCode::NOT_IMPLEMENTED,
        codes::OPTIONS_POSITIONS_UNSUPPORTED,
        "options positions are not integrated",
    )
    .with_details(json!({
        "reason": "no_verified_option_account_source",
        "surface": "api_surface.options",
    }))
}

// ===== /price =====

async fn price(
    State(_state): State<AppState>,
    Json(req): Json<OptionPriceRequest>,
) -> Result<Json<OptionPriceResponse>, AppError> {
    validate_price_request(&req)?;
    let kind = parse_option_type(&req.option_type)?;
    let t = req.days_to_expiry / 365.0;
    let p = match kind {
        OptionType::Call => call_price(
            req.spot_price,
            req.strike,
            t,
            req.risk_free_rate,
            req.volatility,
        ),
        OptionType::Put => put_price(
            req.spot_price,
            req.strike,
            t,
            req.risk_free_rate,
            req.volatility,
        ),
    };
    Ok(Json(OptionPriceResponse { price: p }))
}

// ===== /greeks =====

async fn greeks(
    State(_state): State<AppState>,
    Json(req): Json<OptionGreeksRequest>,
) -> Result<Json<OptionGreeks>, AppError> {
    validate_greeks_request(&req)?;
    let kind = parse_option_type(&req.option_type)?;
    let g = greeks_from_days(
        req.spot_price,
        req.strike,
        req.days_to_expiry,
        req.risk_free_rate,
        req.volatility,
        kind,
    );
    Ok(Json(g))
}

// ===== /iv =====

async fn iv(
    State(_state): State<AppState>,
    Json(req): Json<OptionIvRequest>,
) -> Result<Json<OptionIvResponse>, AppError> {
    validate_iv_request(&req)?;
    let kind = parse_option_type(&req.option_type)?;
    let iv = solve_iv_from_days(
        req.market_price,
        req.spot_price,
        req.strike,
        req.days_to_expiry,
        req.risk_free_rate,
        kind,
    )
    .map_err(|error| {
        let message = error.to_string();
        iv_unsolvable(&message)
    })?;
    Ok(Json(OptionIvResponse { iv }))
}

fn parse_option_type(s: &str) -> Result<OptionType, AppError> {
    match s.to_ascii_lowercase().as_str() {
        "call" | "c" => Ok(OptionType::Call),
        "put" | "p" => Ok(OptionType::Put),
        other => {
            let fields = [json!({
                "field": "optionType",
                "reason": "expected_call_or_put",
                "value": other,
            })];
            Err(invalid_input(&fields))
        }
    }
}

fn validate_price_request(req: &OptionPriceRequest) -> Result<(), AppError> {
    validate_pricing_inputs(
        req.spot_price,
        req.strike,
        req.days_to_expiry,
        req.risk_free_rate,
        Some(req.volatility),
        None,
    )
}

fn validate_greeks_request(req: &OptionGreeksRequest) -> Result<(), AppError> {
    validate_pricing_inputs(
        req.spot_price,
        req.strike,
        req.days_to_expiry,
        req.risk_free_rate,
        Some(req.volatility),
        None,
    )
}

fn validate_iv_request(req: &OptionIvRequest) -> Result<(), AppError> {
    validate_pricing_inputs(
        req.spot_price,
        req.strike,
        req.days_to_expiry,
        req.risk_free_rate,
        None,
        Some(req.market_price),
    )
}

fn validate_pricing_inputs(
    spot_price: f64,
    strike: f64,
    days_to_expiry: f64,
    risk_free_rate: f64,
    volatility: Option<f64>,
    market_price: Option<f64>,
) -> Result<(), AppError> {
    let mut issues = Vec::new();
    validate_positive_cap("spotPrice", spot_price, MAX_PRICE_INPUT, &mut issues);
    validate_positive_cap("strike", strike, MAX_PRICE_INPUT, &mut issues);
    validate_positive_cap(
        "daysToExpiry",
        days_to_expiry,
        MAX_DAYS_TO_EXPIRY,
        &mut issues,
    );
    validate_abs_cap(
        "riskFreeRate",
        risk_free_rate,
        MAX_ABS_RISK_FREE_RATE,
        &mut issues,
    );
    if let Some(volatility) = volatility {
        validate_positive_cap("volatility", volatility, SIGMA_UPPER_BOUND, &mut issues);
    }
    if let Some(market_price) = market_price {
        validate_positive_cap("marketPrice", market_price, MAX_PRICE_INPUT, &mut issues);
    }
    if issues.is_empty() {
        Ok(())
    } else {
        Err(invalid_input(&issues))
    }
}

fn validate_positive_cap(field: &'static str, value: f64, max: f64, issues: &mut Vec<Value>) {
    if !value.is_finite() {
        issues.push(field_issue(field, "must_be_finite", value, None, Some(max)));
    } else if value <= 0.0 {
        issues.push(field_issue(
            field,
            "must_be_positive",
            value,
            Some(0.0),
            Some(max),
        ));
    } else if value > max {
        issues.push(field_issue(field, "above_cap", value, Some(0.0), Some(max)));
    }
}

fn validate_abs_cap(field: &'static str, value: f64, max_abs: f64, issues: &mut Vec<Value>) {
    if !value.is_finite() {
        issues.push(field_issue(
            field,
            "must_be_finite",
            value,
            Some(-max_abs),
            Some(max_abs),
        ));
    } else if value.abs() > max_abs {
        issues.push(field_issue(
            field,
            "absolute_value_above_cap",
            value,
            Some(-max_abs),
            Some(max_abs),
        ));
    }
}

fn field_issue(
    field: &'static str,
    reason: &'static str,
    value: f64,
    min: Option<f64>,
    max: Option<f64>,
) -> Value {
    json!({
        "field": field,
        "reason": reason,
        "value": value,
        "min": min,
        "max": max,
    })
}

fn invalid_input(fields: &[Value]) -> AppError {
    AppError::domain(
        StatusCode::BAD_REQUEST,
        codes::OPTIONS_CALCULATOR_INVALID_INPUT,
        "invalid options calculator input",
    )
    .with_details(json!({ "fields": fields }))
}

fn iv_unsolvable(message: &str) -> AppError {
    AppError::domain(
        StatusCode::BAD_REQUEST,
        codes::OPTIONS_IV_UNSOLVABLE,
        "options implied volatility could not be solved",
    )
    .with_details(json!({ "reason": message }))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::panic)]

    use super::*;

    #[test]
    fn price_validation_rejects_bad_numeric_fields() {
        let req = OptionPriceRequest {
            spot_price: -1.0,
            strike: 100.0,
            days_to_expiry: 0.0,
            risk_free_rate: 0.02,
            volatility: SIGMA_UPPER_BOUND + 1.0,
            option_type: "call".to_owned(),
        };

        let error = match validate_price_request(&req) {
            Ok(()) => panic!("invalid request should fail"),
            Err(error) => error,
        };

        assert_eq!(error.code(), codes::OPTIONS_CALCULATOR_INVALID_INPUT);
        let AppError::Domain { details, .. } = error else {
            panic!("expected domain error");
        };
        let text = details.map_or_else(String::new, |details| details.to_string());
        assert!(text.contains("spotPrice"), "details: {text}");
        assert!(text.contains("daysToExpiry"), "details: {text}");
        assert!(text.contains("volatility"), "details: {text}");
    }

    #[test]
    fn invalid_option_type_uses_typed_problem() {
        let error = match parse_option_type("straddle") {
            Ok(kind) => panic!("invalid type should fail: {kind:?}"),
            Err(error) => error,
        };

        assert_eq!(error.code(), codes::OPTIONS_CALCULATOR_INVALID_INPUT);
    }

    #[test]
    fn positions_route_returns_not_integrated_envelope() {
        let error = positions_unsupported_error();

        assert_eq!(error.status(), StatusCode::NOT_IMPLEMENTED);
        assert_eq!(error.code(), codes::OPTIONS_POSITIONS_UNSUPPORTED);
        let AppError::Domain { details, .. } = error else {
            panic!("expected domain error");
        };
        let text = details.map_or_else(String::new, |details| details.to_string());
        assert!(
            text.contains("no_verified_option_account_source"),
            "details: {text}"
        );
        assert!(text.contains("api_surface.options"), "details: {text}");
    }
}
