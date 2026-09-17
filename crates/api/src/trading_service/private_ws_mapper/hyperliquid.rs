use super::*;

mod all_dexs;
mod events;
use all_dexs::map_all_dexs_clearinghouse;
use events::{
    hyperliquid_fill_event, hyperliquid_funding_delta, hyperliquid_liquidation_delta,
    hyperliquid_non_user_cancel_delta,
};

pub(crate) fn map_hyperliquid_event(
    event: hyperliquid_ws_user::HyperliquidUserWsEvent,
) -> Vec<PrivateWsEvent> {
    match event {
        hyperliquid_ws_user::HyperliquidUserWsEvent::Order(rows) => rows
            .into_iter()
            .flat_map(|row| order_event(&row.client_order_id, row.order, row.status_timestamp_ms))
            .collect(),
        hyperliquid_ws_user::HyperliquidUserWsEvent::OpenOrders(snapshot) => {
            vec![PrivateWsEvent::OpenOrders(PrivateOpenOrdersSnapshot {
                venue: snapshot.venue,
                rows: snapshot.orders,
            })]
        }
        hyperliquid_ws_user::HyperliquidUserWsEvent::Clearinghouse(state) => {
            map_hyperliquid_clearinghouse(&state)
        }
        hyperliquid_ws_user::HyperliquidUserWsEvent::AllDexsClearinghouse(rows) => {
            map_all_dexs_clearinghouse(rows)
        }
        hyperliquid_ws_user::HyperliquidUserWsEvent::SpotState(rows) => {
            map_hyperliquid_spot_state(rows)
        }
        hyperliquid_ws_user::HyperliquidUserWsEvent::Fill(rows) => {
            rows.into_iter().map(hyperliquid_fill_event).collect()
        }
        hyperliquid_ws_user::HyperliquidUserWsEvent::Funding(rows) => rows
            .into_iter()
            .map(hyperliquid_funding_delta)
            .map(PrivateWsEvent::Funding)
            .collect(),
        hyperliquid_ws_user::HyperliquidUserWsEvent::Liquidation(row) => {
            vec![PrivateWsEvent::Liquidation(hyperliquid_liquidation_delta(
                row,
            ))]
        }
        hyperliquid_ws_user::HyperliquidUserWsEvent::NonUserCancel(rows) => rows
            .into_iter()
            .map(hyperliquid_non_user_cancel_delta)
            .map(PrivateWsEvent::NonUserCancel)
            .collect(),
    }
}

pub(super) fn order_event(
    client_order_id: &str,
    order: OrderInfo,
    received_at_ms: i64,
) -> Vec<PrivateWsEvent> {
    let trimmed = client_order_id.trim();
    if trimmed.is_empty() && order.order_id.trim().is_empty() {
        return dirty_account(
            &order.exchange,
            PrivateAccountScope::All,
            "order_update_missing_identity",
        );
    }
    vec![PrivateWsEvent::Order(PrivateOrderDelta {
        client_order_id: trimmed.to_owned(),
        order,
        received_at_ms,
    })]
}

/// PR-DP-08 D-7 (Hyperliquid clearinghouse): Hyperliquid 的 `clearinghouseState`
/// channel 每帧都是 fresh state-of-the-world full snapshot（含 `marginSummary
/// {accountValue, totalMarginUsed}` + `withdrawable` + 全部 `assetPositions`），
/// **不存在 incremental delta 概念**（OKX/Bybit/Bitget 用 `snapshot/delta` 字段
/// 区分，HL 不需要）。文档：
/// <https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/websocket/subscriptions>
///
/// 系统读路径 `get_balance` 走 `parse_perp_balance` (USDC perp 单行)，因此每个
/// clearinghouse 帧会映射 balance snapshot。position snapshot 只有空仓时才
/// 直接替换为空表；非空仓位缺少官方 markPx 字段，不能写入 `mark_price=0`。
/// 非空仓位不能写入伪造 mark price，因此发出带 dex venue 的 scoped
/// `AccountDirty(positions)`；目标 cache 立即变 stale，后续读取触发有界 REST refresh，
/// 其他 dex cache 保持 fresh。
pub(super) fn map_hyperliquid_clearinghouse(
    state: &hyperliquid_ws_user::HyperliquidClearinghouseState,
) -> Vec<PrivateWsEvent> {
    let venue = hyperliquid_clearinghouse_venue(state.dex.as_deref());
    let balance = hyperliquid_balance(&venue, state);
    vec![
        hyperliquid_position_event(&venue, &state.positions),
        PrivateWsEvent::Balances(Box::new(PrivateBalancesSnapshot {
            venue,
            rows: vec![balance],
        })),
    ]
}

pub(super) fn map_hyperliquid_dex_clearinghouse(
    row: hyperliquid_ws_user::HyperliquidDexClearinghouseState,
) -> Vec<PrivateWsEvent> {
    let mut state = row.state;
    if missing_hyperliquid_dex(state.dex.as_deref()) {
        state.dex = Some(row.dex);
    }
    map_hyperliquid_clearinghouse(&state)
}

pub(super) fn missing_hyperliquid_dex(dex: Option<&str>) -> bool {
    match dex {
        Some(value) => value.trim().is_empty(),
        None => true,
    }
}

pub(super) fn hyperliquid_clearinghouse_venue(dex: Option<&str>) -> String {
    let normalized = dex.map(str::trim).filter(|dex| !dex.is_empty());
    normalized
        .map(|dex| format!("hyperliquid:{dex}"))
        .unwrap_or_else(|| "hyperliquid".to_owned())
}

pub(super) fn hyperliquid_balance(
    venue: &str,
    state: &hyperliquid_ws_user::HyperliquidClearinghouseState,
) -> VenueBalanceInfo {
    // 复刻 REST `parse_perp_balance` (hyperliquid_private_data.rs) 的字段映射，
    // 保持 WS / REST 完全一致。注意 `unrealized_pnl` 是把 `assetPositions` 全部
    // `unrealizedPnl` 累加，并跳过 NaN/Inf（与 REST `.filter(|v| v.is_finite())`
    // 同语义），避免单仓 NaN 污染整个账户。
    let unrealized_pnl: f64 = state
        .positions
        .iter()
        .map(|position| position.unrealized_pnl)
        .filter(|value| value.is_finite())
        .sum();
    VenueBalanceInfo {
        venue: venue.to_owned(),
        currency: "USDC".to_owned(),
        total: state.account_value,
        available: state.withdrawable,
        frozen: state.total_margin_used,
        unrealized_pnl,
    }
}

pub(super) fn hyperliquid_position_event(
    venue: &str,
    positions: &[hyperliquid_ws_user::HyperliquidPositionDelta],
) -> PrivateWsEvent {
    if positions.is_empty() {
        return PrivateWsEvent::Positions(PrivatePositionsSnapshot {
            venue: venue.to_owned(),
            rows: Vec::new(),
        });
    }
    tracing::warn!(
        venue = %venue,
        positions = positions.len(),
        "hyperliquid clearinghouseState has positions but no markPx; preserving other dex caches"
    );
    PrivateWsEvent::AccountDirty(PrivateAccountDirty::new(
        venue,
        PrivateAccountScope::Positions,
        "clearinghouse_positions_missing_mark_price",
    ))
}

pub(super) fn map_hyperliquid_spot_state(
    rows: Vec<hyperliquid_ws_user::HyperliquidSpotBalance>,
) -> Vec<PrivateWsEvent> {
    vec![PrivateWsEvent::Balances(Box::new(
        PrivateBalancesSnapshot {
            venue: "hyperliquid:spot".to_owned(),
            rows: rows.into_iter().map(hyperliquid_spot_balance).collect(),
        },
    ))]
}

pub(super) fn hyperliquid_spot_balance(
    row: hyperliquid_ws_user::HyperliquidSpotBalance,
) -> VenueBalanceInfo {
    VenueBalanceInfo {
        venue: "hyperliquid:spot".to_owned(),
        currency: row.coin,
        total: row.total,
        available: (row.total - row.hold).max(0.0),
        frozen: row.hold,
        unrealized_pnl: 0.0,
    }
}
