use super::*;

pub(crate) fn map_bitget_event(event: bitget_ws_user::BitgetUserEvent) -> Vec<PrivateWsEvent> {
    match event {
        bitget_ws_user::BitgetUserEvent::Account(update) => map_bitget_account_update(update),
        bitget_ws_user::BitgetUserEvent::Order(rows) => rows
            .into_iter()
            .flat_map(|row| {
                let received_at_ms = row.order.created_at.timestamp_millis();
                order_event(&row.client_order_id, row.order, received_at_ms)
            })
            .collect(),
        bitget_ws_user::BitgetUserEvent::Position(update) => map_bitget_position_update(update),
        bitget_ws_user::BitgetUserEvent::Fill(rows) => rows
            .into_iter()
            .map(bitget_fill_delta)
            .map(PrivateWsEvent::Fill)
            .collect(),
    }
}

fn map_bitget_position_update(update: bitget_ws_user::BitgetPositionUpdate) -> Vec<PrivateWsEvent> {
    let rows = update
        .positions
        .into_iter()
        .map(bitget_position)
        .collect::<Vec<_>>();
    match update.action.as_str() {
        "snapshot" => vec![PrivateWsEvent::Positions(PrivatePositionsSnapshot {
            venue: "bitget".to_owned(),
            rows,
        })],
        "update" if rows.is_empty() => Vec::new(),
        "update" => vec![PrivateWsEvent::PositionPatch(PrivatePositionsPatch {
            venue: "bitget".to_owned(),
            rows,
        })],
        _ => dirty_account(
            "bitget",
            PrivateAccountScope::Positions,
            "unknown_position_update_action",
        ),
    }
}

fn bitget_position(row: bitget_ws_user::BitgetPositionDelta) -> PositionInfo {
    PositionInfo {
        symbol: row.symbol,
        exchange: "bitget".to_owned(),
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
        margin: row.margin_size,
        maintenance_margin_ratio: row.maintenance_margin_rate,
        position_mode: Some(row.hold_mode),
        margin_mode: Some(row.margin_mode),
        risk_rate: None,
        available_position: Some(row.available),
        frozen_position: Some(row.frozen),
    }
}

fn map_bitget_account_update(update: bitget_ws_user::BitgetAccountUpdate) -> Vec<PrivateWsEvent> {
    if update.action != "snapshot" {
        return dirty_account(
            "bitget",
            PrivateAccountScope::Balances,
            "account_update_requires_rest_refresh",
        );
    }
    let observed_at_ms = common::time::now_ms();
    let summary = VenueAccountSummary {
        venue: "bitget".to_owned(),
        account_type: "uta".to_owned(),
        equity_scope: shared_types::AccountEquityScope::Unified,
        total_equity_usd: update.total_equity,
        total_available_balance_usd: update.effective_equity,
        withdrawable_balance_usd: None,
        total_initial_margin_usd: update.initial_margin,
        total_maintenance_margin_usd: update.maintenance_margin,
        account_im_rate: update.margin_ratio,
        account_mm_rate: update.position_margin_ratio,
        source: "bitget.private_ws.account".to_owned(),
        observed_at_ms,
        freshness_ms: Some(0),
        problem: None,
    };
    let mut rows = Vec::with_capacity(update.accounts.len());
    let mut valuations = Vec::with_capacity(update.accounts.len());
    for account in update.accounts {
        valuations.push(VenueAssetValuation {
            venue: "bitget".to_owned(),
            currency: account.coin.clone(),
            // Bitget's parsed `usdt_equity` retains the official UTA account
            // channel `coin[].usdValue` (legacy frames used `usdtEquity`).
            usd_value: account.usdt_equity,
            source: "bitget.private_ws.account.coin.usdValue".to_owned(),
            observed_at_ms,
        });
        rows.push(bitget_balance(account));
    }
    vec![
        PrivateWsEvent::Balances(Box::new(PrivateBalancesSnapshot {
            venue: "bitget".to_owned(),
            rows,
        })),
        PrivateWsEvent::AssetValuations(Box::new(PrivateAssetValuationSnapshot {
            venue: "bitget".to_owned(),
            rows: valuations,
        })),
        PrivateWsEvent::AccountSummary(summary),
    ]
}

fn bitget_balance(row: bitget_ws_user::BitgetAccountDelta) -> VenueBalanceInfo {
    let total = if row.coin.eq_ignore_ascii_case("USDT") && row.usdt_equity > row.equity {
        row.usdt_equity
    } else {
        row.equity
    };
    VenueBalanceInfo {
        venue: "bitget".to_owned(),
        currency: row.coin,
        total,
        available: row.available,
        frozen: row.frozen,
        unrealized_pnl: row.unrealized_pnl,
    }
}

fn bitget_fill_delta(row: bitget_ws_user::BitgetFillUpdate) -> PrivateFillDelta {
    PrivateFillDelta {
        venue: "bitget".to_owned(),
        exchange_order_id: row.order_id.clone(),
        client_order_id: non_empty_text(&row.client_order_id),
        symbol: non_empty_text(&row.symbol),
        side: order_side_from_text(&row.side),
        venue_event_id: format!("bitget_fill:{}:{}", row.order_id, row.exec_id),
        quantity: row.size,
        price: row.price,
        fee_amount: Some(row.fee),
        fee_currency: row.fee_currency,
        occurred_at_ms: row.trade_time_ms,
    }
}
