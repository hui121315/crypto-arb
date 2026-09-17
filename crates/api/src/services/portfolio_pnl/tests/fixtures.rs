use super::*;
use shared_types::{
    CloseLeg, CloseLegStatus, CloseRun, CloseRunCostReconciliation, CloseRunScope, CloseRunStatus,
    ExecutionLedgerEvent, ExecutionLedgerEventType, ExecutionLedgerOrderRef,
    ExecutionLedgerPayload, ExecutionLedgerQuality, ExecutionMode, FeeLedgerSnapshot,
    FillLedgerSnapshot, FundingPaymentLedgerRecord, LiveOrderState, OrderIntent, OrderRecord,
    OrderSide, OrderSource, OrderType, OrderUpdateSource, PositionPairEvidence,
    PositionPairEvidenceSource, PositionSide, StrategyKind, VenueOrderIdentity,
};

pub(super) fn accepted_order(id: &str, side: OrderSide, created_at_ms: i64) -> OrderRecord {
    let intent = OrderIntent {
        id: id.into(),
        source: OrderSource::ArbitragePreview,
        strategy: Some(StrategyKind::PerpCross),
        mode: ExecutionMode::DryRun,
        exchange: "okx".into(),
        symbol: "BTC".into(),
        side,
        order_type: OrderType::Limit,
        quantity: 1.0,
        price: Some(100.0),
        slippage_tolerance_bps: None,
        reduce_only: false,
        time_in_force: shared_types::TimeInForce::Ioc,
        post_only: false,
        margin_mode: shared_types::MarginMode::Cross,
        leverage: 1.0,
        client_order_id: format!("{id}-client"),
        client_order_id_policy: None,
        created_at_ms,
    };
    OrderRecord {
        identity: VenueOrderIdentity::from_intent(&intent),
        intent,
        state: LiveOrderState::Filled,
        risk: None,
        last_update_source: OrderUpdateSource::PrivateWs,
        exchange_order_id: None,
        message: None,
        filled_quantity: None,
        filled_price: None,
        filled_fee: None,
        updated_at_ms: created_at_ms,
    }
}

pub(super) fn fill_event(
    order: &OrderRecord,
    price: f64,
    occurred_at_ms: i64,
) -> ExecutionLedgerEvent {
    fill_event_with_fee(
        order,
        price,
        0.0,
        occurred_at_ms,
        &format!("fill-{}", order.intent.id),
    )
}

pub(super) fn fill_event_with_id(
    order: &OrderRecord,
    price: f64,
    occurred_at_ms: i64,
    event_id: &str,
) -> ExecutionLedgerEvent {
    fill_event_with_fee(order, price, 0.0, occurred_at_ms, event_id)
}

pub(super) fn fill_event_with_fee(
    order: &OrderRecord,
    price: f64,
    fee: f64,
    occurred_at_ms: i64,
    event_id: &str,
) -> ExecutionLedgerEvent {
    ExecutionLedgerEvent {
        event_id: event_id.into(),
        event_type: ExecutionLedgerEventType::FillSnapshot,
        source: OrderUpdateSource::PrivateWs,
        order: ExecutionLedgerOrderRef {
            run_id: None,
            ticket_id: None,
            leg_role: None,
            reduce_only: None,
            exchange: order.intent.exchange.clone(),
            symbol: order.intent.symbol.clone(),
            side: order.intent.side,
            identity: order.identity_snapshot(),
        },
        payload: ExecutionLedgerPayload::FillSnapshot(FillLedgerSnapshot {
            quantity: 1.0,
            average_price: price,
            quote_value: price,
            quality: ExecutionLedgerQuality::Actual,
            confidence: shared_types::ExecutionFillConfidence::VenueOrderSnapshot,
            fee: (fee.abs() > f64::EPSILON).then_some(FeeLedgerSnapshot {
                amount: fee,
                currency: Some("USDT".into()),
                quality: ExecutionLedgerQuality::Actual,
            }),
        }),
        occurred_at_ms,
        captured_at_ms: occurred_at_ms,
    }
}

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
    close_fee_usd: f64,
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
            venue: "okx".to_owned(),
            symbol: "BTC".to_owned(),
            side: PositionSide::Long,
            status: CloseLegStatus::Filled,
            quantity: 1.0,
            mark_price: 100.0,
            notional_usd: 100.0,
            order: None,
            finality_source: Some(OrderUpdateSource::PrivateWs),
            confirmed_filled_at_ms: Some(DAY_MS + 3_000),
            problem: None,
            pair_evidence: Some(PositionPairEvidence {
                source: PositionPairEvidenceSource::ExecutionRun,
                run_id: run_id.to_owned(),
                ticket_id: ticket_id.to_owned(),
                opportunity_id: "opp-1".to_owned(),
                venue: "okx".to_owned(),
                symbol: "BTC".to_owned(),
                side: PositionSide::Long,
                partner_venue: "binance".to_owned(),
                partner_symbol: "BTC".to_owned(),
                partner_side: PositionSide::Short,
                leg_filled_quantity: 1.0,
                partner_filled_quantity: 1.0,
                matched_notional_usd: 100.0,
                updated_at_ms: DAY_MS + 3_000,
            }),
            cost_events: Vec::new(),
        }],
        submitted_order_count: 1,
        failed_leg_count: 0,
        naked_exposure_usd: 0.0,
        message: "compensated".to_owned(),
        problem: None,
        finality_problem: None,
        finality_checked_at_ms: Some(DAY_MS + 3_000),
        unwind_plan: None,
        cost_events: Vec::new(),
        cost_reconciliation: Some(CloseRunCostReconciliation {
            close_fee_usd: Some(close_fee_usd),
            total_actual_cost_usd: Some(close_fee_usd),
            evidence_event_ids: vec![format!("{id}-close-fee")],
            close_fee_event_ids: vec![format!("{id}-close-fee")],
            ..CloseRunCostReconciliation::default()
        }),
        started_at_ms: DAY_MS + 3_000,
        updated_at_ms: DAY_MS + 3_000,
    }
}

pub(super) fn funding_event(
    order: &OrderRecord,
    amount: f64,
    occurred_at_ms: i64,
    event_id: &str,
) -> ExecutionLedgerEvent {
    ExecutionLedgerEvent {
        event_id: event_id.into(),
        event_type: ExecutionLedgerEventType::FundingPayment,
        source: OrderUpdateSource::PrivateWs,
        order: ExecutionLedgerOrderRef {
            run_id: None,
            ticket_id: None,
            leg_role: None,
            reduce_only: None,
            exchange: order.intent.exchange.clone(),
            symbol: order.intent.symbol.clone(),
            side: order.intent.side,
            identity: order.identity_snapshot(),
        },
        payload: ExecutionLedgerPayload::FundingPayment(FundingPaymentLedgerRecord {
            amount,
            currency: "USDT".into(),
            funding_time_ms: occurred_at_ms,
            quality: ExecutionLedgerQuality::Actual,
        }),
        occurred_at_ms,
        captured_at_ms: occurred_at_ms,
    }
}
