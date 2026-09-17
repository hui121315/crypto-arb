use super::tasks::{tick_or_shutdown, BackgroundTasks, ShutdownToken};
use crate::services::market_data::{
    self, MarketDataCache, MarketRowsSnapshot, MarketSource, MARKET_OP_REST_FUNDING_RATES,
};
use crate::services::market_subscriptions::{MarketSubscriptionFeed, MarketSubscriptions};
use crate::state::AppState;
use std::collections::BTreeMap;
use std::sync::Arc;
use tracing::{info, warn};

const UPDATER_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);
const COLD_START_WS_SETTLE_DELAY: std::time::Duration = std::time::Duration::from_secs(18);
const NO_FUNDING_ROWS: &str = "FUNDING_ROWS_EMPTY";
const FUNDING_HISTORY_APPEND_FAILED: &str = "FUNDING_HISTORY_APPEND_FAILED";
const FUNDING_DIFF_HISTORY_APPEND_FAILED: &str = "FUNDING_DIFF_HISTORY_APPEND_FAILED";
const FUNDING_DIFF_STATS_REFRESH_FAILED: &str = "FUNDING_DIFF_STATS_REFRESH_FAILED";
const FUNDING_STREAM_SERIALIZE_FAILED: &str = "FUNDING_STREAM_SERIALIZE_FAILED";
const FUNDING_DIFF_STATS_BOOTSTRAP_LIMIT: usize = 5_000;

/// 周期聚合 10 家交易所费率并广播到 `funding-rates` WS 频道。
pub(super) fn spawn_updater(state: &AppState, tasks: &mut BackgroundTasks) {
    let interval = UPDATER_INTERVAL;
    let slow_threshold = interval.mul_f32(1.5);
    let state = state.clone();
    let shutdown = tasks.shutdown_token();
    tasks.supervise("funding", interval.as_millis() as i64, move || {
        let state = state.clone();
        let shutdown = shutdown.clone();
        async move { run_updater(state, shutdown).await }
    });

    info!(
        period_secs = interval.as_secs(),
        slow_threshold_secs = slow_threshold.as_secs(),
        "funding rates updater started"
    );
}

async fn run_updater(state: AppState, shutdown: ShutdownToken) {
    let aggregator = Arc::clone(state.aggregator_handle());
    let hub = state.ws_hub().clone();
    let metrics = Arc::clone(state.metrics());
    let history = Arc::clone(state.history_store());
    let market_data = Arc::clone(state.market_data());
    let diff_stats_snapshot = Arc::clone(state.funding_diff_stats_snapshot_handle());
    let interval = UPDATER_INTERVAL;
    let slow_threshold = interval.mul_f32(1.5);

    let registry = state.task_registry().clone();
    let mut ctx = FundingRuntime {
        aggregator,
        hub,
        metrics,
        history,
        market_data,
        market_subscriptions: Arc::clone(state.market_subscriptions()),
        diff_stats_snapshot,
        diff_stats_projector: realtime::FundingDiffStatsProjector::default(),
        slow_threshold,
    };
    let bootstrap_result = bootstrap_funding_diff_stats(&mut ctx).await;
    let ws_ready = tokio::select! {
        biased;
        () = shutdown.cancelled() => false,
        _ = tokio::time::sleep(COLD_START_WS_SETTLE_DELAY) => true,
    };
    if !ws_ready {
        return;
    }
    let started_at_ms = common::time::now_ms();
    let cycle_result = run_once(&mut ctx, true).await.map(|_| ());
    let result = combine_substep_results([bootstrap_result, cycle_result]);
    registry.record_result_timed("funding", started_at_ms, result);

    let mut tick = tokio::time::interval(interval);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    tick.tick().await;
    while tick_or_shutdown(&mut tick, &shutdown).await {
        let started_at_ms = common::time::now_ms();
        let result = run_once(&mut ctx, false).await.map(|_| ());
        registry.record_result_timed("funding", started_at_ms, result);
    }
}

struct FundingRuntime {
    aggregator: Arc<exchange::Aggregator>,
    hub: realtime::WsHub,
    metrics: Arc<crate::metrics::Metrics>,
    history: Arc<realtime::HistoryStore>,
    market_data: Arc<crate::services::market_data::MarketDataCache>,
    market_subscriptions: Arc<MarketSubscriptions>,
    diff_stats_snapshot: Arc<realtime::RefreshingSnapshot<Vec<shared_types::FundingDiffStatsRow>>>,
    diff_stats_projector: realtime::FundingDiffStatsProjector,
    slow_threshold: std::time::Duration,
}

async fn run_once(ctx: &mut FundingRuntime, prewarm: bool) -> Result<usize, String> {
    let enabled_venues = ctx
        .aggregator
        .names()
        .into_iter()
        .filter(|venue| {
            ctx.market_subscriptions
                .enabled(venue, MarketSubscriptionFeed::Funding)
        })
        .collect::<Vec<_>>();
    if enabled_venues.is_empty() {
        ctx.metrics
            .record_funding_fetch(0, 0, common::time::now_ms());
        ctx.metrics.record_funding_exchange_counts(BTreeMap::new());
        return Ok(0);
    }
    let scan_start = std::time::Instant::now();
    let report = ctx
        .aggregator
        .fetch_funding_rates_report_for(&enabled_venues)
        .await;
    let scan_elapsed = scan_start.elapsed();
    ctx.market_data.record_aggregate_fanout_outcome(
        MARKET_OP_REST_FUNDING_RATES,
        MarketSource::RestBaseline,
        &report.venues,
    );
    ctx.market_data
        .record_fanout_outcomes(MarketSource::RestBaseline, report.venues);
    let discovery_rates = report
        .rows
        .into_iter()
        .filter(|row| {
            ctx.market_subscriptions
                .enabled(&row.exchange, MarketSubscriptionFeed::Funding)
        })
        .collect::<Vec<_>>();

    let publish_start = std::time::Instant::now();
    ctx.market_data
        .store_funding_rows(&discovery_rates, MarketSource::RestBaseline);
    let projection = current_funding_projection(&ctx.market_data);
    let rates = projection.rows;
    let row_evidence = projection.row_evidence;
    let count = rates.len();
    let history_result = append_history(ctx, &rates).await;
    let publish_result = publish_rates(
        &ctx.hub,
        &ctx.market_data,
        &rates,
        MarketSource::LocalCache,
        row_evidence,
    );
    let publish_elapsed = publish_start.elapsed();

    let scan_ms = scan_elapsed.as_millis() as u64;
    let counts = counts_by_exchange(&rates);
    ctx.metrics
        .record_funding_fetch(scan_ms, count, common::time::now_ms());
    ctx.metrics.record_funding_exchange_counts(counts);
    log_scan_timing(
        scan_elapsed,
        publish_elapsed,
        ctx.slow_threshold,
        count,
        prewarm,
    );
    funding_cycle_outcome(count, history_result, publish_result)
}

fn current_funding_projection(
    market_data: &MarketDataCache,
) -> MarketRowsSnapshot<shared_types::FundingRateData> {
    market_data.funding_rows_snapshot_with_evidence()
}

async fn append_history(
    ctx: &mut FundingRuntime,
    rates: &[shared_types::FundingRateData],
) -> Result<(), String> {
    let rate_history = append_funding_rate_history(ctx, rates).await;
    let funding_diffs = realtime::HistoryStore::derive_funding_diffs(rates);
    let diff_history = append_funding_diff_history(ctx, &funding_diffs).await;
    let diff_stats = refresh_funding_diff_stats(ctx, &funding_diffs).await;
    combine_substep_results([rate_history, diff_history, diff_stats])
}

async fn append_funding_rate_history(
    ctx: &FundingRuntime,
    rates: &[shared_types::FundingRateData],
) -> Result<(), String> {
    ctx.history
        .append_funding_rates(rates)
        .await
        .map_err(|error| {
            warn!(%error, "funding history append failed");
            format!("{FUNDING_HISTORY_APPEND_FAILED}: {error}")
        })
}

async fn append_funding_diff_history(
    ctx: &FundingRuntime,
    funding_diffs: &[realtime::FundingDiffRow],
) -> Result<(), String> {
    ctx.history
        .append_funding_diffs(funding_diffs)
        .await
        .map_err(|error| {
            warn!(%error, "funding diff history append failed");
            format!("{FUNDING_DIFF_HISTORY_APPEND_FAILED}: {error}")
        })
}

async fn bootstrap_funding_diff_stats(ctx: &mut FundingRuntime) -> Result<(), String> {
    let rows = ctx
        .history
        .query_funding_diffs(realtime::FundingDiffQuery {
            limit: FUNDING_DIFF_STATS_BOOTSTRAP_LIMIT,
            ..realtime::FundingDiffQuery::default()
        })
        .await
        .map_err(|error| {
            warn!(%error, "funding diff stats bootstrap failed");
            format!("{FUNDING_DIFF_STATS_REFRESH_FAILED}: {error}")
        })?;
    ctx.diff_stats_projector.replace(rows);
    let snapshot = ctx.diff_stats_projector.snapshot(common::time::now_ms());
    ctx.diff_stats_snapshot.set(snapshot).await;
    Ok(())
}

async fn refresh_funding_diff_stats(
    ctx: &mut FundingRuntime,
    funding_diffs: &[realtime::FundingDiffRow],
) -> Result<(), String> {
    let rows = ctx
        .diff_stats_projector
        .apply(funding_diffs, common::time::now_ms());
    ctx.diff_stats_snapshot.set(rows).await;
    Ok(())
}

fn counts_by_exchange(rates: &[shared_types::FundingRateData]) -> BTreeMap<String, u64> {
    let mut counts = BTreeMap::new();
    for r in rates {
        *counts.entry(r.exchange.clone()).or_insert(0) += 1;
    }
    counts
}

fn publish_rates(
    hub: &realtime::WsHub,
    market_data: &crate::services::market_data::MarketDataCache,
    rates: &[shared_types::FundingRateData],
    read_source: MarketSource,
    row_evidence: Vec<shared_types::MarketDataRowEvidence>,
) -> Result<(), String> {
    // 零订阅者时跳过全量 rates 克隆与 envelope 序列化。
    if hub.subscriber_count(realtime::channels::FUNDING_RATES) == 0 {
        return Ok(());
    }
    let payload = market_data::envelope::funding_rates_envelope(
        rates.to_vec(),
        read_source,
        common::time::now_ms(),
        &market_data.runtime_health_snapshot(),
        row_evidence,
    );
    let message = realtime::WsMessage::json(&payload).map_err(|error| {
        warn!(%error, "funding rates envelope serialization failed");
        format!("{FUNDING_STREAM_SERIALIZE_FAILED}: {error}")
    })?;
    hub.publish(realtime::channels::FUNDING_RATES, message);
    Ok(())
}

fn funding_cycle_outcome(
    count: usize,
    history_result: Result<(), String>,
    publish_result: Result<(), String>,
) -> Result<usize, String> {
    let row_result = (count > 0)
        .then_some(())
        .ok_or_else(|| format!("{NO_FUNDING_ROWS}: no funding rates fetched"));
    combine_substep_results([row_result, history_result, publish_result]).map(|()| count)
}

fn combine_substep_results<const N: usize>(results: [Result<(), String>; N]) -> Result<(), String> {
    let errors = results
        .into_iter()
        .filter_map(Result::err)
        .collect::<Vec<_>>();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

fn log_scan_timing(
    scan_elapsed: std::time::Duration,
    publish_elapsed: std::time::Duration,
    slow_threshold: std::time::Duration,
    count: usize,
    prewarm: bool,
) {
    let scan_ms = scan_elapsed.as_millis() as u64;
    let publish_ms = publish_elapsed.as_millis() as u64;
    if scan_elapsed >= slow_threshold {
        log_slow_scan(scan_ms, publish_ms, count, prewarm);
    } else {
        log_done_scan(scan_ms, publish_ms, count, prewarm);
    }
}

fn log_slow_scan(scan_ms: u64, publish_ms: u64, count: usize, prewarm: bool) {
    warn!(
        scan_ms,
        publish_ms, count, prewarm, "funding rates fetch slow"
    );
}

fn log_done_scan(scan_ms: u64, publish_ms: u64, count: usize, prewarm: bool) {
    info!(
        scan_ms,
        publish_ms, count, prewarm, "funding rates fetch done"
    );
}

#[cfg(test)]
mod tests;
