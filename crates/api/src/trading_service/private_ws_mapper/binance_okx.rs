use super::*;

mod okx_account;

use okx_account::map_okx_account_update;

pub(crate) fn map_binance_event(event: binance_ws_user::BinanceUserEvent) -> Vec<PrivateWsEvent> {
    match event {
        binance_ws_user::BinanceUserEvent::Order(row) => binance_order_events(*row),
        binance_ws_user::BinanceUserEvent::Account(update) => map_binance_account_update(update),
    }
}

fn map_binance_account_update(
    update: binance_ws_user::BinanceAccountUpdate,
) -> Vec<PrivateWsEvent> {
    let has_balances = !update.balances.is_empty();
    let has_positions = !update.positions.is_empty();
    let mut events = Vec::with_capacity(2);

    if has_positions {
        let rows = update
            .positions
            .into_iter()
            .flat_map(binance_position_rows)
            .collect();
        events.push(PrivateWsEvent::PositionPatch(PrivatePositionsPatch {
            venue: "binance".to_owned(),
            rows,
        }));
    }

    // ACCOUNT_UPDATE exposes wallet and cross-wallet balances, but not the
    // available/frozen split required by VenueBalanceInfo. Keep that surface
    // fail-closed and refresh only balances through the bounded REST path.
    if has_balances {
        events.extend(dirty_account(
            "binance",
            PrivateAccountScope::Balances,
            "account_balance_delta_requires_rest_refresh",
        ));
    }

    if events.is_empty() {
        return dirty_account(
            "binance",
            PrivateAccountScope::All,
            "account_update_without_rows",
        );
    }
    events
}

fn binance_position_rows(row: binance_ws_user::BinancePositionDelta) -> Vec<PositionInfo> {
    let signed_quantity = row.quantity;
    let venue_side = row.side.to_ascii_uppercase();
    let margin_mode = non_empty_text(&row.margin_type).map(|value| value.to_ascii_lowercase());
    let base = PositionInfo {
        symbol: row.symbol,
        exchange: "binance".to_owned(),
        side: String::new(),
        quantity: signed_quantity.abs(),
        entry_price: row.entry_price,
        mark_price: 0.0,
        unrealized_pnl: row.unrealized_pnl,
        leverage: 0.0,
        liquidation_price: None,
        liquidation_distance_pct: None,
        next_funding_ms: None,
        paired_with: None,
        margin: row.isolated_wallet,
        maintenance_margin_ratio: 0.0,
        position_mode: Some(if venue_side == "BOTH" {
            "one_way".to_owned()
        } else {
            "hedge".to_owned()
        }),
        margin_mode,
        risk_rate: None,
        available_position: None,
        frozen_position: None,
    };

    match venue_side.as_str() {
        "LONG" => vec![position_with_side(base, "long", signed_quantity.abs())],
        "SHORT" => vec![position_with_side(base, "short", signed_quantity.abs())],
        "BOTH" if signed_quantity > 0.0 => vec![
            position_with_side(base.clone(), "long", signed_quantity),
            position_with_side(base, "short", 0.0),
        ],
        "BOTH" if signed_quantity < 0.0 => vec![
            position_with_side(base.clone(), "long", 0.0),
            position_with_side(base, "short", signed_quantity.abs()),
        ],
        "BOTH" => vec![
            position_with_side(base.clone(), "long", 0.0),
            position_with_side(base, "short", 0.0),
        ],
        _ => Vec::new(),
    }
}

fn position_with_side(mut row: PositionInfo, side: &str, quantity: f64) -> PositionInfo {
    row.side = side.to_owned();
    row.quantity = quantity;
    row
}

pub(crate) fn map_okx_event(event: okx_ws_user::OkxUserEvent) -> Vec<PrivateWsEvent> {
    match event {
        okx_ws_user::OkxUserEvent::Order(rows) => {
            rows.into_iter().flat_map(okx_order_events).collect()
        }
        okx_ws_user::OkxUserEvent::Position(update) => map_okx_position_update(update),
        okx_ws_user::OkxUserEvent::Account(update) => map_okx_account_update(update),
    }
}

pub(super) fn binance_order_events(
    row: binance_ws_user::BinanceOrderTradeUpdate,
) -> Vec<PrivateWsEvent> {
    let fill = binance_fill_delta(&row);
    let terminal = row.is_terminal();
    let client_order_id = row.client_order_id.trim();
    if client_order_id.is_empty() {
        return dirty_account(
            "binance",
            PrivateAccountScope::All,
            "order_update_missing_client_identity",
        );
    }
    vec![PrivateWsEvent::BinanceOrderTrade(Box::new(
        BinanceOrderTradeDelta {
            order: PrivateOrderDelta {
                client_order_id: client_order_id.to_owned(),
                order: row.order,
                received_at_ms: row.transaction_time_ms.max(row.event_time_ms),
            },
            fill,
            execution_type: row.execution_type,
            order_status: row.order_status,
            reject_reason: row.reject_reason,
            terminal,
        },
    ))]
}

pub(super) fn binance_fill_delta(
    row: &binance_ws_user::BinanceOrderTradeUpdate,
) -> Option<PrivateFillDelta> {
    if !row.execution_type.eq_ignore_ascii_case("TRADE")
        || !valid_fill_value(row.last_filled_quantity)
        || !valid_fill_value(row.last_filled_price)
    {
        return None;
    }
    let trade_id = row.trade_id?;
    let order_id = row.order.order_id.trim();
    if order_id.is_empty() {
        return None;
    }
    Some(PrivateFillDelta {
        venue: "binance".to_owned(),
        exchange_order_id: order_id.to_owned(),
        client_order_id: non_empty_text(&row.client_order_id),
        symbol: non_empty_text(&row.order.symbol),
        side: Some(row.order.side),
        venue_event_id: format!("binance_trade:{order_id}:{trade_id}"),
        quantity: row.last_filled_quantity,
        price: row.last_filled_price,
        fee_amount: Some(row.order.fees),
        fee_currency: row.commission_asset.clone(),
        occurred_at_ms: row.trade_time_ms,
    })
}

pub(super) fn okx_order_events(row: okx_ws_user::OkxOrderUpdate) -> Vec<PrivateWsEvent> {
    let fill = okx_fill_delta(&row);
    let mut events = order_event(&row.client_order_id, row.order, row.updated_time_ms);
    if let Some(fill) = fill {
        events.push(PrivateWsEvent::Fill(fill));
    }
    events
}

pub(super) fn okx_fill_delta(row: &okx_ws_user::OkxOrderUpdate) -> Option<PrivateFillDelta> {
    let fill = row.fill.as_ref()?;
    let order_id = row.order.order_id.trim();
    if order_id.is_empty()
        || !valid_fill_value(fill.fill_size)
        || !valid_fill_value(fill.fill_price)
    {
        return None;
    }
    Some(PrivateFillDelta {
        venue: "okx".to_owned(),
        exchange_order_id: order_id.to_owned(),
        client_order_id: non_empty_text(&row.client_order_id),
        symbol: non_empty_text(&row.order.symbol),
        side: Some(row.order.side),
        venue_event_id: format!("okx_trade:{order_id}:{}", fill.trade_id),
        quantity: fill.fill_size,
        price: fill.fill_price,
        fee_amount: fill.fill_fee,
        fee_currency: fill.fill_fee_currency.clone(),
        occurred_at_ms: fill.fill_time_ms,
    })
}

/// PR-DP-08 D-1 (OKX positions): subscribe 后 OKX 先推 `eventType: "snapshot"`
/// 全量（可能分页，最后一页 `lastPage: true`），随后是
/// `eventType: "regular update"` incremental delta。文档：
/// <https://www.okx.com/docs-v5/en/#trading-account-websocket-positions-channel>
///
/// 我们只在 snapshot **完整结束**（`last_page == true`）时全量替换 position
/// cache，其余 case（incremental delta、分页中间帧）仍走 `AccountDirty` 触发
/// 下一次 read 全量 REST 拉取。这样保留既有 dirty 全量重拉的安全路径，又把
/// snapshot 路径上的一次 REST round-trip 省下。
pub(super) fn map_okx_position_update(
    update: okx_ws_user::OkxPositionUpdate,
) -> Vec<PrivateWsEvent> {
    if update.event_type == "snapshot" && update.last_page {
        vec![PrivateWsEvent::Positions(PrivatePositionsSnapshot {
            venue: "okx".to_owned(),
            rows: update.positions.into_iter().map(okx_position).collect(),
        })]
    } else {
        dirty_account(
            "okx",
            PrivateAccountScope::Positions,
            "position_snapshot_incomplete_or_incremental",
        )
    }
}

pub(super) fn okx_position(row: okx_ws_user::OkxPositionDelta) -> PositionInfo {
    PositionInfo {
        symbol: row.symbol,
        exchange: "okx".to_owned(),
        side: row.side,
        quantity: row.quantity,
        entry_price: row.entry_price,
        mark_price: row.mark_price,
        unrealized_pnl: row.unrealized_pnl,
        leverage: row.leverage,
        liquidation_price: row.liquidation_price,
        liquidation_distance_pct: None,
        next_funding_ms: None,
        paired_with: None,
        margin: row.margin,
        maintenance_margin_ratio: row.maintenance_margin_ratio,
        position_mode: None,
        margin_mode: None,
        risk_rate: None,
        available_position: None,
        frozen_position: None,
    }
}
