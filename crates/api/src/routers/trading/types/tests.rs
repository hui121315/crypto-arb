#![allow(clippy::panic)]

use super::*;
use shared_types::{ExecutionMode, MarginMode, OrderSide, OrderSource, TimeInForce};

#[test]
fn submit_order_intent_trims_ids_and_symbols() {
    let request = submit_request(OrderType::Limit, Some(42_000.0));

    let result = submit_order_intent(&request);
    assert!(result.is_ok(), "valid submit order failed: {result:?}");
    let Ok(intent) = result else { return };

    assert_eq!(intent.id, "manual-1");
    assert_eq!(intent.client_order_id, "client-1");
    assert_eq!(intent.exchange, "binance");
    assert_eq!(intent.symbol, "BTCUSDT");
    assert_eq!(intent.order_type, OrderType::Limit);
    assert_eq!(intent.slippage_tolerance_bps, Some(5.0));
    assert_eq!(
        intent
            .client_order_id_policy
            .as_ref()
            .map(|policy| policy.public_client_order_id.as_str()),
        Some("client-1")
    );
}

#[test]
fn submit_order_intent_requires_limit_price() {
    let request = submit_request(OrderType::Limit, None);

    let result = submit_order_intent(&request);
    assert!(
        result.is_err(),
        "limit price unexpectedly accepted: {result:?}"
    );
    let Err(error) = result else { return };

    assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    assert_eq!(error.code(), codes::SUBMIT_ORDER_INVALID);
    assert!(error.to_string().contains("invalid submit order request"));
}

#[test]
fn submit_order_intent_requires_client_order_id() {
    let mut request = submit_request(OrderType::Limit, Some(42_000.0));
    request.client_order_id = None;

    let result = submit_order_intent(&request);
    assert!(
        result.is_err(),
        "missing clientOrderId unexpectedly accepted: {result:?}"
    );
    let Err(error) = result else { return };

    assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    assert_eq!(error.code(), codes::SUBMIT_ORDER_INVALID);
    let text = domain_details_text(error);
    assert!(text.contains("clientOrderId"), "missing field: {text}");
}

#[test]
fn submit_order_intent_reports_all_basic_field_issues() {
    let mut request = submit_request(OrderType::Market, Some(-1.0));
    request.exchange = " ".to_owned();
    request.symbol = String::new();
    request.quantity = 0.0;
    request.options.leverage = f64::NAN;
    request.options.post_only = true;

    let result = submit_order_intent(&request);
    assert!(
        result.is_err(),
        "invalid fields unexpectedly accepted: {result:?}"
    );
    let Err(error) = result else { return };

    assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    assert_eq!(error.code(), codes::SUBMIT_ORDER_INVALID);
    assert!(
        matches!(
            error,
            AppError::Domain {
                details: Some(_),
                ..
            }
        ),
        "expected domain error with details: {error:?}"
    );
    let AppError::Domain {
        details: Some(details),
        ..
    } = error
    else {
        return;
    };
    let text = details.to_string();
    for field in [
        "exchange", "symbol", "quantity", "leverage", "price", "postOnly",
    ] {
        assert!(text.contains(field), "missing {field}: {text}");
    }
}

#[test]
fn submit_order_intent_rejects_testnet_mode() {
    let mut request = submit_request(OrderType::Limit, Some(42_000.0));
    request.mode = ExecutionMode::Testnet;

    let result = submit_order_intent(&request);
    assert!(
        result.is_err(),
        "testnet mode unexpectedly accepted: {result:?}"
    );
    let Err(error) = result else { return };

    assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    assert_eq!(error.code(), codes::SUBMIT_ORDER_INVALID);
    let text = domain_details_text(error);
    assert!(text.contains("mode"), "missing mode issue: {text}");
    assert!(
        text.contains("unsupported_public_mode"),
        "missing mode issue code: {text}"
    );
}

#[test]
fn submit_order_request_maps_missing_body_fields_to_problem() {
    let result = submit_order_request(serde_json::json!({}));

    assert!(
        result.is_err(),
        "empty body unexpectedly accepted: {result:?}"
    );
    let Err(error) = result else { return };

    assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    assert_eq!(error.code(), codes::SUBMIT_ORDER_INVALID);
    let text = domain_details_text(error);
    assert!(text.contains("body"), "missing body issue: {text}");
    assert!(
        text.contains("invalid_json_shape"),
        "missing body code: {text}"
    );
}

#[test]
fn validate_kill_switch_request_requires_reason() {
    let request = KillSwitchRequest {
        active: true,
        expected_active: Some(false),
        expected_open_order_count: Some(0),
        reason: " ".to_owned(),
    };

    let result = validate_kill_switch_request(&request, false, 0);

    assert!(matches!(result, Err(error) if error.code() == codes::KILL_SWITCH_REQUEST_INVALID));
}

#[test]
fn validate_kill_switch_request_rejects_stale_active_state() {
    let request = KillSwitchRequest {
        active: true,
        expected_active: Some(false),
        expected_open_order_count: Some(0),
        reason: "positions.kill_switch.enable".to_owned(),
    };

    let result = validate_kill_switch_request(&request, true, 0);

    let Err(error) = result else {
        panic!("stale active state unexpectedly accepted");
    };

    assert_eq!(error.status(), StatusCode::CONFLICT);
    assert_eq!(error.code(), codes::KILL_SWITCH_STALE_CONFIRMATION);
}

#[test]
fn validate_kill_switch_request_rejects_stale_open_order_count() {
    let request = KillSwitchRequest {
        active: true,
        expected_active: Some(false),
        expected_open_order_count: Some(1),
        reason: "positions.kill_switch.enable".to_owned(),
    };

    let result = validate_kill_switch_request(&request, false, 2);

    let Err(error) = result else {
        panic!("stale open order count unexpectedly accepted");
    };

    assert_eq!(error.status(), StatusCode::CONFLICT);
    assert_eq!(error.code(), codes::KILL_SWITCH_STALE_CONFIRMATION);
}

fn domain_details_text(error: AppError) -> String {
    assert!(
        matches!(
            error,
            AppError::Domain {
                details: Some(_),
                ..
            }
        ),
        "expected domain error with details: {error:?}"
    );
    match error {
        AppError::Domain {
            details: Some(details),
            ..
        } => details.to_string(),
        _ => String::new(),
    }
}

fn submit_request(order_type: OrderType, price: Option<f64>) -> SubmitOrderRequest {
    SubmitOrderRequest {
        id: Some(" manual-1 ".to_owned()),
        client_order_id: Some(" client-1 ".to_owned()),
        mode: ExecutionMode::DryRun,
        source: OrderSource::Manual,
        strategy: None,
        exchange: " binance ".to_owned(),
        symbol: " BTCUSDT ".to_owned(),
        side: OrderSide::Buy,
        order_type,
        quantity: 1.0,
        price,
        options: shared_types::SubmitOrderOptions {
            reduce_only: false,
            time_in_force: TimeInForce::Gtc,
            post_only: false,
            margin_mode: MarginMode::Cross,
            leverage: 1.0,
            slippage_tolerance_bps: Some(5.0),
        },
    }
}
