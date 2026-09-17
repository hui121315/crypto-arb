#![allow(clippy::panic)]

use super::*;
use shared_types::ActionMutationChange;

#[test]
fn audit_detail_summarizes_close_run_cost_evidence() {
    let run = ActionRun {
        id: "act-1".to_owned(),
        kind: ActionRunKind::PortfolioClosePosition,
        status: ActionRunStatus::Succeeded,
        actor: "system".to_owned(),
        target: Some("close-1".to_owned()),
        request_id: Some("req-1".to_owned()),
        idempotency_key: Some("idem-1".to_owned()),
        message: "close run updated".to_owned(),
        problem: None,
        result: Some(json!({
            "id": "close-1",
            "status": "succeeded",
            "costReconciliation": {
                "totalActualCostUsd": 1.2,
                "evidenceOrderIds": ["order-1"],
                "evidenceEventIds": ["fee-1", "slippage-1"],
                "missingFields": [],
            }
        })),
        mutation: Some(ActionMutationDiff {
            effective_at_ms: 2,
            changes: vec![ActionMutationChange::MaxOpenOrders {
                before: 2,
                after: 4,
            }],
        }),
        started_at_ms: 1,
        updated_at_ms: 2,
    };

    let detail = audit_detail(&run, "success")
        .unwrap_or_else(|error| panic!("audit detail failed: {error}"));

    assert_eq!(detail["resultSummary"]["closeRunId"], "close-1");
    assert_eq!(
        detail["resultSummary"]["costReconciliation"]["evidenceEventIds"][1],
        "slippage-1"
    );
    assert_eq!(detail["actionRun"]["id"], "act-1");
    assert_eq!(detail["mutation"]["changes"][0]["field"], "max_open_orders");
    assert_eq!(detail["actionRun"]["mutation"]["changes"][0]["after"], 4);
    assert!(detail["actionRun"].get("result").is_none());

    let correlation = to_json_value(&action_correlation(&run));
    assert_eq!(correlation["requestId"], "req-1");
    assert_eq!(correlation["actionRunId"], "act-1");
    assert_eq!(correlation["idempotencyKey"], "idem-1");
    assert_eq!(correlation["orderIds"][0], "order-1");
    assert_eq!(correlation["runIds"][0], "close-1");
}

#[test]
fn audit_detail_summarizes_hedge_confirm_execution_evidence() {
    let run = ActionRun {
        id: "act-hedge".to_owned(),
        kind: ActionRunKind::HedgeConfirm,
        status: ActionRunStatus::Succeeded,
        actor: "system".to_owned(),
        target: Some("opp-1".to_owned()),
        request_id: Some("req-hedge".to_owned()),
        idempotency_key: Some("hedge-1".to_owned()),
        message: "hedge confirm submitted".to_owned(),
        problem: None,
        result: Some(json!({
            "idempotencyKey": "hedge-1",
            "status": "submitted",
            "executionRun": {
                "runId": "run-hedge-1",
                "ticketId": "ticket-1",
                "opportunityId": "opp-1",
                "state": "hedged",
                "statusReason": "both legs filled",
                "netExposureUsd": 0.0,
                "longLeg": {
                    "exchange": "okx",
                    "symbol": "BTCUSDT",
                    "state": "filled",
                    "orderIds": ["long-order-1"],
                },
                "shortLeg": {
                    "exchange": "bybit",
                    "symbol": "BTCUSDT",
                    "state": "filled",
                    "orderIds": ["short-order-1"],
                },
                "finalityProblem": {
                    "code": "HEDGE_ORDER_FINALITY_FAILED",
                    "status": 502,
                    "retryAfterMs": 5000,
                },
                "costReconciliation": {
                    "actualOpenCostUsd": 1.25,
                    "actualFundingUsd": -0.12,
                    "fundingEventIds": ["funding-1"],
                    "actualUnwindFeeUsd": 0.33,
                    "actualUnwindSlippageUsd": 0.44,
                    "actualUnwindCostUsd": 0.77,
                    "unwindEventIds": ["unwind-fill-1", "unwind-slippage-1"],
                    "missingFields": [],
                    "actualCostUsd": 1.90,
                    "costDeltaUsd": -0.10,
                }
            }
        })),
        mutation: None,
        started_at_ms: 1,
        updated_at_ms: 2,
    };

    let detail = audit_detail(&run, "success")
        .unwrap_or_else(|error| panic!("audit detail failed: {error}"));

    assert_eq!(
        detail["resultSummary"]["executionRun"]["runId"],
        "run-hedge-1"
    );
    assert_eq!(
        detail["resultSummary"]["executionRun"]["costReconciliation"]["fundingEventIds"][0],
        "funding-1"
    );
    assert_eq!(
        detail["resultSummary"]["executionRun"]["costReconciliation"]["unwindEventIds"][1],
        "unwind-slippage-1"
    );
    assert_eq!(
        detail["resultSummary"]["executionRun"]["costReconciliation"]["actualUnwindCostUsd"],
        0.77
    );
    assert_eq!(
        detail["resultSummary"]["executionRun"]["longLeg"]["orderIds"][0],
        "long-order-1"
    );
    assert_eq!(
        detail["resultSummary"]["executionRun"]["finalityProblem"]["code"],
        "HEDGE_ORDER_FINALITY_FAILED"
    );

    let correlation = to_json_value(&action_correlation(&run));
    assert_eq!(correlation["requestId"], "req-hedge");
    assert_eq!(correlation["actionRunId"], "act-hedge");
    assert_eq!(
        correlation["orderIds"],
        json!(["long-order-1", "short-order-1"])
    );
    assert_eq!(correlation["runIds"], json!(["run-hedge-1"]));
}

#[test]
fn order_action_correlation_keeps_internal_client_and_exchange_ids() {
    let run = ActionRun {
        id: "act-order".to_owned(),
        kind: ActionRunKind::TradingOrderSubmit,
        status: ActionRunStatus::Succeeded,
        actor: "system".to_owned(),
        target: Some("client-order-1".to_owned()),
        request_id: Some("req-order".to_owned()),
        idempotency_key: Some("idem-order".to_owned()),
        message: "order submitted".to_owned(),
        problem: None,
        result: Some(json!({
            "intent": {
                "id": "internal-order-1",
                "clientOrderId": "client-order-1",
                "exchange": "okx",
                "symbol": "BTC-USDT-SWAP",
                "mode": "live"
            },
            "state": "accepted",
            "exchangeOrderId": "exchange-order-1"
        })),
        mutation: None,
        started_at_ms: 1,
        updated_at_ms: 2,
    };

    let correlation = to_json_value(&action_correlation(&run));

    assert_eq!(
        correlation["orderIds"],
        json!(["internal-order-1", "client-order-1", "exchange-order-1"])
    );
    assert!(correlation.get("runIds").is_none());
}
