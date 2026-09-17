use super::collect::{collect_close_run_orders, collect_execution_run_orders};
use super::*;
use shared_types::{
    CloseLeg, CloseRunScope, CloseRunStatus, ExecutionMode, ExecutionRunState, HedgeLegRole,
    MarginMode, OrderIntent, OrderRecord, OrderSide, OrderSource, OrderType, PositionSide,
    TimeInForce,
};

#[path = "tests/terminal_projection.rs"]
mod terminal_projection;

#[test]
fn terminal_order_state_only_skips_final_states() {
    let terminal = [
        LiveOrderState::Filled,
        LiveOrderState::Cancelled,
        LiveOrderState::Rejected,
        LiveOrderState::Failed,
    ];
    for state in terminal {
        assert!(is_terminal_order_state(state));
    }
    let pending = [
        LiveOrderState::Created,
        LiveOrderState::RiskChecked,
        LiveOrderState::Submitted,
        LiveOrderState::Accepted,
        LiveOrderState::PartiallyFilled,
        LiveOrderState::CancelRequested,
        LiveOrderState::Unknown,
    ];
    for state in pending {
        assert!(!is_terminal_order_state(state));
    }
}

#[test]
fn pending_order_collection_dedupes_execution_and_close_refs() {
    let mut refs = PendingOrderMap::new();
    collect_execution_run_orders(&execution_run("order-1", "filled-1"), &mut refs);
    collect_close_run_orders(&close_run("order-1", "close-1"), &mut refs);

    let rows: Vec<_> = refs.into_iter().collect();

    assert_eq!(
        rows,
        vec![
            ("close-1".to_owned(), PendingOrderSource::CloseRun),
            ("order-1".to_owned(), PendingOrderSource::Mixed),
        ]
    );
}

#[test]
fn finality_remote_missing_problem_carries_order_context() {
    let target = PendingOrderTarget {
        raw_order_id: "exchange-1".to_owned(),
        internal_order_id: "internal-1".to_owned(),
        venue: "okx".to_owned(),
        source: PendingOrderSource::ExecutionRun,
        state: LiveOrderState::Accepted,
    };

    let problem = finality_remote_missing_problem(&target, 42);

    assert_eq!(problem.code, codes::HEDGE_ORDER_FINALITY_FAILED);
    assert_eq!(problem.status, Some(409));
    assert_eq!(problem.source.as_deref(), Some(FINALITY_PROBLEM_SOURCE));
    let details = problem
        .details
        .unwrap_or_else(|| serde_json::json!({"missing": true}));
    assert_eq!(
        details,
        serde_json::json!({
            "checkedAtMs": 42,
            "error": null,
            "internalOrderId": "internal-1",
            "orderState": "accepted",
            "rawOrderId": "exchange-1",
            "source": "ExecutionRun",
            "venue": "okx",
        })
    );
}

#[test]
fn outcome_tracks_per_venue_counters() {
    let target = PendingOrderTarget {
        raw_order_id: "exchange-1".to_owned(),
        internal_order_id: "internal-1".to_owned(),
        venue: "binance".to_owned(),
        source: PendingOrderSource::ExecutionRun,
        state: LiveOrderState::Accepted,
    };
    let mut outcome = RunFinalityOutcome::default();

    outcome.record_scanned(&target);
    outcome.record_refreshed(&target);

    assert!(outcome.venue_outcomes.contains_key("binance"));
    let venue = &outcome.venue_outcomes["binance"];
    assert_eq!(outcome.refreshed_order_count, 1);
    assert_eq!(venue.scanned_order_count, 1);
    assert_eq!(venue.refreshed_order_count, 1);
}

#[test]
fn failure_records_first_problem_sample() {
    let target_a = PendingOrderTarget {
        raw_order_id: "exchange-a".to_owned(),
        internal_order_id: "internal-a".to_owned(),
        venue: "okx".to_owned(),
        source: PendingOrderSource::ExecutionRun,
        state: LiveOrderState::Accepted,
    };
    let target_b = PendingOrderTarget {
        raw_order_id: "exchange-b".to_owned(),
        internal_order_id: "internal-b".to_owned(),
        venue: "okx".to_owned(),
        source: PendingOrderSource::CloseRun,
        state: LiveOrderState::Submitted,
    };
    let mut outcome = RunFinalityOutcome::default();

    let first = finality_remote_missing_problem(&target_a, 7);
    outcome.record_remote_missing(&target_a, &first, 7);
    let second = finality_remote_missing_problem(&target_b, 9);
    outcome.record_remote_missing(&target_b, &second, 9);

    let venue_sample = outcome
        .venue_outcomes
        .get("okx")
        .and_then(|venue| venue.sample_problem.as_ref());
    assert_eq!(
        outcome
            .sample_problem
            .as_ref()
            .map(|sample| sample.raw_order_id.as_str()),
        Some("exchange-a")
    );
    assert_eq!(
        venue_sample.map(|sample| sample.internal_order_id.as_str()),
        Some("internal-a")
    );
    assert_eq!(
        venue_sample.map(|sample| sample.source.as_str()),
        Some("ExecutionRun")
    );
    assert_eq!(venue_sample.and_then(|sample| sample.status), Some(409));
    assert_eq!(
        venue_sample.map(|sample| sample.message.as_str()),
        Some(first.message.as_str())
    );
}

fn execution_run(pending_order_id: &str, terminal_order_id: &str) -> ExecutionRun {
    ExecutionRun {
        run_id: "run-1".to_owned(),
        ticket_id: "ticket-1".to_owned(),
        opportunity_id: "opp-1".to_owned(),
        state: ExecutionRunState::SecondLegSubmitted,
        long_leg: execution_leg(
            HedgeLegRole::Long,
            pending_order_id,
            LiveOrderState::Submitted,
        ),
        short_leg: execution_leg(
            HedgeLegRole::Short,
            terminal_order_id,
            LiveOrderState::Filled,
        ),
        net_exposure_usd: 0.0,
        cost_reconciliation: None,
        valuation_problem: None,
        unwind_problem: None,
        finality_problem: None,
        finality_checked_at_ms: None,
        evidence: Default::default(),
        recovery_action: None,
        status_reason: "waiting".to_owned(),
        created_at_ms: 1,
        updated_at_ms: 1,
    }
}

fn execution_leg(role: HedgeLegRole, order_id: &str, state: LiveOrderState) -> ExecutionRunLeg {
    ExecutionRunLeg {
        role,
        exchange: "okx".to_owned(),
        symbol: "BTCUSDT".to_owned(),
        order_ids: vec![order_id.to_owned()],
        identity: None,
        finality_source: None,
        confirmed_filled_at_ms: None,
        state,
        target_quantity: 1.0,
        filled_quantity: None,
        target_notional_usd: 100.0,
        filled_notional_usd: None,
        filled_fee: None,
    }
}

fn close_run(shared_order_id: &str, close_order_id: &str) -> CloseRun {
    CloseRun {
        id: "close-run-1".to_owned(),
        scope: CloseRunScope::Pair,
        status: CloseRunStatus::PartiallySubmitted,
        action_run_id: None,
        request_id: None,
        idempotency_key: None,
        snapshot_version: "snap-1".to_owned(),
        expected_leg_count: 2,
        reason: None,
        legs: vec![
            close_leg(shared_order_id, LiveOrderState::Accepted),
            close_leg(close_order_id, LiveOrderState::CancelRequested),
            close_leg("filled-close", LiveOrderState::Filled),
        ],
        submitted_order_count: 2,
        failed_leg_count: 0,
        naked_exposure_usd: 0.0,
        message: "waiting".to_owned(),
        problem: None,
        finality_problem: None,
        finality_checked_at_ms: None,
        unwind_plan: None,
        cost_events: Vec::new(),
        cost_reconciliation: None,
        started_at_ms: 1,
        updated_at_ms: 1,
    }
}

fn close_leg(order_id: &str, state: LiveOrderState) -> CloseLeg {
    CloseLeg {
        venue: "okx".to_owned(),
        symbol: "BTCUSDT".to_owned(),
        side: PositionSide::Long,
        status: shared_types::CloseLegStatus::Submitted,
        quantity: 1.0,
        mark_price: 100.0,
        notional_usd: 100.0,
        order: Some(order_record(order_id, state)),
        finality_source: None,
        confirmed_filled_at_ms: None,
        problem: None,
        pair_evidence: None,
        cost_events: Vec::new(),
    }
}

fn order_record(order_id: &str, state: LiveOrderState) -> OrderRecord {
    OrderRecord {
        intent: OrderIntent {
            id: order_id.to_owned(),
            source: OrderSource::ArbitragePreview,
            strategy: None,
            mode: ExecutionMode::DryRun,
            exchange: "okx".to_owned(),
            symbol: "BTCUSDT".to_owned(),
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
            client_order_id: format!("client-{order_id}"),
            client_order_id_policy: None,
            created_at_ms: 1,
        },
        state,
        risk: None,
        identity: Default::default(),
        last_update_source: Default::default(),
        exchange_order_id: None,
        message: None,
        filled_quantity: None,
        filled_price: None,
        filled_fee: None,
        updated_at_ms: 1,
    }
}
