use super::*;
use shared_types::{
    problem::codes, ExecutionEnvironment, ExecutionMode, HedgeConfirmStatus, MarginMode,
    OrderIntent, OrderSide, OrderSource, OrderType, OrderUpdateSource, TimeInForce,
};

#[test]
fn confirm_outcome_detail_summarizes_unwind_and_problem() -> Result<(), serde_json::Error> {
    let response: HedgeConfirmResponse = serde_json::from_value(serde_json::json!({
        "idempotencyKey": "idem-1",
        "status": "first_leg_partial_unwind_failed",
        "executionRun": null,
        "longRecord": order_record(LiveOrderState::PartiallyFilled),
        "shortRecord": null,
        "unwindRecord": null,
        "problem": null,
        "partialOutcome": {
            "cause": "first_leg_partial",
            "originalStatus": "first_leg_partial_unwind_failed",
            "runId": "run-1",
            "runState": "unwind_required",
            "netExposureUsd": 42.0,
            "recoveryAction": "manual_review",
            "primaryMessage": "第一腿部分成交",
            "unwindStatus": "submit_failed",
            "unwindTargetLeg": "long",
            "unwindQuantity": 0.4,
            "unwindProblem": {
                "code": codes::HEDGE_UNWIND_SUBMIT_FAILED,
                "message": "route down"
            },
            "manualReviewRequired": true
        },
        "error": "route down"
    }))?;

    let detail = confirm_outcome_detail(&response).unwrap_or_default();

    assert!(detail.contains("事故 第一腿部分成交"));
    assert!(detail.contains("unwind 提交失败"));
    assert!(detail.contains("qty 0.400000"));
    assert!(detail.contains("需要人工复核"));
    assert!(detail.contains(codes::HEDGE_UNWIND_SUBMIT_FAILED));
    assert!(detail.contains("long okx/BTCUSDT/partial/filled 0.400000"));
    Ok(())
}

#[test]
fn confirm_outcome_detail_keeps_basic_submitted_response_visible() {
    let response = HedgeConfirmResponse {
        idempotency_key: "idem-1".into(),
        status: HedgeConfirmStatus::Submitted,
        context: shared_types::HedgeConfirmContext::default(),
        execution_run: None,
        long_record: None,
        short_record: None,
        unwind_record: None,
        problem: None,
        partial_outcome: None,
        error: None,
    };

    assert_eq!(
        confirm_outcome_detail(&response).as_deref(),
        Some("结果 已提交 · idempotency idem-1")
    );
}

#[test]
fn confirm_outcome_summary_hides_machine_identifiers() -> Result<(), serde_json::Error> {
    let response: HedgeConfirmResponse = serde_json::from_value(serde_json::json!({
        "idempotencyKey": "idem-machine-1",
        "status": "submitted",
        "executionRun": {
            "runId": "run-machine-1",
            "ticketId": "ticket-machine-1",
            "opportunityId": "opp-1",
            "state": "hedged",
            "longLeg": execution_leg("long", "bybit"),
            "shortLeg": execution_leg("short", "binance"),
            "netExposureUsd": 0.0,
            "evidence": {
                "schemaVersion": 2,
                "longLeg": { "role": "long" },
                "shortLeg": { "role": "short" }
            },
            "recoveryAction": null,
            "statusReason": "filled",
            "createdAtMs": 1,
            "updatedAtMs": 2
        },
        "longRecord": null,
        "shortRecord": null,
        "unwindRecord": null,
        "problem": null,
        "partialOutcome": null,
        "error": null
    }))?;

    let summary = confirm_outcome_summary(&response).unwrap_or_default();

    assert_eq!(summary, "提交结果 · 已提交 · BTCUSDT · bybit / binance");
    assert!(!summary.contains("machine"));
    Ok(())
}

#[test]
fn confirm_context_detail_keeps_runtime_identity_and_leg_problems() {
    let context = HedgeConfirmContext {
        opportunity_id: "opp-1".into(),
        idempotency_key: "idem-1".into(),
        ticket_id: Some("ticket-1".into()),
        run_id: Some("run-1".into()),
        environment: Some(ExecutionEnvironment::Live),
        long_venue: Some("okx".into()),
        short_venue: Some("bybit".into()),
        long_problem: None,
        short_problem: Some(shared_types::ApiProblem::new(
            "RATE_LIMITED",
            "short failed",
        )),
    };

    let detail = confirm_context_detail(&context).unwrap_or_default();

    assert!(detail.contains("实盘"));
    assert!(detail.contains("Ticket ticket-1"));
    assert!(detail.contains("Run run-1"));
    assert!(detail.contains("Idempotency idem-1"));
    assert!(detail.contains("long okx"));
    assert!(detail.contains("short bybit [RATE_LIMITED]"));
}

fn order_record(state: LiveOrderState) -> OrderRecord {
    OrderRecord {
        intent: OrderIntent {
            id: "order-1".into(),
            source: OrderSource::ArbitragePreview,
            strategy: None,
            mode: ExecutionMode::Live,
            exchange: "okx".into(),
            symbol: "BTCUSDT".into(),
            side: OrderSide::Buy,
            order_type: OrderType::Limit,
            quantity: 1.0,
            price: Some(100.0),
            slippage_tolerance_bps: None,
            reduce_only: false,
            time_in_force: TimeInForce::Ioc,
            post_only: false,
            margin_mode: MarginMode::Cross,
            leverage: 1.0,
            client_order_id: "client-1".into(),
            client_order_id_policy: None,
            created_at_ms: 1,
        },
        state,
        risk: None,
        identity: Default::default(),
        last_update_source: OrderUpdateSource::AdapterAck,
        exchange_order_id: Some("exchange-1".into()),
        message: None,
        filled_quantity: Some(0.4),
        filled_price: Some(100.0),
        filled_fee: None,
        updated_at_ms: 2,
    }
}

fn execution_leg(role: &str, exchange: &str) -> serde_json::Value {
    serde_json::json!({
        "role": role,
        "exchange": exchange,
        "symbol": "BTCUSDT",
        "orderIds": [],
        "state": "filled",
        "targetQuantity": 1.0,
        "filledQuantity": 1.0,
        "targetNotionalUsd": 100.0,
        "filledNotionalUsd": 100.0,
        "filledFee": 0.05
    })
}
