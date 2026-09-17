use super::*;
use shared_types::VenueAccountSummary;

mod account_nav;
mod nav_breakdown;

pub(super) use account_nav::{account_nav, AccountNav};
use nav_breakdown::nav_breakdown;

pub(crate) fn risk_from_rows(
    state: &AppState,
    positions: &[PositionRow],
    account_summaries: &[VenueAccountSummary],
    total_nav_usd: f64,
    now_ms: i64,
    historical_pnl_usd: &[f64],
) -> RiskSnapshot {
    let cfg = state.trading_service().risk_config();
    compute_risk(RiskInputs {
        positions,
        account_summaries,
        historical_pnl_usd,
        total_nav_usd,
        hard_limits: HardLimitsUsage {
            open_orders_used: state.trading_service().open_order_count() as u32,
            open_orders_max: cfg.max_open_orders as u32,
            max_order_notional_usd: cfg.max_order_notional,
            kill_switch_active: cfg.kill_switch_active,
            ..HardLimitsUsage::default()
        },
        now_ms,
    })
}

#[cfg(test)]
pub(crate) fn summary_from_rows(rows: &[PositionRow], now_ms: i64) -> PortfolioSummary {
    let nav = position_equity_estimate(rows);
    summary_from_rows_with_nav(
        rows,
        &AccountStateSnapshot::default(),
        now_ms,
        AccountNav {
            value: nav,
            evidence: estimated_nav_evidence(now_ms),
        },
        Some(nav),
        PortfolioPnlToday::default(),
    )
}

pub(super) fn summary_from_rows_with_nav(
    rows: &[PositionRow],
    account_state: &AccountStateSnapshot,
    now_ms: i64,
    nav: AccountNav,
    nav_yesterday_usd: Option<f64>,
    pnl: PortfolioPnlToday,
) -> PortfolioSummary {
    let AccountNav {
        value: total_nav_usd,
        evidence: nav_evidence,
    } = nav;
    let nav_breakdown = nav_breakdown(rows, account_state, total_nav_usd, &nav_evidence, now_ms);
    compute_summary(&SummaryInputs {
        positions: rows,
        total_nav_usd,
        nav_evidence,
        nav_breakdown,
        nav_yesterday_usd,
        realized_pnl_today_usd: pnl.realized_pnl_usd,
        funding_today_usd: pnl.funding_usd,
        fee_rebate_today_usd: pnl.fee_rebate_usd,
        pnl_evidence: pnl.evidence,
        now_ms,
    })
}

#[cfg(test)]
fn estimated_nav_evidence(now_ms: i64) -> PortfolioNavEvidence {
    PortfolioNavEvidence {
        status: AccountFieldQualityStatus::Estimated,
        source: "position_margin_estimate".to_owned(),
        observed_at_ms: now_ms,
        ..PortfolioNavEvidence::default()
    }
}

#[derive(Clone, Copy)]
pub(super) struct NavSamplePlan {
    pub(super) should_persist: bool,
    pub(super) skipped_source: &'static str,
    pub(super) skipped_problem: &'static str,
}

pub(super) fn nav_sample_plan(nav: &AccountNav) -> NavSamplePlan {
    if nav.evidence.status != AccountFieldQualityStatus::Actual {
        return NavSamplePlan {
            should_persist: false,
            skipped_source: nav_persist::NAV_SAMPLE_SOURCE_ACCOUNT_EQUITY_MISSING,
            skipped_problem: "account-level equity coverage incomplete; NAV sample skipped",
        };
    }
    NavSamplePlan {
        should_persist: true,
        skipped_source: nav_persist::NAV_SAMPLE_SOURCE_ACCOUNT_EQUITY,
        skipped_problem: "",
    }
}

pub(super) async fn update_nav_history(
    state: &AppState,
    nav: f64,
    now_ms: i64,
    plan: NavSamplePlan,
) -> Option<f64> {
    let recover_unknown = state
        .portfolio_nav_storage_health()
        .snapshot(now_ms)
        .latest_sample_status
        .as_deref()
        .is_some_and(|status| status == nav_persist::NAV_SAMPLE_STATUS_UNKNOWN);
    let sample = {
        let mut rows = state.portfolio_nav_history().write().await;
        prune_nav_history(&mut rows, now_ms);
        let sample =
            nav_sample_due(&rows, now_ms, nav, plan, recover_unknown).then_some((now_ms, nav));
        if let Some(row) = sample {
            rows.push(row);
        }
        let nav_yesterday = nav_at_lookback(&rows, now_ms);
        (nav_yesterday, sample)
    };
    if let Some((ts, nav)) = sample.1 {
        nav_persist::append_sample(
            state.config(),
            ts,
            nav,
            state.portfolio_nav_storage_health(),
        )
        .await;
    } else if !plan.should_persist {
        state.portfolio_nav_storage_health().record_sample_skipped(
            now_ms,
            plan.skipped_source,
            plan.skipped_problem,
        );
    }
    sample.0
}

pub(super) fn nav_sample_due(
    rows: &[(i64, f64)],
    now_ms: i64,
    nav: f64,
    plan: NavSamplePlan,
    recover_unknown: bool,
) -> bool {
    plan.should_persist
        && nav.is_finite()
        && (recover_unknown || should_sample_nav(rows, now_ms, nav))
}

pub(super) fn pnl_history_values(rows: &[(i64, f64)]) -> Vec<f64> {
    rows.iter().map(|(_, pnl)| *pnl).collect()
}

pub(super) fn prune_nav_history(rows: &mut Vec<(i64, f64)>, now_ms: i64) {
    let oldest = now_ms.saturating_sub(NAV_HISTORY_MS);
    rows.retain(|(ts, nav)| *ts >= oldest && nav.is_finite());
}

pub(super) fn should_sample_nav(rows: &[(i64, f64)], now_ms: i64, nav: f64) -> bool {
    nav.is_finite()
        && rows
            .last()
            .map(|(ts, _)| now_ms.saturating_sub(*ts) >= NAV_SAMPLE_MS)
            .unwrap_or(true)
}

pub(super) fn nav_at_lookback(rows: &[(i64, f64)], now_ms: i64) -> Option<f64> {
    rows.iter()
        .filter(|(ts, _)| *ts <= now_ms.saturating_sub(NAV_LOOKBACK_MS))
        .max_by_key(|(ts, _)| *ts)
        .map(|(_, value)| *value)
}

#[cfg(test)]
pub(super) fn position_equity_estimate(rows: &[PositionRow]) -> f64 {
    rows.iter()
        .map(|row| row.margin_usd + row.unrealized_pnl_usd)
        .sum()
}
