use shared_types::{LiveOrderState, OrderInfo, OrderRecord, OrderStatus};
use std::collections::{BTreeMap, BTreeSet};

pub use shared_types::{
    OrderReconcileDiff as ReconcileDiff, OrderReconcileDiffKind as ReconcileDiffKind,
};

pub fn diff_orders(local: &[OrderRecord], remote: &[OrderInfo]) -> Vec<ReconcileDiff> {
    let mut local_by_exchange_id: BTreeMap<String, &OrderRecord> = BTreeMap::new();
    // lost-ack 记录：提交后 ack 丢失（HTTP 超时）的本地 open 单没有
    // exchange_order_id——这正是最需要对账的一类，此前被直接丢弃：本地不报
    // RemoteMissing，远端同一订单以 internal_order_id=None 的 LocalMissing 出现，
    // 修复循环永远无法认领，可能长期双份持仓或裸腿。
    let mut lost_ack: Vec<&OrderRecord> = Vec::new();
    for record in local
        .iter()
        .filter(|record| is_open_live_state(record.state))
    {
        match record.exchange_order_id.as_ref() {
            Some(id) => {
                local_by_exchange_id.insert(id.clone(), record);
            }
            None => lost_ack.push(record),
        }
    }
    let remote_by_exchange_id: BTreeMap<String, &OrderInfo> = remote
        .iter()
        .filter(|order| is_open_order_status(order.status))
        .map(|order| (order.order_id.clone(), order))
        .collect();
    let remote_by_client_id: BTreeMap<&str, &OrderInfo> = remote_by_exchange_id
        .values()
        .filter_map(|order| {
            order
                .client_order_id
                .as_deref()
                .filter(|id| !id.is_empty())
                .map(|id| (id, *order))
        })
        .collect();

    let (mut diffs, claimed_remote_ids) = diff_lost_ack(lost_ack, &remote_by_client_id);

    let mut ids: BTreeSet<String> = BTreeSet::new();
    ids.extend(local_by_exchange_id.keys().cloned());
    ids.extend(remote_by_exchange_id.keys().cloned());

    diffs.extend(diff_indexed_orders(
        ids,
        &local_by_exchange_id,
        &remote_by_exchange_id,
        &claimed_remote_ids,
    ));
    diffs
}

fn diff_lost_ack<'a>(
    lost_ack: Vec<&OrderRecord>,
    remote_by_client_id: &BTreeMap<&'a str, &'a OrderInfo>,
) -> (Vec<ReconcileDiff>, BTreeSet<&'a str>) {
    let mut claimed_remote_ids = BTreeSet::new();
    let mut diffs = Vec::new();
    for record in lost_ack {
        match remote_by_client_id.get(record.intent.client_order_id.as_str()) {
            Some(order) => {
                claimed_remote_ids.insert(order.order_id.as_str());
                diffs.push(ReconcileDiff {
                    kind: ReconcileDiffKind::StateMismatch,
                    exchange_order_id: order.order_id.clone(),
                    internal_order_id: Some(record.intent.id.clone()),
                    local_state: Some(record.state),
                    remote_state: Some(live_state_from_status(order.status)),
                    local_quantity: Some(record.intent.quantity),
                    remote_quantity: Some(order.quantity),
                });
            }
            None => diffs.push(ReconcileDiff {
                kind: ReconcileDiffKind::RemoteMissing,
                exchange_order_id: record.intent.client_order_id.clone(),
                internal_order_id: Some(record.intent.id.clone()),
                local_state: Some(record.state),
                remote_state: None,
                local_quantity: Some(record.intent.quantity),
                remote_quantity: None,
            }),
        }
    }
    (diffs, claimed_remote_ids)
}

fn diff_indexed_orders(
    ids: BTreeSet<String>,
    local_by_exchange_id: &BTreeMap<String, &OrderRecord>,
    remote_by_exchange_id: &BTreeMap<String, &OrderInfo>,
    claimed_remote_ids: &BTreeSet<&str>,
) -> Vec<ReconcileDiff> {
    let mut diffs = Vec::new();
    for id in ids {
        if claimed_remote_ids.contains(id.as_str()) {
            continue;
        }
        match (
            local_by_exchange_id.get(&id),
            remote_by_exchange_id.get(&id),
        ) {
            (Some(local), Some(remote)) => {
                let remote_state = live_state_from_status(remote.status);
                if local.state != remote_state {
                    diffs.push(ReconcileDiff {
                        kind: ReconcileDiffKind::StateMismatch,
                        exchange_order_id: id.clone(),
                        internal_order_id: Some(local.intent.id.clone()),
                        local_state: Some(local.state),
                        remote_state: Some(remote_state),
                        local_quantity: Some(local.intent.quantity),
                        remote_quantity: Some(remote.quantity),
                    });
                }
                if (local.intent.quantity - remote.quantity).abs() > 1e-12 {
                    diffs.push(ReconcileDiff {
                        kind: ReconcileDiffKind::QuantityMismatch,
                        exchange_order_id: id,
                        internal_order_id: Some(local.intent.id.clone()),
                        local_state: Some(local.state),
                        remote_state: Some(remote_state),
                        local_quantity: Some(local.intent.quantity),
                        remote_quantity: Some(remote.quantity),
                    });
                }
            }
            (Some(local), None) => diffs.push(ReconcileDiff {
                kind: ReconcileDiffKind::RemoteMissing,
                exchange_order_id: id,
                internal_order_id: Some(local.intent.id.clone()),
                local_state: Some(local.state),
                remote_state: None,
                local_quantity: Some(local.intent.quantity),
                remote_quantity: None,
            }),
            (None, Some(remote)) => diffs.push(ReconcileDiff {
                kind: ReconcileDiffKind::LocalMissing,
                exchange_order_id: id,
                internal_order_id: None,
                local_state: None,
                remote_state: Some(live_state_from_status(remote.status)),
                local_quantity: None,
                remote_quantity: Some(remote.quantity),
            }),
            (None, None) => {}
        }
    }

    diffs
}

fn is_open_live_state(state: LiveOrderState) -> bool {
    matches!(
        state,
        LiveOrderState::Created
            | LiveOrderState::RiskChecked
            | LiveOrderState::Submitted
            | LiveOrderState::Accepted
            | LiveOrderState::PartiallyFilled
            | LiveOrderState::CancelRequested
            | LiveOrderState::Unknown
    )
}

fn is_open_order_status(status: OrderStatus) -> bool {
    matches!(
        status,
        OrderStatus::Pending | OrderStatus::Open | OrderStatus::PartiallyFilled
    )
}

fn live_state_from_status(status: OrderStatus) -> LiveOrderState {
    match status {
        OrderStatus::Pending => LiveOrderState::Accepted,
        OrderStatus::Open => LiveOrderState::Accepted,
        OrderStatus::PartiallyFilled => LiveOrderState::PartiallyFilled,
        OrderStatus::Filled => LiveOrderState::Filled,
        OrderStatus::Canceled => LiveOrderState::Cancelled,
        OrderStatus::Rejected => LiveOrderState::Rejected,
        OrderStatus::Expired => LiveOrderState::Failed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use shared_types::{
        ExecutionMode, OrderIntent, OrderSide, OrderSource, OrderType, RiskDecision,
    };

    fn local(id: &str, exchange_order_id: &str, state: LiveOrderState, qty: f64) -> OrderRecord {
        OrderRecord {
            intent: OrderIntent {
                id: id.into(),
                source: OrderSource::Manual,
                strategy: None,
                mode: ExecutionMode::Testnet,
                exchange: "mock".into(),
                symbol: "BTC".into(),
                side: OrderSide::Buy,
                order_type: OrderType::Limit,
                quantity: qty,
                price: Some(50_000.0),
                slippage_tolerance_bps: None,
                reduce_only: false,
                time_in_force: shared_types::TimeInForce::Ioc,
                post_only: false,
                margin_mode: shared_types::MarginMode::Cross,
                leverage: 1.0,
                client_order_id: format!("client-{id}"),
                client_order_id_policy: None,
                created_at_ms: 1,
            },
            state,
            risk: Some(RiskDecision::allow(qty * 50_000.0)),
            identity: Default::default(),
            last_update_source: Default::default(),
            exchange_order_id: Some(exchange_order_id.into()),
            message: None,
            filled_quantity: None,
            filled_price: None,
            filled_fee: None,
            updated_at_ms: 2,
        }
    }

    fn remote(id: &str, status: OrderStatus, qty: f64) -> OrderInfo {
        OrderInfo {
            execution_style: None,
            venue_time_in_force: None,
            client_order_id: None,
            reduce_only: None,
            order_id: id.into(),
            symbol: "BTC".into(),
            exchange: "mock".into(),
            side: OrderSide::Buy,
            order_type: OrderType::Limit,
            status,
            quantity: qty,
            price: 50_000.0,
            filled_quantity: 0.0,
            filled_price: 0.0,
            fees: 0.0,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn no_diff_when_local_and_remote_match() {
        let diffs = diff_orders(
            &[local("l1", "x1", LiveOrderState::Accepted, 0.01)],
            &[remote("x1", OrderStatus::Open, 0.01)],
        );
        assert!(diffs.is_empty());
    }

    #[test]
    fn pending_remote_order_matches_local_accepted() {
        let diffs = diff_orders(
            &[local("l1", "x1", LiveOrderState::Accepted, 0.01)],
            &[remote("x1", OrderStatus::Pending, 0.01)],
        );
        assert!(diffs.is_empty());
    }

    #[test]
    fn detects_remote_missing() {
        let diffs = diff_orders(&[local("l1", "x1", LiveOrderState::Accepted, 0.01)], &[]);
        assert_eq!(diffs[0].kind, ReconcileDiffKind::RemoteMissing);
    }

    #[test]
    fn detects_local_missing() {
        let diffs = diff_orders(&[], &[remote("x1", OrderStatus::Open, 0.01)]);
        assert_eq!(diffs[0].kind, ReconcileDiffKind::LocalMissing);
    }

    #[test]
    fn detects_state_mismatch() {
        let diffs = diff_orders(
            &[local("l1", "x1", LiveOrderState::Accepted, 0.01)],
            &[remote("x1", OrderStatus::PartiallyFilled, 0.01)],
        );
        assert_eq!(diffs[0].kind, ReconcileDiffKind::StateMismatch);
    }

    #[test]
    fn detects_quantity_mismatch() {
        let diffs = diff_orders(
            &[local("l1", "x1", LiveOrderState::Accepted, 0.01)],
            &[remote("x1", OrderStatus::Open, 0.02)],
        );
        assert_eq!(diffs[0].kind, ReconcileDiffKind::QuantityMismatch);
    }

    fn local_lost_ack(id: &str, state: LiveOrderState, qty: f64) -> OrderRecord {
        let mut record = local(id, "unused", state, qty);
        record.exchange_order_id = None;
        record
    }

    fn remote_with_client_id(
        id: &str,
        client_id: &str,
        status: OrderStatus,
        qty: f64,
    ) -> OrderInfo {
        let mut order = remote(id, status, qty);
        order.client_order_id = Some(client_id.into());
        order
    }

    #[test]
    fn lost_ack_local_matches_remote_by_client_order_id() {
        // ack 丢失的本地单（无 exchange_order_id）通过 client_order_id 关联到
        // 远端真实存在的订单：产出可被修复循环认领的 StateMismatch，
        // 且该远端订单不再被误报为 LocalMissing。
        let diffs = diff_orders(
            &[local_lost_ack("l1", LiveOrderState::Submitted, 0.01)],
            &[remote_with_client_id(
                "x9",
                "client-l1",
                OrderStatus::Open,
                0.01,
            )],
        );

        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].kind, ReconcileDiffKind::StateMismatch);
        assert_eq!(diffs[0].exchange_order_id, "x9");
        assert_eq!(diffs[0].internal_order_id.as_deref(), Some("l1"));
        assert_eq!(diffs[0].remote_state, Some(LiveOrderState::Accepted));
    }

    #[test]
    fn lost_ack_local_without_remote_match_reports_repairable_remote_missing() {
        let diffs = diff_orders(
            &[local_lost_ack("l1", LiveOrderState::Submitted, 0.01)],
            &[],
        );

        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].kind, ReconcileDiffKind::RemoteMissing);
        // 修复循环按 internal_order_id 认领；标识字段回落到 client_order_id。
        assert_eq!(diffs[0].internal_order_id.as_deref(), Some("l1"));
        assert_eq!(diffs[0].exchange_order_id, "client-l1");
    }

    #[test]
    fn ignores_terminal_local_and_remote_orders() {
        let diffs = diff_orders(
            &[local("l1", "x1", LiveOrderState::Filled, 0.01)],
            &[remote("x1", OrderStatus::Filled, 0.01)],
        );
        assert!(diffs.is_empty());
    }
}
