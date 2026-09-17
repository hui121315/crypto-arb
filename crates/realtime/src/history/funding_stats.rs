use super::types::FundingDiffRow;
use shared_types::{
    problem::codes, ApiProblem, FundingDiffSampleHealth, FundingDiffStatsRow,
    FundingDiffWindowStats, FundingHistoryEvidence,
};

const CYCLE_BUCKETS: [u32; 3] = [1, 3, 9];
const HOUR_MS: i64 = 3_600_000;
const FUNDING_DIFF_SOURCE: &str = "history_store:funding_diff";

mod projector;

pub use projector::FundingDiffStatsProjector;

pub(super) fn build_rows(
    rows: Vec<FundingDiffRow>,
    computed_at_ms: i64,
) -> Vec<FundingDiffStatsRow> {
    let mut projector = FundingDiffStatsProjector::default();
    projector.replace(rows);
    projector.snapshot(computed_at_ms)
}

fn build_pair_row(rows: &mut [FundingDiffRow], computed_at_ms: i64) -> Option<FundingDiffStatsRow> {
    rows.sort_by_key(|row| row.occurred_at_ms);
    let latest = rows.last()?;
    let base_interval_hours = base_interval_hours(latest);
    let freshness_ms = freshness_ms(computed_at_ms, latest.occurred_at_ms);
    let windows = CYCLE_BUCKETS
        .into_iter()
        .map(|cycles| window_stats(rows, latest, base_interval_hours, cycles, computed_at_ms))
        .collect::<Vec<_>>();
    let evidence = windows
        .last()
        .map(|window| window.evidence.clone())
        .unwrap_or_default();
    Some(FundingDiffStatsRow {
        symbol: latest.symbol.clone(),
        long_exchange: latest.long_exchange.clone(),
        short_exchange: latest.short_exchange.clone(),
        computed_at_ms,
        latest_at_ms: latest.occurred_at_ms,
        latest_diff_bps: latest.gross_diff_bps,
        base_interval_hours,
        source: FUNDING_DIFF_SOURCE.into(),
        freshness_ms: Some(freshness_ms),
        problem: row_problem(freshness_ms, base_interval_hours),
        problem_detail: row_problem_detail(freshness_ms, base_interval_hours),
        retry_after_ms: None,
        evidence,
        windows,
    })
}

fn window_stats(
    rows: &[FundingDiffRow],
    latest: &FundingDiffRow,
    base_interval_hours: u32,
    cycles: u32,
    computed_at_ms: i64,
) -> FundingDiffWindowStats {
    let window_hours = base_interval_hours.saturating_mul(cycles);
    let cutoff_ms = latest
        .occurred_at_ms
        .saturating_sub(i64::from(window_hours) * HOUR_MS);
    let values = window_values(rows, cutoff_ms, latest.occurred_at_ms);
    stats_from_values(
        cycles,
        window_hours,
        latest.gross_diff_bps,
        computed_at_ms,
        freshness_ms(computed_at_ms, latest.occurred_at_ms),
        &values,
    )
}

fn window_values(rows: &[FundingDiffRow], cutoff_ms: i64, latest_ms: i64) -> Vec<f64> {
    rows.iter()
        .filter(|row| row.occurred_at_ms >= cutoff_ms && row.occurred_at_ms <= latest_ms)
        .map(|row| row.gross_diff_bps)
        .filter(|value| value.is_finite())
        .collect()
}

fn stats_from_values(
    cycles: u32,
    window_hours: u32,
    latest_diff_bps: f64,
    computed_at_ms: i64,
    freshness_ms: i64,
    values: &[f64],
) -> FundingDiffWindowStats {
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let sample_health = sample_health(values.len(), cycles, window_hours, freshness_ms);
    let problem = window_problem(sample_health, values.len(), cycles);
    let problem_detail = window_problem_detail(sample_health, values.len(), cycles);
    FundingDiffWindowStats {
        cycles,
        window_hours,
        sample_count: values.len(),
        mean_diff_bps: mean(values),
        p50_diff_bps: percentile(&sorted, 50.0),
        p75_diff_bps: percentile(&sorted, 75.0),
        p90_diff_bps: percentile(&sorted, 90.0),
        p95_diff_bps: percentile(&sorted, 95.0),
        stddev_diff_bps: stddev(values),
        positive_ratio: positive_ratio(values),
        reversal_count: reversal_count(values),
        current_percentile: percentile_rank(values, latest_diff_bps),
        source: FUNDING_DIFF_SOURCE.into(),
        freshness_ms: Some(freshness_ms),
        sample_health,
        problem,
        problem_detail: problem_detail.clone(),
        retry_after_ms: None,
        evidence: FundingHistoryEvidence {
            source: FUNDING_DIFF_SOURCE.into(),
            observed_at_ms: computed_at_ms,
            latest_at_ms: computed_at_ms.saturating_sub(freshness_ms),
            freshness_ms: Some(freshness_ms),
            sample_count: values.len(),
            sample_health,
            problem: problem_detail,
            retry_after_ms: None,
        },
    }
}

fn freshness_ms(computed_at_ms: i64, latest_at_ms: i64) -> i64 {
    computed_at_ms.saturating_sub(latest_at_ms).max(0)
}

fn sample_health(
    sample_count: usize,
    cycles: u32,
    window_hours: u32,
    freshness_ms: i64,
) -> FundingDiffSampleHealth {
    if sample_count == 0 {
        return FundingDiffSampleHealth::Empty;
    }
    if is_stale(freshness_ms, window_hours) {
        return FundingDiffSampleHealth::Stale;
    }
    if sample_count < cycles.max(1) as usize {
        return FundingDiffSampleHealth::Thin;
    }
    FundingDiffSampleHealth::Ok
}

fn is_stale(freshness_ms: i64, window_hours: u32) -> bool {
    let stale_after_ms = i64::from(window_hours.max(1)) * HOUR_MS;
    freshness_ms > stale_after_ms
}

fn row_problem(freshness_ms: i64, base_interval_hours: u32) -> Option<String> {
    is_stale(freshness_ms, base_interval_hours)
        .then(|| format!("funding diff latest sample stale: freshness_ms={freshness_ms}"))
}

fn row_problem_detail(freshness_ms: i64, base_interval_hours: u32) -> Option<ApiProblem> {
    let message = row_problem(freshness_ms, base_interval_hours)?;
    Some(problem(codes::FUNDING_DIFF_HISTORY_STALE, message))
}

fn window_problem(
    sample_health: FundingDiffSampleHealth,
    sample_count: usize,
    cycles: u32,
) -> Option<String> {
    match sample_health {
        FundingDiffSampleHealth::Empty => Some("funding diff window has no samples".into()),
        FundingDiffSampleHealth::Thin => Some(format!(
            "funding diff window has {sample_count} samples for {cycles} cycles"
        )),
        FundingDiffSampleHealth::Stale => Some("funding diff window latest sample is stale".into()),
        FundingDiffSampleHealth::Unknown | FundingDiffSampleHealth::Ok => None,
    }
}

fn window_problem_detail(
    sample_health: FundingDiffSampleHealth,
    sample_count: usize,
    cycles: u32,
) -> Option<ApiProblem> {
    let message = window_problem(sample_health, sample_count, cycles)?;
    Some(problem(codes::FUNDING_DIFF_WINDOW_DEGRADED, message))
}

fn problem(code: &'static str, message: String) -> ApiProblem {
    ApiProblem::new(code, message).with_source(FUNDING_DIFF_SOURCE)
}

fn percentile(sorted_values: &[f64], percentile: f64) -> f64 {
    if sorted_values.is_empty() {
        return 0.0;
    }
    let rank = percentile / 100.0 * (sorted_values.len().saturating_sub(1) as f64);
    sorted_values[rank.round() as usize]
}

fn percentile_rank(values: &[f64], latest_diff_bps: f64) -> u8 {
    if values.is_empty() || !latest_diff_bps.is_finite() {
        return 0;
    }
    let below_or_equal = values
        .iter()
        .filter(|value| **value <= latest_diff_bps)
        .count();
    ((below_or_equal as f64 / values.len() as f64) * 100.0).round() as u8
}

fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.iter().sum::<f64>() / values.len() as f64
}

fn stddev(values: &[f64]) -> f64 {
    if values.len() < 2 {
        return 0.0;
    }
    let avg = mean(values);
    let variance = values
        .iter()
        .map(|value| {
            let delta = value - avg;
            delta * delta
        })
        .sum::<f64>()
        / values.len() as f64;
    variance.sqrt()
}

fn positive_ratio(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let positive = values.iter().filter(|value| **value > 0.0).count();
    positive as f64 / values.len() as f64
}

fn reversal_count(values: &[f64]) -> usize {
    let mut previous = 0;
    let mut count = 0;
    for current in values.iter().map(|value| value.signum() as i8) {
        if current != 0 && previous != 0 && current != previous {
            count += 1;
        }
        if current != 0 {
            previous = current;
        }
    }
    count
}

fn base_interval_hours(row: &FundingDiffRow) -> u32 {
    row.long_interval_hours.max(row.short_interval_hours).max(1)
}

fn valid_row(row: &FundingDiffRow) -> bool {
    row.gross_diff_bps.is_finite()
        && row.long_interval_hours > 0
        && row.short_interval_hours > 0
        && !row.symbol.is_empty()
        && !row.long_exchange.is_empty()
        && !row.short_exchange.is_empty()
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct FundingDiffKey {
    symbol: String,
    long_exchange: String,
    short_exchange: String,
}

impl FundingDiffKey {
    fn from(row: &FundingDiffRow) -> Self {
        Self {
            symbol: row.symbol.clone(),
            long_exchange: row.long_exchange.clone(),
            short_exchange: row.short_exchange.clone(),
        }
    }
}

#[cfg(test)]
mod tests;
