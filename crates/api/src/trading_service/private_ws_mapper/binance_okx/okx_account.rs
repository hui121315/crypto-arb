use super::*;

/// An OKX account snapshot can replace the complete venue balance cache only
/// after its final page. Incremental or partial frames keep the bounded REST
/// refresh path fail-closed.
pub(super) fn map_okx_account_update(update: okx_ws_user::OkxAccountUpdate) -> Vec<PrivateWsEvent> {
    let mut events = update
        .summary
        .as_ref()
        .map(okx_account_summary)
        .map(PrivateWsEvent::AccountSummary)
        .into_iter()
        .collect::<Vec<_>>();
    if update.event_type == "snapshot" && update.last_page {
        events.push(PrivateWsEvent::Balances(Box::new(
            PrivateBalancesSnapshot {
                venue: "okx".to_owned(),
                rows: update.balances.into_iter().map(okx_balance).collect(),
            },
        )));
    } else {
        events.extend(dirty_account(
            "okx",
            PrivateAccountScope::Balances,
            "balance_snapshot_incomplete_or_incremental",
        ));
    }
    events
}

fn okx_account_summary(row: &okx_ws_user::OkxAccountSummaryDelta) -> VenueAccountSummary {
    let now_ms = common::time::now_ms();
    VenueAccountSummary {
        venue: "okx".to_owned(),
        account_type: "trading_account".to_owned(),
        equity_scope: shared_types::AccountEquityScope::Unified,
        total_equity_usd: row.total_equity_usd,
        total_available_balance_usd: row.total_available_balance_usd,
        withdrawable_balance_usd: None,
        total_initial_margin_usd: row.total_initial_margin_usd,
        total_maintenance_margin_usd: row.total_maintenance_margin_usd,
        account_im_rate: account_ratio(row.total_initial_margin_usd, row.total_equity_usd),
        account_mm_rate: account_ratio(row.total_maintenance_margin_usd, row.total_equity_usd),
        source: "okx.private_ws.account".to_owned(),
        observed_at_ms: row.updated_time_ms,
        freshness_ms: Some(now_ms.saturating_sub(row.updated_time_ms).max(0)),
        problem: None,
    }
}

fn account_ratio(margin: f64, equity: f64) -> f64 {
    if equity > 0.0 {
        margin / equity
    } else {
        0.0
    }
}

fn okx_balance(row: okx_ws_user::OkxBalanceDelta) -> VenueBalanceInfo {
    VenueBalanceInfo {
        venue: "okx".to_owned(),
        currency: row.currency,
        total: row.total,
        available: row.available,
        frozen: row.frozen,
        unrealized_pnl: row.unrealized_pnl,
    }
}
