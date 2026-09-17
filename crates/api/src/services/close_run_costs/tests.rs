use super::*;
use shared_types::{
    CloseLeg, CloseLegStatus, CloseRunScope, CloseRunStatus, ExecutionMode, LiveOrderState,
    MarginMode, OrderIntent, OrderSide, OrderSource, OrderType, OrderUpdateSource, PositionSide,
    TimeInForce,
};

#[test]
fn reconciles_complete_close_and_compensation_costs() -> Result<(), &'static str> {
    let mut run = close_run(close_leg(order_record(
        "close-1",
        OrderSide::Sell,
        99.0,
        0.2,
    )));
    run.cost_events = vec![
        cost_event("funding-1", CloseRunCostComponent::Funding, -0.12),
        cost_event("manual-1", CloseRunCostComponent::ManualHandling, 4.0),
    ];
    run.unwind_plan = Some(shared_types::CloseRunUnwindPlan {
        status: shared_types::CloseRunUnwindPlanStatus::Compensated,
        filled_legs: Vec::new(),
        failed_legs: Vec::new(),
        compensation_candidates: Vec::new(),
        remaining_positions: Vec::new(),
        compensation_attempts: vec![shared_types::CloseRunCompensationAttempt {
            venue: "binance".to_owned(),
            symbol: "MUUSDT".to_owned(),
            side: PositionSide::Long,
            compensation_order_side: OrderSide::Buy,
            target_quantity: 1.0,
            status: CloseLegStatus::Filled,
            order: Some(order_record("comp-1", OrderSide::Buy, 101.0, 0.3)),
            finality_source: None,
            confirmed_filled_at_ms: Some(2),
            problem: None,
            cost_events: vec![
                cost_event("comp-fee-1", CloseRunCostComponent::Fee, 0.3),
                cost_event("comp-slippage-1", CloseRunCostComponent::Slippage, 1.0),
            ],
            action_run_id: None,
            submitted_at_ms: 1,
            updated_at_ms: 2,
        }],
        manual_terminal_evidence: None,
        next_actions: Vec::new(),
        required_evidence: Vec::new(),
    });

    refresh_cost_reconciliation(&mut run);

    let cost = run
        .cost_reconciliation
        .ok_or("cost reconciliation missing")?;
    assert_eq!(cost.close_fee_usd, Some(0.2));
    assert_eq!(cost.close_slippage_usd, Some(1.0));
    assert_eq!(cost.compensation_fee_usd, Some(0.3));
    assert_eq!(cost.compensation_slippage_usd, Some(1.0));
    assert_eq!(cost.funding_usd, Some(-0.12));
    assert_eq!(cost.manual_handling_usd, Some(4.0));
    assert_eq!(cost.total_actual_cost_usd, Some(6.38));
    assert_eq!(
        cost.evidence_event_ids,
        vec![
            "close-fee-1".to_owned(),
            "close-slippage-1".to_owned(),
            "comp-fee-1".to_owned(),
            "comp-slippage-1".to_owned(),
            "funding-1".to_owned(),
            "manual-1".to_owned(),
        ]
    );
    assert_eq!(cost.funding_event_ids, vec!["funding-1".to_owned()]);
    assert_eq!(cost.manual_handling_event_ids, vec!["manual-1".to_owned()]);
    assert!(cost.missing_fields.is_empty());
    Ok(())
}

#[test]
fn reconciles_run_level_manual_cost_without_order_costs() -> Result<(), &'static str> {
    let mut run = close_run(close_leg(order_record_without_fee(
        "close-1",
        OrderSide::Sell,
        99.0,
    )));
    run.legs.clear();
    run.cost_events = vec![cost_event(
        "manual-1",
        CloseRunCostComponent::ManualHandling,
        12.5,
    )];

    refresh_cost_reconciliation(&mut run);

    let cost = run
        .cost_reconciliation
        .ok_or("cost reconciliation missing")?;
    assert_eq!(cost.manual_handling_usd, Some(12.5));
    assert_eq!(cost.total_actual_cost_usd, Some(12.5));
    assert!(cost.missing_fields.is_empty());
    Ok(())
}

#[test]
fn keeps_total_pending_when_fee_evidence_is_missing() -> Result<(), &'static str> {
    let mut run = close_run(close_leg(order_record_without_fee(
        "close-1",
        OrderSide::Sell,
        99.0,
    )));

    refresh_cost_reconciliation(&mut run);

    let cost = run
        .cost_reconciliation
        .ok_or("cost reconciliation missing")?;
    assert_eq!(cost.close_slippage_usd, Some(1.0));
    assert_eq!(cost.total_actual_cost_usd, None);
    assert_eq!(cost.missing_fields, vec![CLOSE_FEE.to_owned()]);
    assert_eq!(
        cost.close_slippage_event_ids,
        vec!["close-slippage-1".to_owned()]
    );
    Ok(())
}

#[test]
fn fill_derived_slippage_without_ledger_event_stays_missing() -> Result<(), &'static str> {
    let mut leg = close_leg(order_record("close-1", OrderSide::Sell, 99.0, 0.2));
    leg.cost_events
        .retain(|event| event.component != CloseRunCostComponent::Slippage);
    let mut run = close_run(leg);

    refresh_cost_reconciliation(&mut run);

    let cost = run
        .cost_reconciliation
        .ok_or("cost reconciliation missing")?;
    assert_eq!(cost.close_fee_usd, Some(0.2));
    assert_eq!(cost.close_slippage_usd, None);
    assert_eq!(cost.total_actual_cost_usd, None);
    assert_eq!(cost.missing_fields, vec![CLOSE_SLIPPAGE.to_owned()]);
    Ok(())
}

fn close_run(leg: CloseLeg) -> CloseRun {
    CloseRun {
        id: "close-run-1".to_owned(),
        scope: CloseRunScope::Single,
        status: CloseRunStatus::Submitted,
        action_run_id: None,
        request_id: None,
        idempotency_key: None,
        snapshot_version: "pos-1".to_owned(),
        expected_leg_count: 1,
        reason: None,
        legs: vec![leg],
        submitted_order_count: 1,
        failed_leg_count: 0,
        naked_exposure_usd: 0.0,
        message: "submitted".to_owned(),
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

fn close_leg(order: OrderRecord) -> CloseLeg {
    CloseLeg {
        venue: "binance".to_owned(),
        symbol: "MUUSDT".to_owned(),
        side: PositionSide::Long,
        status: CloseLegStatus::Filled,
        quantity: 1.0,
        mark_price: 100.0,
        notional_usd: 100.0,
        order: Some(order),
        finality_source: None,
        confirmed_filled_at_ms: Some(2),
        problem: None,
        pair_evidence: None,
        cost_events: vec![
            cost_event("close-fee-1", CloseRunCostComponent::Fee, 0.2),
            cost_event("close-slippage-1", CloseRunCostComponent::Slippage, 1.0),
        ],
    }
}

fn order_record(id: &str, side: OrderSide, price: f64, fee: f64) -> OrderRecord {
    let mut record = order_record_without_fee(id, side, price);
    record.filled_fee = Some(fee);
    record
}

fn cost_event(
    event_id: &str,
    component: CloseRunCostComponent,
    amount_usd: f64,
) -> CloseRunCostLedgerEvent {
    CloseRunCostLedgerEvent {
        event_id: event_id.to_owned(),
        component,
        amount_usd,
        source: OrderUpdateSource::PrivateWs,
        quality: ExecutionLedgerQuality::Actual,
        occurred_at_ms: 10,
        captured_at_ms: 11,
    }
}

fn order_record_without_fee(id: &str, side: OrderSide, price: f64) -> OrderRecord {
    OrderRecord {
        intent: OrderIntent {
            id: id.to_owned(),
            source: OrderSource::Manual,
            strategy: None,
            mode: ExecutionMode::DryRun,
            exchange: "binance".to_owned(),
            symbol: "MUUSDT".to_owned(),
            side,
            order_type: OrderType::Market,
            quantity: 1.0,
            price: Some(100.0),
            slippage_tolerance_bps: None,
            reduce_only: true,
            time_in_force: TimeInForce::Ioc,
            post_only: false,
            margin_mode: MarginMode::Cross,
            leverage: 1.0,
            client_order_id: format!("client-{id}"),
            client_order_id_policy: None,
            created_at_ms: 1,
        },
        state: LiveOrderState::Filled,
        risk: None,
        identity: Default::default(),
        last_update_source: Default::default(),
        exchange_order_id: None,
        message: None,
        filled_quantity: Some(1.0),
        filled_price: Some(price),
        filled_fee: None,
        updated_at_ms: 2,
    }
}
