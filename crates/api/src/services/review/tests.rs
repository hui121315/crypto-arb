use super::*;
use shared_types::{
    ExecutionLedgerEventType, ExecutionLedgerOrderRef, ExecutionLedgerPayload,
    ExecutionLedgerQuality, ExecutionMode, FeeLedgerSnapshot, FillLedgerSnapshot, HedgeLegRole,
    OrderIntent, OrderSide, OrderSource, OrderType, OrderUpdateSource, StrategyKind,
    StrategyPerformanceSampleStatus, VenueOrderIdentity,
};

mod close_run_linkage;
mod envelope;
mod funding_ingest;
mod paging;
mod sql_replay;
mod storage_health;

pub(super) fn assert_review_storage_health<T>(
    envelope: &ReviewEnvelope<T>,
    operation: &str,
    row_count: usize,
) {
    assert!(envelope.storage_health.as_ref().is_some_and(|health| {
        health.operation == operation
            && health.status == VenueOperationStatus::Warn
            && health.source == REVIEW_STORAGE_SOURCE
            && health.configured == Some(false)
            && health.rows == Some(row_count as u64)
    }));
}

pub(super) fn review_ledger_problem_detail<T>(
    envelope: &ReviewEnvelope<T>,
    key: &str,
) -> Option<serde_json::Value> {
    envelope
        .problems
        .iter()
        .find(|problem| problem.code == codes::REVIEW_LEDGER_INCOMPLETE)
        .and_then(|problem| problem.details.clone())
        .and_then(|details| details.get(key).cloned())
}

pub(super) fn order(id: &str, side: OrderSide, price: f64) -> OrderRecord {
    let now_ms = common::time::now_ms();
    order_at(id, side, price, now_ms)
}

pub(super) fn order_at(id: &str, side: OrderSide, price: f64, now_ms: i64) -> OrderRecord {
    let intent = OrderIntent {
        id: id.into(),
        source: OrderSource::ArbitragePreview,
        strategy: Some(StrategyKind::SpotPerp),
        mode: ExecutionMode::DryRun,
        exchange: "mock".into(),
        symbol: "BTC".into(),
        side,
        order_type: OrderType::Limit,
        quantity: 1.0,
        price: Some(price),
        slippage_tolerance_bps: None,
        reduce_only: false,
        time_in_force: shared_types::TimeInForce::Ioc,
        post_only: false,
        margin_mode: shared_types::MarginMode::Cross,
        leverage: 1.0,
        client_order_id: format!("{id}-client"),
        client_order_id_policy: None,
        created_at_ms: now_ms,
    };
    OrderRecord {
        identity: VenueOrderIdentity::from_intent(&intent),
        intent,
        state: LiveOrderState::Filled,
        risk: None,
        last_update_source: OrderUpdateSource::PrivateWs,
        exchange_order_id: Some(format!("{id}-x")),
        message: None,
        filled_quantity: Some(1.0),
        filled_price: Some(price),
        filled_fee: Some(0.01),
        updated_at_ms: now_ms,
    }
}

pub(super) fn fill_event(order: &OrderRecord, role: HedgeLegRole) -> ExecutionLedgerEvent {
    ExecutionLedgerEvent {
        event_id: format!("fill-{}", order.intent.id),
        event_type: ExecutionLedgerEventType::FillSnapshot,
        source: OrderUpdateSource::PrivateWs,
        order: ExecutionLedgerOrderRef {
            run_id: None,
            ticket_id: None,
            leg_role: Some(role),
            reduce_only: None,
            exchange: order.intent.exchange.clone(),
            symbol: order.intent.symbol.clone(),
            side: order.intent.side,
            identity: order.identity_snapshot(),
        },
        payload: ExecutionLedgerPayload::FillSnapshot(FillLedgerSnapshot {
            quantity: 1.0,
            average_price: order.filled_price.unwrap_or_default(),
            quote_value: order.filled_price.unwrap_or_default(),
            quality: ExecutionLedgerQuality::Actual,
            confidence: shared_types::ExecutionFillConfidence::VenueOrderSnapshot,
            fee: Some(FeeLedgerSnapshot {
                amount: 0.01,
                currency: None,
                quality: ExecutionLedgerQuality::Actual,
            }),
        }),
        occurred_at_ms: order.updated_at_ms,
        captured_at_ms: order.updated_at_ms,
    }
}

pub(super) fn missed_row(id: String, detected_at_ms: i64) -> MissedOpportunity {
    MissedOpportunity {
        opportunity_id: format!("{id}-opp"),
        id,
        strategy: StrategyKind::PerpCross,
        symbol: "BTC".into(),
        detected_at_ms,
        expected_pnl_usd: 1.0,
        reason: shared_types::MissReason::ManualSkip,
        detail: "manual".into(),
    }
}
