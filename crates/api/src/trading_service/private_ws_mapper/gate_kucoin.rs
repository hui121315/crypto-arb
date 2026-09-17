use super::*;

mod kucoin;

pub(crate) use kucoin::map_kucoin_event;

pub(crate) fn map_gate_event(event: gate_ws_user::GateUserEvent) -> Vec<PrivateWsEvent> {
    match event {
        gate_ws_user::GateUserEvent::Order(rows) => {
            rows.into_iter().flat_map(gate_order_event).collect()
        }
        gate_ws_user::GateUserEvent::Position(rows) => map_gate_position_update(&rows),
        gate_ws_user::GateUserEvent::Balance(_) => dirty_account(
            "gate",
            PrivateAccountScope::Balances,
            "balance_event_requires_rest_refresh",
        ),
        gate_ws_user::GateUserEvent::UserTrade(rows) if rows.is_empty() => Vec::new(),
        gate_ws_user::GateUserEvent::UserTrade(_) => dirty_account(
            "gate",
            PrivateAccountScope::All,
            "contract_multiplier_requires_order_query",
        ),
    }
}

fn gate_order_event(mut row: gate_ws_user::GateOrderUpdate) -> Vec<PrivateWsEvent> {
    let received_at_ms = row.order.created_at.timestamp_millis();
    let client_order_id = row.client_order_id.trim().to_owned();
    if client_order_id.is_empty() && row.order.order_id.trim().is_empty() {
        return dirty_account(
            "gate",
            PrivateAccountScope::All,
            "order_update_missing_identity",
        );
    }
    if !matches!(
        row.live_state,
        shared_types::LiveOrderState::Filled
            | shared_types::LiveOrderState::Cancelled
            | shared_types::LiveOrderState::Rejected
            | shared_types::LiveOrderState::Failed
    ) {
        return dirty_account(
            "gate",
            PrivateAccountScope::All,
            "contract_multiplier_requires_order_query",
        );
    }
    // Gate private WS quantities are contract counts. Preserve terminal
    // identity/status evidence, but let the metadata-backed order query fill
    // base quantity and fees instead of projecting lots as coins.
    row.order.quantity = 0.0;
    row.order.filled_quantity = 0.0;
    row.order.fees = 0.0;
    vec![PrivateWsEvent::Order(PrivateOrderDelta {
        client_order_id,
        order: row.order,
        received_at_ms,
    })]
}

/// Gate `futures.positions` reports contract counts but not each contract's
/// official multiplier. Normalize through the adapter's metadata-backed REST
/// read instead of writing raw contract counts into the base-quantity cache.
pub(super) fn map_gate_position_update(
    rows: &[gate_ws_user::GatePositionDelta],
) -> Vec<PrivateWsEvent> {
    if rows.is_empty() {
        return Vec::new();
    }
    dirty_account(
        "gate",
        PrivateAccountScope::Positions,
        "contract_multiplier_requires_rest_refresh",
    )
}

pub(super) fn balance_patch(venue: &str, rows: Vec<VenueBalanceInfo>) -> Vec<PrivateWsEvent> {
    if rows.is_empty() {
        Vec::new()
    } else {
        vec![PrivateWsEvent::BalancePatch(Box::new(
            PrivateBalancesPatch {
                venue: venue.to_owned(),
                rows,
            },
        ))]
    }
}
