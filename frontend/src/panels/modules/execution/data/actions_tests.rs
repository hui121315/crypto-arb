use super::*;
use shared_types::{
    ExecutionCostReconciliation, ExecutionRun, ExecutionRunLeg, ExecutionRunState, HedgeLegRole,
    OrderUpdateSource, RecoveryAction,
};

#[path = "actions_tests/context.rs"]
mod context;
#[path = "actions_tests/outcome.rs"]
mod outcome;

#[test]
fn failed_label_keeps_mode_context() {
    assert_eq!(submit_failed_label("模拟"), "模拟提交失败");
    assert_eq!(submit_failed_label("实盘"), "实盘提交失败");
}

#[test]
fn local_confirm_failure_is_not_overwritten_by_a_stale_run_snapshot() {
    let failed = ActionState::failed(
        "模拟提交失败",
        shared_types::ApiProblem::new("HEDGE_PRE_TRADE_REJECTED", "scoped preflight blocked"),
    );

    assert!(!allows_execution_run_restore(&failed));
    assert!(!allows_execution_run_restore(&ActionState::pending(
        "提交中"
    )));
    assert!(allows_execution_run_restore(&ActionState::Idle));
    assert!(allows_execution_run_restore(&ActionState::accepted(
        "等待终态"
    )));
    assert!(allows_execution_run_restore(&ActionState::succeeded(
        "已完成"
    )));
}

#[test]
fn explicit_preview_ticket_rejects_historical_same_opportunity_run() {
    let run = run("run-old");

    assert_eq!(
        explicit_preview_run_match(&run, Some("ticket-new"), "opp-1"),
        Some(false),
    );
    assert_eq!(
        explicit_preview_run_match(&run, Some("ticket-1"), "opp-1"),
        Some(true),
    );
    assert_eq!(explicit_preview_run_match(&run, None, "opp-1"), None);
}

#[test]
fn confirm_request_keeps_api_seed_separate_from_ui_label() -> Result<(), serde_json::Error> {
    let request = ConfirmHedgeRequest {
        seed: ConfirmHedgeSeed::new(
            "opp-1".into(),
            "idem-1".into(),
            Some("ticket-1".into()),
            shared_types::ExecutionEnvironment::Paper,
            Some("okx".into()),
            Some("bybit".into()),
        ),
        mode_label: "模拟",
        client_order_ids: vec!["client-long".into(), "client-short".into()],
    };
    let body = serde_json::to_value(&request.seed.request)?;

    assert_static_label(request.mode_label);
    assert_eq!(request.seed.opportunity_id, "opp-1");
    assert_eq!(request.seed.context.long_venue.as_deref(), Some("okx"));
    assert_eq!(request.seed.context.short_venue.as_deref(), Some("bybit"));
    assert_eq!(
        body.get("idempotencyKey").and_then(|value| value.as_str()),
        Some("idem-1")
    );
    assert_eq!(
        body.get("ticketId").and_then(|value| value.as_str()),
        Some("ticket-1")
    );
    assert!(body.get("modeLabel").is_none());
    Ok(())
}

fn run(id: &str) -> ExecutionRun {
    run_with_state(id, ExecutionRunState::SecondLegSubmitted)
}

fn run_with_state(id: &str, state: ExecutionRunState) -> ExecutionRun {
    ExecutionRun {
        run_id: id.into(),
        ticket_id: "ticket-1".into(),
        opportunity_id: "opp-1".into(),
        state,
        long_leg: leg("long"),
        short_leg: leg("short"),
        net_exposure_usd: 0.0,
        cost_reconciliation: Some(ExecutionCostReconciliation::default()),
        valuation_problem: None,
        unwind_problem: None,
        finality_problem: None,
        finality_checked_at_ms: None,
        evidence: Default::default(),
        recovery_action: Some(RecoveryAction::CancelOpenOrders),
        status_reason: "submitted".into(),
        created_at_ms: 1,
        updated_at_ms: 1,
    }
}

fn leg(id: &str) -> ExecutionRunLeg {
    ExecutionRunLeg {
        role: if id == "long" {
            HedgeLegRole::Long
        } else {
            HedgeLegRole::Short
        },
        exchange: "mock".into(),
        symbol: "BTCUSDT".into(),
        order_ids: vec![format!("{id}-order")],
        identity: None,
        finality_source: Some(OrderUpdateSource::AdapterAck),
        confirmed_filled_at_ms: None,
        state: shared_types::LiveOrderState::Accepted,
        target_quantity: 1.0,
        filled_quantity: None,
        target_notional_usd: 100.0,
        filled_notional_usd: None,
        filled_fee: None,
    }
}

fn assert_static_label(_: &'static str) {}
