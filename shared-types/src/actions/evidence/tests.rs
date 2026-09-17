use super::*;
use crate::{
    ActionRun, ActionRunStatus, CloseLeg, CloseLegStatus, CloseRun, CloseRunCostReconciliation,
    CloseRunScope, CloseRunStatus, PositionSide,
};

#[test]
fn action_run_snapshot_restores_kind_request_and_idempotency_context() {
    let run = ActionRun {
        id: "action-1".into(),
        kind: ActionRunKind::TradingKillSwitch,
        status: ActionRunStatus::Succeeded,
        actor: "tester".into(),
        target: Some("trading".into()),
        request_id: Some("req-1".into()),
        idempotency_key: Some("idem-1".into()),
        message: "done".into(),
        problem: None,
        result: None,
        mutation: None,
        started_at_ms: 10,
        updated_at_ms: 20,
    };

    let evidence = ActionEvidence::from_action_run(&run);

    assert_eq!(evidence.action_kind, Some(ActionRunKind::TradingKillSwitch));
    assert_eq!(evidence.action_run_id.as_deref(), Some("action-1"));
    assert_eq!(evidence.request_id.as_deref(), Some("req-1"));
    assert_eq!(evidence.idempotency_key.as_deref(), Some("idem-1"));
    assert_eq!(evidence.observed_at_ms, Some(20));
    assert!(evidence.sources.contains(&ActionEvidenceSource::ActionRun));
}

#[test]
fn merge_deduplicates_order_and_scope_identity_lists() {
    let first = ActionEvidence::client_request("req-1", Some("idem-1".into()))
        .with_action_kind(ActionRunKind::PortfolioClosePair)
        .with_client_order_ids(["client-1".into()])
        .with_venues(["binance".into()])
        .with_symbols(["BTCUSDT".into()]);
    let second = ActionEvidence::default()
        .with_action_kind(ActionRunKind::TradingOrderSubmit)
        .with_run_id(Some("run-1".into()))
        .with_client_order_ids(["client-1".into(), "client-2".into()])
        .with_venues(["binance".into(), "okx".into()])
        .with_symbols(["BTCUSDT".into(), "BTC-USDT-SWAP".into()]);

    let merged = first.merged(second);

    assert_eq!(merged.action_kind, Some(ActionRunKind::PortfolioClosePair));
    assert_eq!(merged.client_order_ids, ["client-1", "client-2"]);
    assert_eq!(merged.venues, ["binance", "okx"]);
    assert_eq!(merged.symbols, ["BTCUSDT", "BTC-USDT-SWAP"]);
    assert_eq!(merged.run_id.as_deref(), Some("run-1"));
}

#[test]
fn close_run_preserves_action_scope_and_order_evidence() {
    let run = CloseRun {
        id: "close-pair-1".into(),
        scope: CloseRunScope::Pair,
        status: CloseRunStatus::PartiallySubmitted,
        action_run_id: Some("action-close-1".into()),
        request_id: Some("req-close-1".into()),
        idempotency_key: Some("idem-close-1".into()),
        snapshot_version: "positions-v1".into(),
        expected_leg_count: 2,
        reason: Some("positions.close_pair".into()),
        legs: vec![
            close_leg("binance", "BTCUSDT", PositionSide::Long),
            close_leg("okx", "BTC-USDT-SWAP", PositionSide::Short),
        ],
        submitted_order_count: 1,
        failed_leg_count: 1,
        naked_exposure_usd: 1_000.0,
        message: "one leg requires compensation".into(),
        problem: None,
        finality_problem: None,
        finality_checked_at_ms: None,
        unwind_plan: None,
        cost_events: Vec::new(),
        cost_reconciliation: Some(CloseRunCostReconciliation {
            evidence_order_ids: vec!["order-close-1".into()],
            ..CloseRunCostReconciliation::default()
        }),
        started_at_ms: 10,
        updated_at_ms: 20,
    };

    let evidence = ActionEvidence::from_close_run(&run);
    let summary = evidence.summary();

    assert_eq!(
        evidence.action_kind,
        Some(ActionRunKind::PortfolioClosePair)
    );
    assert_eq!(evidence.venues, ["binance", "okx"]);
    assert_eq!(evidence.symbols, ["BTCUSDT", "BTC-USDT-SWAP"]);
    assert_eq!(evidence.order_ids, ["order-close-1"]);
    assert!(summary.contains("action_kind portfolio_close_pair"));
    assert!(summary.contains("venue binance,okx"));
    assert!(summary.contains("symbol BTCUSDT,BTC-USDT-SWAP"));
}

fn close_leg(venue: &str, symbol: &str, side: PositionSide) -> CloseLeg {
    CloseLeg {
        venue: venue.into(),
        symbol: symbol.into(),
        side,
        status: CloseLegStatus::Submitted,
        quantity: 1.0,
        mark_price: 100.0,
        notional_usd: 100.0,
        order: None,
        finality_source: None,
        confirmed_filled_at_ms: None,
        problem: None,
        pair_evidence: None,
        cost_events: Vec::new(),
    }
}
