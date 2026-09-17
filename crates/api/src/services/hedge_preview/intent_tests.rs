use super::*;
use shared_types::{HedgeExecutionParams, HedgePreviewRequest};

fn preview_request(capital_usd: f64, leverage: f64) -> HedgePreviewRequest {
    HedgePreviewRequest {
        opportunity_id: "opp-1".to_owned(),
        opportunity_snapshot_id: None,
        capital_usd,
        leverage,
        long_price: None,
        short_price: None,
        long_notional_usd: None,
        short_notional_usd: None,
        execution_params: None,
    }
}

#[test]
fn require_positive_finite_accepts_positive_value() {
    assert!(require_positive_finite(1.0, "field").is_ok());
}

#[test]
fn require_positive_finite_rejects_zero_and_negative() {
    for value in [0.0, -1.0] {
        assert!(
            require_positive_finite(value, "field").is_err(),
            "value {value} must be rejected"
        );
    }
}

#[test]
fn require_positive_finite_rejects_non_finite() {
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(
            require_positive_finite(value, "field").is_err(),
            "non-finite value {value} must be rejected"
        );
    }
}

#[test]
fn validate_preview_rejects_non_finite_capital_and_leverage() {
    let mut nan_capital = preview_request(f64::NAN, 1.0);
    assert!(validate_preview("opp-1", &mut nan_capital).is_err());

    let mut inf_leverage = preview_request(750.0, f64::INFINITY);
    assert!(validate_preview("opp-1", &mut inf_leverage).is_err());
}

#[test]
fn validate_preview_accepts_finite_positive_request() {
    let mut request = preview_request(750.0, 2.0);
    assert!(validate_preview("opp-1", &mut request).is_ok());
}

#[test]
fn pr_ak_validate_preview_rejects_path_body_opportunity_mismatch() -> Result<(), String> {
    let mut request = preview_request(750.0, 2.0);
    request.opportunity_id = "opp-other".to_owned();

    let error = validate_preview("opp-1", &mut request)
        .err()
        .ok_or_else(|| "mismatched opportunity identity must fail closed".to_owned())?;

    assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    assert_eq!(error.code(), codes::HEDGE_PREVIEW_OPPORTUNITY_MISMATCH);
    Ok(())
}

#[test]
fn validate_execution_params_rejects_non_finite_capital_and_leverage() {
    let nan_capital = HedgeExecutionParams {
        capital_usd: f64::NAN,
        ..Default::default()
    };
    assert!(validate_execution_params(&nan_capital).is_err());

    let inf_leverage = HedgeExecutionParams {
        leverage: f64::INFINITY,
        ..Default::default()
    };
    assert!(validate_execution_params(&inf_leverage).is_err());
}

#[test]
fn prefunded_spot_legs_are_always_unlevered() {
    assert_eq!(leverage_for_product(FeeProduct::Spot, 5.0), 1.0);
    assert_eq!(leverage_for_product(FeeProduct::Perp, 5.0), 5.0);
}
