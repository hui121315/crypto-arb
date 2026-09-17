//! 订单模块测试夹具：被 `queue.rs` 与 `projection.rs` 的单测共享的 `OrderRecord` /
//! `ListEnvelope` / `ExecutionRun` 构造器，避免两处重复维护。

use shared_types::{
    ApiProblem, ExecutionMode, ExecutionRun, ExecutionRunLeg, ExecutionRunState, HedgeLegRole,
    ListEnvelope, ListPage, ListStatus, LiveOrderState, MarginMode, OrderIntent, OrderRecord,
    OrderSide, OrderSource, OrderType,
};

pub(in crate::panels::modules::execution::data::orders) fn order(id: &str) -> OrderRecord {
    order_at(id, 1)
}

pub(in crate::panels::modules::execution::data::orders) fn envelope(
    rows: Vec<OrderRecord>,
) -> ListEnvelope<OrderRecord> {
    envelope_with(rows, ListStatus::Fresh, Vec::new())
}

pub(in crate::panels::modules::execution::data::orders) fn envelope_with(
    rows: Vec<OrderRecord>,
    status: ListStatus,
    problems: Vec<ApiProblem>,
) -> ListEnvelope<OrderRecord> {
    let returned_count = rows.len();
    ListEnvelope::new(
        rows,
        ListPage {
            limit: 50,
            max_limit: 100,
            start_offset: 0,
            returned_count,
            total_rows: returned_count,
            has_more: false,
            next_cursor: None,
            ..ListPage::default()
        },
        status,
        "order_journal",
        10,
        problems,
    )
}

pub(in crate::panels::modules::execution::data::orders) fn order_at(
    id: &str,
    updated_at_ms: i64,
) -> OrderRecord {
    OrderRecord {
        intent: OrderIntent {
            id: id.to_owned(),
            source: OrderSource::Manual,
            strategy: None,
            mode: ExecutionMode::DryRun,
            exchange: "paper".to_owned(),
            symbol: "BTC-USDT".to_owned(),
            side: OrderSide::Buy,
            order_type: OrderType::Limit,
            quantity: 1.0,
            price: Some(1.0),
            slippage_tolerance_bps: None,
            reduce_only: false,
            time_in_force: shared_types::TimeInForce::Ioc,
            post_only: false,
            margin_mode: MarginMode::Cross,
            leverage: 1.0,
            client_order_id: id.to_owned(),
            client_order_id_policy: None,
            created_at_ms: 1,
        },
        state: shared_types::LiveOrderState::Created,
        risk: None,
        identity: Default::default(),
        last_update_source: Default::default(),
        exchange_order_id: None,
        message: None,
        filled_quantity: None,
        filled_price: None,
        filled_fee: None,
        updated_at_ms,
    }
}

pub(in crate::panels::modules::execution::data::orders) fn run_with_orders(
    long: &[&str],
    short: &[&str],
) -> ExecutionRun {
    ExecutionRun {
        run_id: "run-1".to_owned(),
        ticket_id: "ticket-1".to_owned(),
        opportunity_id: "opp-1".to_owned(),
        state: ExecutionRunState::SubmittingSecondLeg,
        long_leg: leg(HedgeLegRole::Long, long),
        short_leg: leg(HedgeLegRole::Short, short),
        net_exposure_usd: 0.0,
        cost_reconciliation: None,
        valuation_problem: None,
        unwind_problem: None,
        finality_problem: None,
        finality_checked_at_ms: None,
        evidence: Default::default(),
        recovery_action: None,
        status_reason: "test".to_owned(),
        created_at_ms: 1,
        updated_at_ms: 2,
    }
}

fn leg(role: HedgeLegRole, order_ids: &[&str]) -> ExecutionRunLeg {
    ExecutionRunLeg {
        role,
        exchange: "paper".to_owned(),
        symbol: "BTC-USDT".to_owned(),
        order_ids: order_ids.iter().map(|id| (*id).to_owned()).collect(),
        identity: None,
        finality_source: None,
        confirmed_filled_at_ms: None,
        state: LiveOrderState::Accepted,
        target_quantity: 1.0,
        filled_quantity: None,
        target_notional_usd: 1.0,
        filled_notional_usd: None,
        filled_fee: None,
    }
}
