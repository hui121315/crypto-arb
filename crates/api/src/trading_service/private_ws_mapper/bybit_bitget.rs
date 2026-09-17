use super::*;

mod bitget;

pub(crate) use bitget::map_bitget_event;

pub(crate) fn map_bybit_event(event: bybit_ws_user::BybitUserEvent) -> Vec<PrivateWsEvent> {
    match event {
        bybit_ws_user::BybitUserEvent::Order(rows) => rows
            .into_iter()
            .flat_map(|row| {
                let received_at_ms = row.order.created_at.timestamp_millis();
                order_event(&row.client_order_id, row.order, received_at_ms)
            })
            .collect(),
        bybit_ws_user::BybitUserEvent::Execution(rows) => rows
            .into_iter()
            .map(bybit_fill_delta)
            .map(PrivateWsEvent::Fill)
            .collect(),
        bybit_ws_user::BybitUserEvent::Position(update) => map_bybit_position_update(update),
        bybit_ws_user::BybitUserEvent::Wallet(update) => map_bybit_wallet_update(update),
    }
}

/// PR-DP-08 D-3 (Bybit V5 position): Bybit V5 在订阅 `position` 主题后会按
/// category 先推 `type: "snapshot"` 的全量行（每个 category 各一帧），随后
/// `type: "delta"` 增量。文档：
/// <https://bybit-exchange.github.io/docs/v5/websocket/private/position>
///
/// 系统全栈仅查 Bybit `category=linear&settleCoin=USDT`（见 `bybit::get_positions`），
/// 因此只有 `type == "snapshot"` 时整表替换 cache，并 filter 仅保留 linear 行（与
/// REST 行为对齐，避免偶发的 inverse / option 行污染）；其他 case（incremental
/// delta、空 type）仍走 `AccountDirty` 触发下一次 read 全量 REST 拉取。
pub(super) fn map_bybit_position_update(
    update: bybit_ws_user::BybitPositionUpdate,
) -> Vec<PrivateWsEvent> {
    if update.update_type == "snapshot" {
        vec![PrivateWsEvent::Positions(PrivatePositionsSnapshot {
            venue: "bybit".to_owned(),
            rows: update
                .positions
                .into_iter()
                .filter(|row| row.category == "linear")
                .map(bybit_position)
                .collect(),
        })]
    } else {
        dirty_account(
            "bybit",
            PrivateAccountScope::Positions,
            "position_delta_requires_rest_refresh",
        )
    }
}

pub(super) fn bybit_position(row: bybit_ws_user::BybitPositionDelta) -> PositionInfo {
    PositionInfo {
        symbol: row.symbol,
        exchange: "bybit".to_owned(),
        side: row.side,
        quantity: row.size,
        entry_price: row.entry_price,
        mark_price: row.mark_price,
        unrealized_pnl: row.unrealized_pnl,
        leverage: row.leverage,
        liquidation_price: row.liquidation_price,
        liquidation_distance_pct: None,
        next_funding_ms: None,
        paired_with: None,
        margin: 0.0,
        maintenance_margin_ratio: 0.0,
        position_mode: None,
        margin_mode: None,
        risk_rate: None,
        available_position: None,
        frozen_position: None,
    }
}

/// PR-DP-08 D-4 (Bybit V5 wallet): Bybit V5 在订阅 `wallet` 主题后会按 `account_type`
/// 先推 `type: "snapshot"` 全量行，随后 `type: "delta"` 增量。文档：
/// <https://bybit-exchange.github.io/docs/v5/websocket/private/wallet>
///
/// 系统默认 `BybitConfig.account_type = Unified`（生产 99% UTA），mapper 在
/// snapshot 路径仅保留 `accountType == "UNIFIED"` 行（与 REST `/v5/account/wallet-balance`
/// 默认 query `accountType=UNIFIED` 行为一致）；其他 `account_type`（CONTRACT/SPOT/FUND）
/// 与 incremental delta 仍走 `AccountDirty` 触发下一次 read REST 重拉，由 adapter
/// 按 `BybitConfig.account_type` 重新查询。
pub(super) fn map_bybit_wallet_update(
    update: bybit_ws_user::BybitWalletUpdate,
) -> Vec<PrivateWsEvent> {
    if update.update_type != "snapshot" {
        return dirty_account(
            "bybit",
            PrivateAccountScope::Balances,
            "wallet_delta_requires_rest_refresh",
        );
    }
    let mut rows = Vec::new();
    let mut valuations = Vec::new();
    let mut events = Vec::new();
    let observed_at_ms = update.observed_at_ms;
    for account in update
        .accounts
        .into_iter()
        .filter(|account| account.account_type == "UNIFIED")
    {
        events.push(PrivateWsEvent::AccountSummary(VenueAccountSummary {
            venue: "bybit".to_owned(),
            account_type: account.account_type.clone(),
            equity_scope: shared_types::AccountEquityScope::Unified,
            total_equity_usd: account.total_equity,
            total_available_balance_usd: account.total_available_balance,
            withdrawable_balance_usd: None,
            total_initial_margin_usd: account.total_initial_margin,
            total_maintenance_margin_usd: account.total_maintenance_margin,
            account_im_rate: account.account_im_rate,
            account_mm_rate: account.account_mm_rate,
            source: "bybit.private_ws.wallet".to_owned(),
            observed_at_ms,
            freshness_ms: Some(common::time::now_ms().saturating_sub(observed_at_ms).max(0)),
            problem: None,
        }));
        let assigned_currency = bybit_unified_available_currency(&account.coins);
        for coin in account.coins {
            valuations.push(VenueAssetValuation {
                venue: "bybit".to_owned(),
                currency: coin.coin.clone(),
                usd_value: coin.usd_value,
                source: "bybit.private_ws.wallet.coin.usdValue".to_owned(),
                observed_at_ms,
            });
            rows.push(bybit_balance(
                coin,
                assigned_currency.as_deref(),
                account.total_available_balance,
            ));
        }
    }
    if events.is_empty() {
        return dirty_account(
            "bybit",
            PrivateAccountScope::Balances,
            "wallet_snapshot_missing_unified_account",
        );
    }
    events.push(PrivateWsEvent::AssetValuations(Box::new(
        PrivateAssetValuationSnapshot {
            venue: "bybit".to_owned(),
            rows: valuations,
        },
    )));
    events.push(PrivateWsEvent::Balances(Box::new(
        PrivateBalancesSnapshot {
            venue: "bybit".to_owned(),
            rows,
        },
    )));
    events
}

pub(super) fn bybit_balance(
    row: bybit_ws_user::BybitWalletCoinDelta,
    assigned_currency: Option<&str>,
    total_available_balance: f64,
) -> VenueBalanceInfo {
    let available = assigned_currency
        .filter(|currency| currency.eq_ignore_ascii_case(&row.coin))
        .map_or(0.0, |_| total_available_balance);
    VenueBalanceInfo {
        venue: "bybit".to_owned(),
        currency: row.coin,
        total: row.equity,
        // Bybit V5 UNIFIED wallet uses account-level `totalAvailableBalance`;
        // `availableToWithdraw` is not a reliable UTA coin-level source.
        available,
        frozen: row.locked,
        unrealized_pnl: row.unrealized_pnl,
    }
}

fn bybit_unified_available_currency(
    coins: &[bybit_ws_user::BybitWalletCoinDelta],
) -> Option<String> {
    ["USDT", "USDC", "USD"].iter().find_map(|preferred| {
        coins
            .iter()
            .find(|coin| coin.coin.eq_ignore_ascii_case(preferred))
            .map(|coin| coin.coin.clone())
    })
}

pub(super) fn bybit_fill_delta(row: bybit_ws_user::BybitExecutionUpdate) -> PrivateFillDelta {
    PrivateFillDelta {
        venue: "bybit".to_owned(),
        exchange_order_id: row.order_id.clone(),
        client_order_id: non_empty_text(&row.client_order_id),
        symbol: non_empty_text(&row.symbol),
        side: order_side_from_text(&row.side),
        venue_event_id: format!("bybit_execution:{}:{}", row.order_id, row.exec_id),
        quantity: row.size,
        price: row.price,
        fee_amount: row.fee,
        fee_currency: row.fee_currency,
        occurred_at_ms: row.trade_time_ms,
    }
}
