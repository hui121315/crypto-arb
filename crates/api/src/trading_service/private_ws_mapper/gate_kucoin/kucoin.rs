use super::*;

pub(crate) fn map_kucoin_event(event: kucoin_ws_user::KucoinUserEvent) -> Vec<PrivateWsEvent> {
    match event {
        kucoin_ws_user::KucoinUserEvent::Order(row) => kucoin_order_event(*row),
        kucoin_ws_user::KucoinUserEvent::Balance(row) => map_kucoin_balance_update(row),
        kucoin_ws_user::KucoinUserEvent::Position(row) => map_kucoin_position_update(&row),
    }
}

fn kucoin_order_event(mut row: kucoin_ws_user::KucoinOrderUpdate) -> Vec<PrivateWsEvent> {
    let client_order_id = row.client_order_id.trim().to_owned();
    if client_order_id.is_empty() && row.order.order_id.trim().is_empty() {
        return dirty_account(
            "kucoin",
            PrivateAccountScope::All,
            "order_update_missing_identity",
        );
    }
    if !row.terminal {
        return dirty_account(
            "kucoin",
            PrivateAccountScope::All,
            "contract_multiplier_requires_order_query",
        );
    }
    // Classic futures order pushes publish contract counts and omit the
    // official multiplier. Keep terminal identity/status evidence only; the
    // signed order query owns base quantity, fills and fees.
    row.order.quantity = 0.0;
    row.order.filled_quantity = 0.0;
    row.order.fees = 0.0;
    vec![PrivateWsEvent::Order(PrivateOrderDelta {
        client_order_id,
        order: row.order,
        received_at_ms: row.event_time_ms,
    })]
}

/// KuCoin classic futures wallet pushes only `walletBalance.change` as a full
/// current balance row. Older partial subjects such as `availableBalance.change`
/// are left dirty because they do not carry enough fields for margin checks.
fn map_kucoin_balance_update(row: kucoin_ws_user::KucoinBalanceDelta) -> Vec<PrivateWsEvent> {
    if row.subject != "walletBalance.change" {
        return dirty_account(
            "kucoin",
            PrivateAccountScope::Balances,
            "partial_balance_subject",
        );
    }
    balance_patch("kucoin", vec![kucoin_balance(row)])
}

fn kucoin_balance(row: kucoin_ws_user::KucoinBalanceDelta) -> VenueBalanceInfo {
    VenueBalanceInfo {
        venue: "kucoin".to_owned(),
        currency: row.currency,
        total: row.total,
        available: row.available,
        frozen: row.hold_balance,
        unrealized_pnl: row.unrealized_pnl,
    }
}

/// `/contract/positionAll` is a change notification, not a guaranteed full
/// position snapshot. Invalidate the affected account scope and let the
/// metadata-backed signed read restore base units and full risk fields.
fn map_kucoin_position_update(row: &kucoin_ws_user::KucoinPositionDelta) -> Vec<PrivateWsEvent> {
    match row {
        kucoin_ws_user::KucoinPositionDelta::Change { .. } => dirty_account(
            "kucoin",
            PrivateAccountScope::Positions,
            "contract_multiplier_requires_rest_refresh",
        ),
        kucoin_ws_user::KucoinPositionDelta::Settlement { .. } => dirty_account(
            "kucoin",
            PrivateAccountScope::All,
            "funding_settlement_requires_account_refresh",
        ),
        kucoin_ws_user::KucoinPositionDelta::RiskLimitAdjustment { success: true } => {
            dirty_account(
                "kucoin",
                PrivateAccountScope::Positions,
                "risk_limit_adjustment_requires_rest_refresh",
            )
        }
        kucoin_ws_user::KucoinPositionDelta::RiskLimitAdjustment { success: false } => Vec::new(),
    }
}
