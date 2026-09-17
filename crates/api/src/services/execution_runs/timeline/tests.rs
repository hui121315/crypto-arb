use super::*;
use shared_types::{
    ExecutionMode, MarginMode, OrderIntent, OrderSide, OrderSource, OrderType, TimeInForce,
    VenueOrderIdentity,
};

#[test]
fn timeline_is_bounded_and_reports_dropped_events() {
    let mut run = run();

    for index in 0..(EXECUTION_RUN_TIMELINE_LIMIT + 4) {
        run.updated_at_ms = index as i64 + 1;
        run.status_reason = format!("transition {index}");
        append_internal_transition(&mut run);
    }

    assert_eq!(run.evidence.events.len(), EXECUTION_RUN_TIMELINE_LIMIT);
    assert_eq!(run.evidence.dropped_event_count, 4);
    assert_eq!(
        run.evidence
            .events
            .last()
            .map(|event| event.message.as_str()),
        Some("transition 99")
    );
}

#[test]
fn private_ws_fill_promotes_leg_confidence_and_keeps_request_id() {
    let mut run = run();
    run.evidence.request_id = Some("req-run-1".to_owned());
    let record = filled_record();

    append_order_update_evidence(&mut run, &record);

    assert_eq!(
        run.evidence.long_leg.finality_confidence,
        ExecutionFillConfidence::VenueOrderSnapshot
    );
    let event = run.evidence.events.last();
    assert_eq!(
        event.map(|event| event.kind),
        Some(ExecutionRunEventKind::Fill)
    );
    assert_eq!(
        event.and_then(|event| event.leg_role),
        Some(HedgeLegRole::Long)
    );
    assert_eq!(
        event.and_then(|event| event.request_id.as_deref()),
        Some("req-run-1")
    );
    assert_eq!(
        event.map(|event| event.finality_confidence),
        Some(ExecutionFillConfidence::VenueOrderSnapshot)
    );
}

#[test]
fn missing_ticket_plans_are_recorded_as_typed_failure_evidence() {
    let mut run = run();

    initialize_evidence(&mut run, None, Some("req-ticket-plan".to_owned()));

    let failure = run
        .evidence
        .events
        .iter()
        .find(|event| event.kind == ExecutionRunEventKind::Failure);
    assert!(failure.is_some(), "ticket plan failure evidence missing");
    assert_eq!(
        failure
            .and_then(|failure| failure.problem.as_ref())
            .map(|problem| problem.code.as_str()),
        Some(codes::HEDGE_TICKET_ORDER_PLAN_EVIDENCE_INVALID)
    );
    assert_eq!(
        failure.and_then(|failure| failure.request_id.as_deref()),
        Some("req-ticket-plan")
    );
    assert!(run.evidence.long_leg.compile_plan.is_none());
    assert!(run.evidence.short_leg.compile_plan.is_none());
}

fn run() -> ExecutionRun {
    ExecutionRun {
        run_id: "run-1".to_owned(),
        ticket_id: "ticket-1".to_owned(),
        opportunity_id: "opp-1".to_owned(),
        state: ExecutionRunState::SecondLegSubmitted,
        long_leg: leg(HedgeLegRole::Long),
        short_leg: leg(HedgeLegRole::Short),
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

fn leg(role: HedgeLegRole) -> ExecutionRunLeg {
    ExecutionRunLeg {
        role,
        exchange: "okx".to_owned(),
        symbol: "BTC-USDT-SWAP".to_owned(),
        order_ids: vec![match role {
            HedgeLegRole::Long => "order-long".to_owned(),
            HedgeLegRole::Short => "order-short".to_owned(),
        }],
        identity: None,
        finality_source: None,
        confirmed_filled_at_ms: None,
        state: LiveOrderState::Accepted,
        target_quantity: 1.0,
        filled_quantity: None,
        target_notional_usd: 100.0,
        filled_notional_usd: None,
        filled_fee: None,
    }
}

fn filled_record() -> OrderRecord {
    let intent = OrderIntent {
        id: "order-long".to_owned(),
        source: OrderSource::ArbitragePreview,
        strategy: None,
        mode: ExecutionMode::Live,
        exchange: "okx".to_owned(),
        symbol: "BTC-USDT-SWAP".to_owned(),
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
        client_order_id: "client-long".to_owned(),
        client_order_id_policy: None,
        created_at_ms: 1,
    };
    OrderRecord {
        identity: VenueOrderIdentity::from_intent(&intent),
        intent,
        state: LiveOrderState::Filled,
        risk: None,
        last_update_source: OrderUpdateSource::PrivateWs,
        exchange_order_id: Some("exchange-long".to_owned()),
        message: Some("filled".to_owned()),
        filled_quantity: Some(1.0),
        filled_price: Some(100.0),
        filled_fee: Some(0.1),
        updated_at_ms: 2,
    }
}
