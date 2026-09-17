use std::collections::BTreeMap;

use rust_decimal::{prelude::FromPrimitive, Decimal};
use shared_types::{
    OnchainExecutionAccounting as Accounting, OnchainExecutionAccountingStatus as Status,
    OnchainExecutionAssetChange, OnchainExecutionSubmitResponse, OnchainExecutionUsdValue,
    OnchainUsdValuation,
};

use super::super::usd_valuation;
use crate::state::AppState;

mod flows;

const MAX_AGE_MS: i64 = 30_000;
const AUTO_WINDOW_MS: i64 = 600_000;

pub(super) fn enrich(
    state: &AppState,
    run: &mut OnchainExecutionSubmitResponse,
    can_finalize: bool,
) {
    let mut next = Accounting {
        status: Status::PendingReceipts,
        flows: Vec::new(),
        net_assets: Vec::new(),
        usd_value: None,
        problems: Vec::new(),
    };
    let result = (|| {
        if !can_finalize {
            return Err("执行日志或未决订单尚未核清，暂不确认净变动".into());
        }
        next.flows = flows::collect(run)?;
        next.net_assets = flows::net(&next.flows)?;
        next.status = Status::PendingValuation;
        if let Some(saved) = run
            .accounting
            .as_ref()
            .filter(|saved| saved.flows == next.flows && saved.net_assets == next.net_assets)
        {
            if let Some(value) = &saved.usd_value {
                let checked = value_at(&next.net_assets, value.rates.clone(), value.valued_at_ms)?;
                if checked != *value {
                    return Err("已保存的美元折算与原币收支不一致".into());
                }
                next.usd_value = Some(checked);
                next.status = Status::Valued;
                return Ok(());
            }
        }
        let now_ms = common::time::now_ms();
        let mut config = state.onchain_monitor().snapshot().config.clone();
        if let Some(primary) = run
            .legs
            .iter()
            .find(|leg| leg.kind == shared_types::OnchainExecutionLegKind::PrimaryCex)
        {
            config.cex_venue = primary.venue.clone();
        }
        config.max_age_ms = config.max_age_ms.clamp(1, MAX_AGE_MS);
        let rates = next
            .net_assets
            .iter()
            .map(|asset| usd_valuation::evidence(state, &config, &asset.asset, now_ms))
            .collect::<Result<Vec<_>, _>>()?;
        next.usd_value = Some(value_at(&next.net_assets, rates, now_ms)?);
        next.status = Status::Valued;
        Ok::<(), String>(())
    })();
    if let Err(problem) = result {
        next.problems.push(problem);
    }
    run.accounting = Some(next);
}

fn value_at(
    assets: &[OnchainExecutionAssetChange],
    rates: Vec<OnchainUsdValuation>,
    now_ms: i64,
) -> Result<OnchainExecutionUsdValue, String> {
    if assets.len() != rates.len() {
        return Err("美元汇率覆盖不完整".into());
    }
    let mut total = Decimal::ZERO;
    let mut seen = std::collections::BTreeSet::new();
    for (asset, rate) in assets.iter().zip(&rates) {
        let amount = flows::amount(Some(&asset.amount_exact))?;
        if !seen.insert(&asset.asset) || amount == Decimal::ZERO {
            return Err("净资产重复或无效".into());
        }
        usd_valuation::rate(Some(rate), &asset.asset, MAX_AGE_MS, now_ms)
            .ok_or("折算汇率不是同一资产的新鲜 WS")?;
        let pair = onchain_monitor::normalized_pair_symbol(&rate.symbol);
        if asset.asset == "USD"
            && (rate.source != "same_currency"
                || rate.usd_bid != 1.0
                || rate.usd_ask != 1.0
                || pair != "USDUSD")
        {
            return Err("法币美元口径无效".into());
        }
        if asset.asset != "USD"
            && (rate.venue.is_empty()
                || (pair
                    != onchain_monitor::normalized_pair_symbol(&format!("{}/USD", asset.asset))
                    && pair
                        != onchain_monitor::normalized_pair_symbol(&format!(
                            "USD/{}",
                            asset.asset
                        ))))
        {
            return Err("折算缺少真实 USD 市场，不允许隐式稳定币平价".into());
        }
        let price = if amount > Decimal::ZERO {
            rate.usd_bid
        } else {
            rate.usd_ask
        };
        let usd = Decimal::from_f64(price)
            .and_then(|price| amount.checked_mul(price))
            .filter(|usd| *usd != Decimal::ZERO)
            .ok_or("美元折算超出支持精度")?;
        total = total.checked_add(usd).ok_or("美元净变动合计溢出")?;
    }
    Ok(OnchainExecutionUsdValue {
        net_usd_exact: total.normalize().to_string(),
        valued_at_ms: now_ms,
        rates,
    })
}

pub(crate) fn value_flows(
    flows: &[shared_types::OnchainExecutionCashFlow],
    rates: Vec<OnchainUsdValuation>,
    now_ms: i64,
) -> Result<OnchainExecutionUsdValue, String> {
    value_at(&flows::net(flows)?, rates, now_ms)
}

pub(super) fn refresh_one(
    state: &AppState,
    run_id: &str,
) -> Option<OnchainExecutionSubmitResponse> {
    let mut current = state.onchain_execution_runs().get_mut(run_id)?;
    let mut next = current.clone();
    super::settlement::enrich(state, &mut next);
    if next != *current {
        next.updated_at_ms = common::time::now_ms();
        if let Err(problem) = state.onchain_execution_run_store().append_run(&next) {
            next = super::interrupted_response(next, problem);
        }
        *current = next;
    }
    Some(current.clone())
}

pub(crate) async fn refresh_pending(state: &AppState, now_ms: i64) {
    if state.onchain_execution_run_store().readiness().is_err() {
        return;
    }
    let mut rows = state
        .onchain_execution_runs()
        .iter()
        .filter(|run| {
            now_ms >= run.started_at_ms
                && now_ms.saturating_sub(run.started_at_ms) <= AUTO_WINDOW_MS
                && run
                    .accounting
                    .as_ref()
                    .is_none_or(|a| a.status != Status::Valued)
                && !matches!(
                    run.status,
                    shared_types::OnchainExecutionRunStatus::Executing
                        | shared_types::OnchainExecutionRunStatus::AwaitingChainFinality
                )
        })
        .map(|run| run.value().clone())
        .collect::<Vec<_>>();
    rows.sort_by_key(|run| run.updated_at_ms);
    if !rows.is_empty() {
        let offset = (now_ms.div_euclid(5_000).rem_euclid(rows.len() as i64)) as usize;
        rows.rotate_left(offset);
    }
    rows.truncate(4);
    let mut needed = BTreeMap::<String, std::collections::BTreeSet<String>>::new();
    for row in &rows {
        if let Some(current) = refresh_one(state, &row.run_id) {
            if let Some(accounting) = current
                .accounting
                .filter(|a| a.status == Status::PendingValuation)
            {
                let venue = current
                    .legs
                    .iter()
                    .find(|l| l.kind == shared_types::OnchainExecutionLegKind::PrimaryCex)
                    .map(|l| l.venue.clone())
                    .unwrap_or_else(|| "kraken".into());
                needed
                    .entry(venue)
                    .or_default()
                    .extend(accounting.net_assets.into_iter().take(8).map(|a| a.asset));
            }
        }
    }
    let mut config = state.onchain_monitor().snapshot().config.clone();
    // Reuse the existing WS demand leases; never add market-wide REST polling for accounting.
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        for (venue, assets) in needed {
            config.cex_venue = venue;
            let assets = assets.into_iter().collect::<Vec<_>>();
            usd_valuation::refresh_assets(state, &config, &assets).await;
        }
    })
    .await;
    for row in rows {
        refresh_one(state, &row.run_id);
    }
}

#[cfg(test)]
mod tests;
