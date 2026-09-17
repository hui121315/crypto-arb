use super::super::events::{first_leg_event, order_finality_complete};
use super::super::run_model::exact_fill_quantity;
use super::super::*;
use super::*;
use shared_types::problem::codes;

#[test]
fn first_leg_event_marks_partial_fill() {
    let mut record = order_record(LiveOrderState::Submitted);
    assert_eq!(first_leg_event(&record), "hedge_first_leg_submitted");
    record.state = LiveOrderState::PartiallyFilled;
    assert_eq!(first_leg_event(&record), "hedge_first_leg_partial");
}

#[test]
fn exact_fill_quantity_requires_positive_finite_quantity() {
    let mut record = order_record(LiveOrderState::PartiallyFilled);
    assert_eq!(exact_fill_quantity(&record), None);
    record.filled_quantity = Some(0.4);
    assert_eq!(exact_fill_quantity(&record), Some(0.4));
    record.filled_quantity = Some(f64::NAN);
    assert_eq!(exact_fill_quantity(&record), None);
}

#[test]
fn filled_state_waits_for_complete_quantity_and_price_evidence() {
    let mut record = order_record(LiveOrderState::Filled);
    assert!(!order_finality_complete(&record));

    record.filled_quantity = Some(1.0);
    assert!(!order_finality_complete(&record));

    record.filled_price = Some(100.0);
    assert!(order_finality_complete(&record));

    record.state = LiveOrderState::Cancelled;
    record.filled_quantity = None;
    record.filled_price = None;
    assert!(order_finality_complete(&record));
}

#[test]
fn apply_leg_record_preserves_partial_fill_notional() {
    let mut record = order_record(LiveOrderState::PartiallyFilled);
    record.last_update_source = OrderUpdateSource::PrivateWs;
    record.exchange_order_id = Some("ex-open".into());
    record.identity = record.identity_snapshot();
    record.identity.record_venue_client_order_id("venue-open");
    record.filled_quantity = Some(0.4);
    record.filled_price = Some(101.0);
    record.filled_fee = Some(0.2);
    let mut leg = ExecutionRunLeg {
        role: HedgeLegRole::Long,
        exchange: "okx".into(),
        symbol: "BTC".into(),
        order_ids: Vec::new(),
        identity: None,
        finality_source: None,
        confirmed_filled_at_ms: None,
        state: LiveOrderState::Created,
        target_quantity: 1.0,
        filled_quantity: None,
        target_notional_usd: 100.0,
        filled_notional_usd: None,
        filled_fee: None,
    };

    assert!(apply_leg_record(&mut leg, &record).is_none());

    assert_eq!(leg.state, LiveOrderState::PartiallyFilled);
    assert_eq!(leg.filled_quantity, Some(0.4));
    assert_eq!(leg.filled_fee, Some(0.2));
    assert!(leg.order_ids.iter().any(|id| id == "open"));
    assert!(leg.order_ids.iter().any(|id| id == "open-client"));
    assert!(leg.order_ids.iter().any(|id| id == "venue-open"));
    assert!(leg.order_ids.iter().any(|id| id == "ex-open"));
    assert_eq!(
        leg.identity
            .as_ref()
            .and_then(|identity| identity.venue_client_order_id.as_deref()),
        Some("venue-open")
    );
    assert_eq!(
        leg.identity
            .as_ref()
            .and_then(|identity| identity.exchange_order_id.as_deref()),
        Some("ex-open")
    );
    assert_eq!(leg.finality_source, Some(OrderUpdateSource::PrivateWs));
    assert_eq!(leg.confirmed_filled_at_ms, None);
    assert!(leg
        .filled_notional_usd
        .is_some_and(|notional| (notional - 40.4).abs() < 1e-9));
}

#[test]
fn apply_leg_record_missing_price_returns_problem_without_zero_notional() -> Result<(), &'static str>
{
    let mut record = order_record(LiveOrderState::Filled);
    record.filled_quantity = Some(0.4);
    record.filled_price = None;
    record.intent.price = None;
    let mut leg = leg_with_fee(HedgeLegRole::Long, None);
    leg.filled_notional_usd = None;

    let problem = apply_leg_record(&mut leg, &record).ok_or("valuation problem missing")?;

    assert_eq!(problem.code, codes::HEDGE_EXECUTION_VALUATION_MISSING);
    assert_eq!(leg.filled_quantity, Some(0.4));
    assert_eq!(leg.filled_notional_usd, None);
    assert_eq!(leg.confirmed_filled_at_ms, None);
    Ok(())
}

#[test]
fn complete_dry_run_fill_is_immediately_final_for_paper_pairing() {
    let mut record = order_record(LiveOrderState::Filled);
    record.last_update_source = OrderUpdateSource::AdapterAck;
    record.filled_quantity = Some(1.0);
    record.filled_price = Some(100.0);
    record.filled_fee = Some(0.05);
    record.updated_at_ms = 42;
    let mut leg = leg_with_fee(HedgeLegRole::Long, None);
    leg.confirmed_filled_at_ms = None;

    assert!(apply_leg_record(&mut leg, &record).is_none());

    assert_eq!(leg.confirmed_filled_at_ms, Some(42));
}

#[test]
fn live_adapter_ack_does_not_bypass_external_fill_finality() {
    let mut record = order_record(LiveOrderState::Filled);
    record.intent.mode = ExecutionMode::Live;
    record.last_update_source = OrderUpdateSource::AdapterAck;
    record.filled_quantity = Some(1.0);
    record.filled_price = Some(100.0);
    record.updated_at_ms = 42;
    let mut leg = leg_with_fee(HedgeLegRole::Long, None);
    leg.confirmed_filled_at_ms = None;

    assert!(apply_leg_record(&mut leg, &record).is_none());

    assert_eq!(leg.confirmed_filled_at_ms, None);
}

#[test]
fn apply_leg_record_filled_without_quantity_does_not_infer_from_intent() -> Result<(), &'static str>
{
    let mut record = order_record(LiveOrderState::Filled);
    record.filled_quantity = None;
    record.filled_price = Some(100.0);
    let mut leg = leg_with_fee(HedgeLegRole::Long, None);
    leg.filled_quantity = None;
    leg.filled_notional_usd = None;

    let problem = apply_leg_record(&mut leg, &record).ok_or("valuation problem missing")?;

    assert_eq!(problem.code, codes::HEDGE_EXECUTION_VALUATION_MISSING);
    assert_eq!(leg.filled_quantity, None);
    assert_eq!(leg.filled_notional_usd, None);
    assert_eq!(
        problem
            .details
            .as_ref()
            .and_then(|details| details.get("reason"))
            .and_then(serde_json::Value::as_str),
        Some("missing_filled_quantity")
    );
    Ok(())
}

#[test]
fn refresh_cost_reconciliation_records_open_cost_components_after_fill_evidence(
) -> Result<(), &'static str> {
    let mut run = ExecutionRun {
        run_id: "run-1".into(),
        ticket_id: "ticket-1".into(),
        opportunity_id: "opp-1".into(),
        state: ExecutionRunState::Hedged,
        long_leg: leg_with_fee(HedgeLegRole::Long, Some(0.7)),
        short_leg: leg_with_fee(HedgeLegRole::Short, Some(0.5)),
        net_exposure_usd: 0.0,
        cost_reconciliation: Some(ExecutionCostReconciliation {
            estimated_open_cost_usd: 1.0,
            estimated_close_cost_usd: 1.5,
            estimated_slippage_usd: 0.5,
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
        status_reason: "完成".into(),
        created_at_ms: 1,
        updated_at_ms: 2,
    };

    refresh_cost_reconciliation(&mut run);

    let cost = run
        .cost_reconciliation
        .ok_or("cost reconciliation missing")?;
    assert_eq!(cost.filled_fee_usd, Some(1.2));
    assert_close_option(cost.actual_slippage_usd, 0.0)?;
    assert_close_option(cost.actual_open_cost_usd, 1.2)?;
    assert_eq!(cost.actual_cost_usd, None);
    assert_eq!(cost.cost_delta_usd, None);
    Ok(())
}
