use shared_types::{
    DeltaPerAsset, FundingCluster, HardLimitsUsage, MarginPerVenue, PositionRow, PositionSide,
    RiskSnapshot, VenueAccountSummary,
};
use std::cmp::Ordering;
use std::collections::BTreeMap;

const DEFAULT_MAINTENANCE_MARGIN_RATIO: f64 = 0.05;

#[derive(Debug)]
pub struct RiskInputs<'a> {
    pub positions: &'a [PositionRow],
    pub account_summaries: &'a [VenueAccountSummary],
    pub historical_pnl_usd: &'a [f64],
    pub total_nav_usd: f64,
    pub hard_limits: HardLimitsUsage,
    pub now_ms: i64,
}

pub fn compute_risk(inputs: RiskInputs<'_>) -> RiskSnapshot {
    let total_nav = valid_nav(inputs.total_nav_usd);
    let var_99_1d_usd = historical_var_99(inputs.historical_pnl_usd);

    RiskSnapshot {
        var_99_1d_usd,
        var_pct_of_nav: pct(var_99_1d_usd, total_nav).abs(),
        var_sample_size: inputs.historical_pnl_usd.len(),
        funding_clustering: funding_clusters(inputs.positions, inputs.now_ms),
        delta_concentration: delta_per_asset(inputs.positions),
        margin_utilization: margin_per_venue(inputs.positions, inputs.account_summaries),
        hard_limits: hard_limits(inputs.hard_limits, inputs.positions),
        updated_at_ms: inputs.now_ms,
    }
}

fn valid_nav(value: f64) -> f64 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        0.0
    }
}

fn historical_var_99(pnl: &[f64]) -> f64 {
    if pnl.is_empty() {
        return 0.0;
    }
    let mut sorted = pnl.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
    let idx = ((sorted.len() as f64) * 0.01).floor() as usize;
    let worst_pnl = sorted[idx.min(sorted.len() - 1)];
    (-worst_pnl).max(0.0)
}

fn funding_clusters(rows: &[PositionRow], now_ms: i64) -> Vec<FundingCluster> {
    [30_u32, 60, 120]
        .into_iter()
        .map(|cap| {
            let mut cluster = FundingCluster {
                settles_in_minutes: cap,
                position_count: 0,
                total_outflow_usd: 0.0,
            };
            for row in rows {
                let Some(ts) = row.next_funding_ms else {
                    continue;
                };
                if ts < now_ms {
                    continue;
                }
                if !row.funding_rate_verified {
                    continue;
                }
                if minutes_until(ts, now_ms) <= cap {
                    cluster.position_count += 1;
                    cluster.total_outflow_usd += funding_outflow_usd(row);
                }
            }
            cluster
        })
        .collect()
}

fn minutes_until(ts: i64, now_ms: i64) -> u32 {
    ((ts.saturating_sub(now_ms)) / 60_000).min(u32::MAX as i64) as u32
}

fn funding_outflow_usd(row: &PositionRow) -> f64 {
    funding_payment_usd(row).max(0.0)
}

/// 该腿的资金费净额：正值=需要支付（计入 outflow），负值=收取（不计入 outflow）。
/// 方向由仓位方向与费率符号共同决定：多头在正费率付费、空头在负费率付费。
fn funding_payment_usd(row: &PositionRow) -> f64 {
    let side_sign = match row.side {
        PositionSide::Long => 1.0,
        PositionSide::Short => -1.0,
    };
    row.quantity.abs() * row.mark_price * row.funding_rate_8h * side_sign
}

fn delta_per_asset(rows: &[PositionRow]) -> Vec<DeltaPerAsset> {
    let mut map: BTreeMap<String, (f64, f64)> = BTreeMap::new();
    for row in rows {
        let entry = map.entry(row.symbol.clone()).or_insert((0.0, 0.0));
        let qty = match row.side {
            PositionSide::Long => row.quantity,
            PositionSide::Short => -row.quantity,
        };
        entry.0 += qty;
        entry.1 += qty * row.mark_price;
    }
    map.into_iter()
        .map(|(asset, (net_qty, net_notional_usd))| DeltaPerAsset {
            asset,
            net_qty,
            net_notional_usd,
        })
        .collect()
}

fn margin_per_venue(
    rows: &[PositionRow],
    account_summaries: &[VenueAccountSummary],
) -> Vec<MarginPerVenue> {
    let mut map: BTreeMap<String, (f64, f64, f64)> = BTreeMap::new();
    for row in rows {
        let entry = map.entry(row.venue.clone()).or_default();
        entry.0 += row.margin_usd.max(0.0);
        entry.1 += row.margin_usd.max(0.0) * maintenance_margin_ratio(row);
        entry.2 += row.margin_usd + row.unrealized_pnl_usd;
    }
    map.into_iter()
        .map(|(venue, position)| {
            account_margin_row(&venue, position, account_summaries).unwrap_or_else(|| {
                let (initial_margin_usd, maintenance_margin_usd, equity_usd) = position;
                MarginPerVenue {
                    venue,
                    utilization_pct: positive_finite(equity_usd)
                        .then(|| pct(initial_margin_usd, equity_usd).max(0.0)),
                    initial_margin_usd: Some(initial_margin_usd),
                    maintenance_margin_usd,
                    equity_usd,
                    estimated: true,
                }
            })
        })
        .collect()
}

fn account_margin_row(
    venue: &str,
    position: (f64, f64, f64),
    account_summaries: &[VenueAccountSummary],
) -> Option<MarginPerVenue> {
    let position_initial_margin_usd = position.0;
    account_summaries
        .iter()
        .filter(|summary| summary.venue.eq_ignore_ascii_case(venue))
        .filter(|summary| account_margin_summary_usable(summary, position_initial_margin_usd))
        .max_by_key(|summary| summary.observed_at_ms)
        .map(|summary| MarginPerVenue {
            venue: venue.to_owned(),
            utilization_pct: Some(
                pct(summary.total_initial_margin_usd, summary.total_equity_usd).max(0.0),
            ),
            initial_margin_usd: Some(summary.total_initial_margin_usd),
            maintenance_margin_usd: summary.total_maintenance_margin_usd,
            equity_usd: summary.total_equity_usd,
            estimated: false,
        })
}

fn account_margin_summary_usable(
    summary: &VenueAccountSummary,
    position_initial_margin_usd: f64,
) -> bool {
    summary.problem.is_none()
        && positive_finite(summary.total_equity_usd)
        && non_negative_finite(summary.total_initial_margin_usd)
        && non_negative_finite(summary.total_maintenance_margin_usd)
        && (position_initial_margin_usd <= f64::EPSILON
            || summary.total_initial_margin_usd > f64::EPSILON)
}

fn positive_finite(value: f64) -> bool {
    value.is_finite() && value > 0.0
}

fn non_negative_finite(value: f64) -> bool {
    value.is_finite() && value >= 0.0
}

fn has_real_maintenance_ratio(row: &PositionRow) -> bool {
    row.maintenance_margin_ratio.is_finite() && row.maintenance_margin_ratio > 0.0
}

fn maintenance_margin_ratio(row: &PositionRow) -> f64 {
    if has_real_maintenance_ratio(row) {
        row.maintenance_margin_ratio
    } else {
        DEFAULT_MAINTENANCE_MARGIN_RATIO
    }
}

fn hard_limits(mut limits: HardLimitsUsage, rows: &[PositionRow]) -> HardLimitsUsage {
    limits.max_symbol_notional_usd = max_symbol_notional(rows);
    limits
}

fn max_symbol_notional(rows: &[PositionRow]) -> f64 {
    let mut map: BTreeMap<&str, f64> = BTreeMap::new();
    for row in rows {
        *map.entry(row.symbol.as_str()).or_default() += row.quantity.abs() * row.mark_price;
    }
    map.values().copied().fold(0.0, f64::max)
}

fn pct(n: f64, d: f64) -> f64 {
    if d.abs() > f64::EPSILON {
        n / d * 100.0
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests;
