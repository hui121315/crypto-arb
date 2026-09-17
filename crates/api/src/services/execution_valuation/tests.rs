#![allow(clippy::expect_used)]

use super::*;
use shared_types::{
    ExecutionCostReconciliation, ExecutionMode, ExecutionRunState, MarginMode, OrderSide,
    OrderSource, OrderType, OrderUpdateSource, TimeInForce,
};

#[test]
fn fill_notional_requires_filled_price() -> Result<(), &'static str> {
    let mut record = record(Some(99.0), Some(100.0));

    assert_eq!(
        fill_notional(&record, 0.5).map_err(|_| "filled notional missing")?,
        49.5
    );

    record.filled_price = None;
    let problem = fill_notional(&record, 0.5).map_err(|problem| problem.code.clone());
    assert_eq!(
        problem,
        Err(codes::HEDGE_EXECUTION_VALUATION_MISSING.to_owned())
    );
    Ok(())
}

#[test]
fn missing_price_returns_typed_problem() {
    let result = order_notional(&intent(None));
    assert!(result.is_err());
    let Some(problem) = result.err() else {
        return;
    };

    assert_eq!(problem.code, codes::HEDGE_EXECUTION_VALUATION_MISSING);
    assert_eq!(problem.source.as_deref(), Some(SOURCE_EXECUTION_VALUATION));
    assert_eq!(
        problem
            .details
            .as_ref()
            .and_then(|details| details.get("reason"))
            .and_then(serde_json::Value::as_str),
        Some("missing_intent_price")
    );
}

#[test]
fn cost_reconciliation_requires_complete_fee_and_slippage_evidence() {
    let mut run = run();
    run.long_leg.filled_fee = Some(0.2);

    refresh_cost_reconciliation(&mut run);

    let cost = run.cost_reconciliation.expect("cost reconciliation");
    assert_eq!(cost.filled_fee_usd, Some(0.2));
    assert_eq!(cost.actual_slippage_usd, Some(0.0));
    assert_eq!(cost.actual_open_cost_usd, None);
    assert_eq!(cost.actual_cost_usd, None);
    assert_eq!(cost.cost_delta_usd, None);
}

#[test]
fn cost_reconciliation_preserves_funding_attribution_on_refresh() {
    let mut run = run();
    let cost = run
        .cost_reconciliation
        .as_mut()
        .expect("cost reconciliation");
    cost.actual_funding_usd = Some(-0.12);
    cost.funding_event_ids.push("funding-1".to_owned());

    refresh_cost_reconciliation(&mut run);

    let cost = run.cost_reconciliation.expect("cost reconciliation");
    assert_eq!(cost.actual_funding_usd, Some(-0.12));
    assert_eq!(cost.funding_event_ids, ["funding-1"]);
    assert_eq!(cost.actual_cost_usd, None);
    assert_eq!(cost.cost_delta_usd, None);
}

#[test]
fn equal_base_fills_are_delta_neutral_even_when_entry_prices_differ() {
    let mut run = run();
    run.long_leg.filled_quantity = Some(1.0);
    run.long_leg.filled_notional_usd = Some(100.0);
    run.short_leg.filled_quantity = Some(1.0);
    run.short_leg.filled_notional_usd = Some(101.0);

    assert_eq!(net_base_exposure_usd(&run), 0.0);

    run.short_leg.filled_quantity = Some(0.9);
    run.short_leg.filled_notional_usd = Some(90.9);
    assert!((net_base_exposure_usd(&run) - 10.05).abs() < 1e-9);
}

fn record(filled_price: Option<f64>, intent_price: Option<f64>) -> OrderRecord {
    OrderRecord {
        intent: intent(intent_price),
        state: shared_types::LiveOrderState::Filled,
        risk: None,
        identity: Default::default(),
        last_update_source: OrderUpdateSource::OrderQuery,
        exchange_order_id: None,
        message: None,
        filled_quantity: Some(1.0),
        filled_price,
        filled_fee: None,
        updated_at_ms: 1,
    }
}

fn intent(price: Option<f64>) -> OrderIntent {
    OrderIntent {
        id: "order-1".to_owned(),
        source: OrderSource::ArbitragePreview,
        strategy: None,
        mode: ExecutionMode::Live,
        exchange: "okx".to_owned(),
        symbol: "BTC-USDT".to_owned(),
        side: OrderSide::Buy,
        order_type: OrderType::Market,
        quantity: 1.0,
        price,
        slippage_tolerance_bps: None,
        reduce_only: false,
        time_in_force: TimeInForce::Ioc,
        post_only: false,
        margin_mode: MarginMode::Cross,
        leverage: 1.0,
        client_order_id: "client-order-1".to_owned(),
        client_order_id_policy: None,
        created_at_ms: 1,
    }
}

fn run() -> ExecutionRun {
    ExecutionRun {
        run_id: "run-1".to_owned(),
        ticket_id: "ticket-1".to_owned(),
        opportunity_id: "opp-1".to_owned(),
        state: ExecutionRunState::Hedged,
        long_leg: leg(HedgeLegRole::Long),
        short_leg: leg(HedgeLegRole::Short),
        net_exposure_usd: 0.0,
        cost_reconciliation: Some(ExecutionCostReconciliation {
            estimated_open_cost_usd: 1.0,
            estimated_close_cost_usd: 1.0,
            estimated_slippage_usd: 1.0,
            estimated_total_cost_usd: 3.0,
            filled_fee_usd: None,
            actual_slippage_usd: None,
            actual_open_cost_usd: None,
            actual_funding_usd: None,
            funding_event_ids: Vec::new(),
            actual_unwind_fee_usd: None,
            actual_unwind_slippage_usd: None,
            actual_unwind_cost_usd: None,
            unwind_event_ids: Vec::new(),
            missing_fields: Vec::new(),
            actual_cost_usd: None,
            cost_delta_usd: None,
        }),
        valuation_problem: None,
        unwind_problem: None,
        finality_problem: None,
        finality_checked_at_ms: None,
        evidence: Default::default(),
        recovery_action: None,
        status_reason: "seed".to_owned(),
        created_at_ms: 1,
        updated_at_ms: 1,
    }
}

fn leg(role: HedgeLegRole) -> ExecutionRunLeg {
    ExecutionRunLeg {
        role,
        exchange: "okx".to_owned(),
        symbol: "BTC-USDT".to_owned(),
        order_ids: Vec::new(),
        identity: None,
        finality_source: None,
        confirmed_filled_at_ms: None,
        state: shared_types::LiveOrderState::Filled,
        target_quantity: 1.0,
        filled_quantity: Some(1.0),
        target_notional_usd: 100.0,
        filled_notional_usd: Some(100.0),
        filled_fee: None,
    }
}
