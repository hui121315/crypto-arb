use super::*;

pub(super) fn linked_event(
    mut event: ExecutionLedgerEvent,
    run_id: &str,
    ticket_id: &str,
) -> ExecutionLedgerEvent {
    event.order.run_id = Some(run_id.to_owned());
    event.order.ticket_id = Some(ticket_id.to_owned());
    event
}

pub(super) fn close_run_with_cost(
    id: &str,
    run_id: &str,
    ticket_id: &str,
    cost_reconciliation: CloseRunCostReconciliation,
) -> CloseRun {
    CloseRun {
        id: id.to_owned(),
        scope: CloseRunScope::Pair,
        status: CloseRunStatus::Compensated,
        action_run_id: None,
        request_id: None,
        idempotency_key: None,
        snapshot_version: "snapshot-1".to_owned(),
        expected_leg_count: 1,
        reason: None,
        legs: vec![CloseLeg {
            venue: "mock".to_owned(),
            symbol: "BTC".to_owned(),
            side: PositionSide::Long,
            status: CloseLegStatus::Filled,
            quantity: 1.0,
            mark_price: 100.0,
            notional_usd: 100.0,
            order: None,
            finality_source: Some(OrderUpdateSource::PrivateWs),
            confirmed_filled_at_ms: Some(1_000),
            problem: None,
            pair_evidence: Some(PositionPairEvidence {
                source: PositionPairEvidenceSource::ExecutionRun,
                run_id: run_id.to_owned(),
                ticket_id: ticket_id.to_owned(),
                opportunity_id: "opp-1".to_owned(),
                venue: "mock".to_owned(),
                symbol: "BTC".to_owned(),
                side: PositionSide::Long,
                partner_venue: "okx".to_owned(),
                partner_symbol: "BTC".to_owned(),
                partner_side: PositionSide::Short,
                leg_filled_quantity: 1.0,
                partner_filled_quantity: 1.0,
                matched_notional_usd: 100.0,
                updated_at_ms: 1_000,
            }),
            cost_events: Vec::new(),
        }],
        submitted_order_count: 1,
        failed_leg_count: 0,
        naked_exposure_usd: 0.0,
        message: "compensated".to_owned(),
        problem: None,
        finality_problem: None,
        finality_checked_at_ms: Some(1_000),
        unwind_plan: None,
        cost_events: Vec::new(),
        cost_reconciliation: Some(cost_reconciliation),
        started_at_ms: 1_000,
        updated_at_ms: 1_000,
    }
}

pub(super) fn successful_pair_close_run(
    id: &str,
    run_id: &str,
    ticket_id: &str,
    long_close_price: f64,
    short_close_price: f64,
    cost_reconciliation: CloseRunCostReconciliation,
) -> CloseRun {
    CloseRun {
        id: id.to_owned(),
        scope: CloseRunScope::Pair,
        status: CloseRunStatus::Succeeded,
        action_run_id: None,
        request_id: None,
        idempotency_key: None,
        snapshot_version: "snapshot-1".to_owned(),
        expected_leg_count: 2,
        reason: None,
        legs: vec![
            filled_close_leg(
                "close-long",
                PositionSide::Long,
                OrderSide::Sell,
                long_close_price,
                3_000,
                run_id,
                ticket_id,
            ),
            filled_close_leg(
                "close-short",
                PositionSide::Short,
                OrderSide::Buy,
                short_close_price,
                4_000,
                run_id,
                ticket_id,
            ),
        ],
        submitted_order_count: 2,
        failed_leg_count: 0,
        naked_exposure_usd: 0.0,
        message: "closed".to_owned(),
        problem: None,
        finality_problem: None,
        finality_checked_at_ms: Some(4_000),
        unwind_plan: None,
        cost_events: Vec::new(),
        cost_reconciliation: Some(cost_reconciliation),
        started_at_ms: 3_000,
        updated_at_ms: 4_000,
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn filled_close_leg(
    id: &str,
    side: PositionSide,
    order_side: OrderSide,
    price: f64,
    filled_at_ms: i64,
    run_id: &str,
    ticket_id: &str,
) -> CloseLeg {
    let mut order = order_with_price(id, order_side, Some(price));
    order.intent.source = OrderSource::Manual;
    order.intent.reduce_only = true;
    order.intent.created_at_ms = filled_at_ms.saturating_sub(10);
    order.filled_quantity = Some(1.0);
    order.filled_price = Some(price);
    order.updated_at_ms = filled_at_ms;
    CloseLeg {
        venue: "mock".to_owned(),
        symbol: "BTC".to_owned(),
        side,
        status: CloseLegStatus::Filled,
        quantity: 1.0,
        mark_price: price,
        notional_usd: price,
        order: Some(order),
        finality_source: Some(OrderUpdateSource::PrivateWs),
        confirmed_filled_at_ms: Some(filled_at_ms),
        problem: None,
        pair_evidence: Some(PositionPairEvidence {
            source: PositionPairEvidenceSource::ExecutionRun,
            run_id: run_id.to_owned(),
            ticket_id: ticket_id.to_owned(),
            opportunity_id: "opp-1".to_owned(),
            venue: "mock".to_owned(),
            symbol: "BTC".to_owned(),
            side,
            partner_venue: "mock-partner".to_owned(),
            partner_symbol: "BTC".to_owned(),
            partner_side: match side {
                PositionSide::Long => PositionSide::Short,
                PositionSide::Short => PositionSide::Long,
            },
            leg_filled_quantity: 1.0,
            partner_filled_quantity: 1.0,
            matched_notional_usd: 100.0,
            updated_at_ms: filled_at_ms,
        }),
        cost_events: Vec::new(),
    }
}
