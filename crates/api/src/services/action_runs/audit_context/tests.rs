#![allow(clippy::panic)]

use super::*;

const HIGH_RISK_KINDS: [ActionRunKind; 22] = [
    ActionRunKind::TradingRiskConfigUpdate,
    ActionRunKind::TradingAdapterSelect,
    ActionRunKind::TradingKillSwitch,
    ActionRunKind::TradingFeeSnapshotUpsert,
    ActionRunKind::TradingOrderSubmit,
    ActionRunKind::TradingOrderCancel,
    ActionRunKind::TradingOrderReconcile,
    ActionRunKind::AutomationConfigUpdate,
    ActionRunKind::AutomationControl,
    ActionRunKind::AutomationLiveUnlock,
    ActionRunKind::HedgeConfirm,
    ActionRunKind::WebhookConfigUpdate,
    ActionRunKind::VenueCredentialsUpdate,
    ActionRunKind::VenueCredentialsClear,
    ActionRunKind::VenueCredentialsMigrate,
    ActionRunKind::OnchainProviderCredentialsUpdate,
    ActionRunKind::OnchainProviderCredentialsClear,
    ActionRunKind::PortfolioClosePosition,
    ActionRunKind::PortfolioClosePair,
    ActionRunKind::PortfolioCloseAll,
    ActionRunKind::PortfolioCloseCompensation,
    ActionRunKind::PortfolioCloseManualTerminal,
];

#[test]
fn every_high_risk_action_projects_one_canonical_mutation_route() {
    for kind in HIGH_RISK_KINDS {
        let context = action_event_context(&test_run(kind, None));
        assert!(
            matches!(context.method.as_deref(), Some("POST" | "PATCH")),
            "kind={kind:?}, method={:?}",
            context.method
        );
        assert!(
            context
                .path
                .as_deref()
                .is_some_and(|path| path.starts_with('/')),
            "kind={kind:?}, path={:?}",
            context.path
        );
        assert_eq!(context.action_kind, Some(kind));
        assert!(context.resource_kind.is_some());
    }
}

#[test]
fn order_context_is_flat_and_machine_queryable() {
    let mut run = test_run(
        ActionRunKind::TradingOrderSubmit,
        Some(json!({
            "intent": {
                "id": "internal-order-1",
                "clientOrderId": "client-order-1",
                "exchange": "okx",
                "symbol": "BTC-USDT-SWAP",
                "mode": "live"
            },
            "state": "rejected",
            "exchangeOrderId": "exchange-order-1"
        })),
    );
    run.status = ActionRunStatus::Failed;
    run.actor = "api-token:operator:0123456789abcdef".to_owned();
    run.problem = Some(ApiProblem {
        code: "ORDER_REJECTED".to_owned(),
        message: "order rejected".to_owned(),
        status: Some(422),
        request_id: run.request_id.clone(),
        retry_after_ms: None,
        source: Some("trading.order.submit".to_owned()),
        recovery_action: Some(shared_types::ApiRecoveryAction::ReviewRequest),
        details: None,
    });

    let value = serialized_context(&run);

    assert_eq!(value["method"], "POST");
    assert_eq!(value["path"], "/api/trading/orders");
    assert_eq!(value["status"], 422);
    assert_eq!(value["actorKind"], "bearer_token");
    assert_eq!(value["actionKind"], "trading_order_submit");
    assert_eq!(value["resourceKind"], "order");
    assert_eq!(value["orderId"], "internal-order-1");
    assert_eq!(value["clientOrderId"], "client-order-1");
    assert_eq!(value["venue"], "okx");
    assert_eq!(value["symbol"], "BTC-USDT-SWAP");
    assert_eq!(value["problemCode"], "ORDER_REJECTED");
}

#[test]
fn hedge_context_keeps_run_ticket_and_both_leg_subjects() {
    let run = test_run(
        ActionRunKind::HedgeConfirm,
        Some(json!({
            "idempotencyKey": "hedge-1",
            "status": "partial",
            "executionRun": {
                "runId": "run-1",
                "ticketId": "ticket-1",
                "longLeg": {
                    "exchange": "okx",
                    "symbol": "BTC-USDT-SWAP",
                    "orderIds": ["long-order-1"]
                },
                "shortLeg": {
                    "exchange": "bybit",
                    "symbol": "BTCUSDT",
                    "orderIds": ["short-order-1"]
                },
                "finalityProblem": {
                    "code": "HEDGE_ORDER_FINALITY_FAILED",
                    "status": 502
                }
            }
        })),
    );

    let value = serialized_context(&run);

    assert_eq!(value["runId"], "run-1");
    assert_eq!(value["ticketId"], "ticket-1");
    assert_eq!(value["orderId"], "long-order-1");
    assert_eq!(value["venues"], json!(["okx", "bybit"]));
    assert_eq!(value["symbols"], json!(["BTC-USDT-SWAP", "BTCUSDT"]));
    assert_eq!(value["problemCode"], "HEDGE_ORDER_FINALITY_FAILED");
}

fn test_run(kind: ActionRunKind, result: Option<serde_json::Value>) -> ActionRun {
    ActionRun {
        id: "act-test".to_owned(),
        kind,
        status: ActionRunStatus::Succeeded,
        actor: "system".to_owned(),
        target: Some("target".to_owned()),
        request_id: Some("req-test".to_owned()),
        idempotency_key: Some("idem-test".to_owned()),
        message: "done".to_owned(),
        problem: None,
        result,
        mutation: None,
        started_at_ms: 1,
        updated_at_ms: 2,
    }
}

fn serialized_context(run: &ActionRun) -> serde_json::Value {
    serde_json::to_value(action_event_context(run))
        .unwrap_or_else(|error| panic!("audit context encode failed: {error}"))
}
