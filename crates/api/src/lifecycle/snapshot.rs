use super::tasks::{BackgroundTasks, ShutdownToken};
use crate::services::{
    instrument_registry::InstrumentRegistry, market_data::MarketDataCache, opportunity,
    snapshots::ARBITRAGE_SNAPSHOT_REFRESH_INTERVAL_MS,
};
use crate::state::AppState;
#[cfg(test)]
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{info, warn};

mod alerts;
mod history_sampling;
mod logging;
mod market_health;
mod publish;
#[cfg(test)]
mod tests;
mod top_window;

use alerts::fire_matching_alerts;
use history_sampling::{append_history_if_due, OpportunityHistorySchedule};
use logging::{log_scan_timing, main_p0_snapshot_query_key};
use market_health::attach_market_data_problems;
use publish::publish_snapshot;
#[cfg(test)]
use publish::{
    record_stream_payload_metrics, snapshot_payload_value, ARBITRAGE_STREAM_SERIALIZE_FAILED,
};
use top_window::{snapshot_notice_if_subscribed, TopWindowState};

const MARKET_EVENT_SCAN_FLOOR: std::time::Duration = std::time::Duration::from_millis(250);
const MARKET_EVENT_SCAN_DUTY_MULTIPLIER: u32 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScanTrigger {
    Market,
    Periodic,
}
#[derive(Clone)]
struct SnapshotSinks {
    history: Arc<realtime::HistoryStore>,
    watchlist: Arc<tokio::sync::RwLock<Vec<realtime::WatchlistItem>>>,
    alert_rules: Arc<tokio::sync::RwLock<Vec<realtime::AlertRule>>>,
    alert_cooldowns: Arc<dashmap::DashMap<i64, i64>>,
    watchlist_alert_store: Arc<realtime::WatchlistAlertStore>,
    watchlist_alert_mutation_lock: Arc<tokio::sync::Mutex<()>>,
}
#[derive(Clone)]
struct SnapshotRuntime {
    state: AppState,
    engine: Arc<arbitrage::ArbitrageEngineV3>,
    scan_lock: Arc<tokio::sync::Mutex<()>>,
    hub: realtime::WsHub,
    metrics: Arc<crate::metrics::Metrics>,
    market_data: Arc<MarketDataCache>,
    instrument_registry: Arc<InstrumentRegistry>,
    sinks: SnapshotSinks,
    slow_threshold: std::time::Duration,
}

/// 周期扫描套利机会，写入快照、历史和 WS `arbitrage` 频道。
pub(super) fn spawn_updater(state: &AppState, tasks: &mut BackgroundTasks) {
    let interval = std::time::Duration::from_millis(ARBITRAGE_SNAPSHOT_REFRESH_INTERVAL_MS);
    let slow_threshold = interval.mul_f32(1.5);
    let state = state.clone();
    let shutdown = tasks.shutdown_token();
    tasks.supervise(
        "arbitrage_snapshot",
        interval.as_millis() as i64,
        move || {
            let state = state.clone();
            let shutdown = shutdown.clone();
            async move { run_updater(state, shutdown).await }
        },
    );

    info!(
        period_secs = interval.as_secs(),
        event_floor_ms = MARKET_EVENT_SCAN_FLOOR.as_millis() as u64,
        event_duty_multiplier = MARKET_EVENT_SCAN_DUTY_MULTIPLIER,
        slow_threshold_secs = slow_threshold.as_secs(),
        "arbitrage snapshot updater started"
    );
}

async fn run_updater(state: AppState, shutdown: ShutdownToken) {
    let interval = std::time::Duration::from_millis(ARBITRAGE_SNAPSHOT_REFRESH_INTERVAL_MS);
    let slow_threshold = interval.mul_f32(1.5);
    let runtime = SnapshotRuntime {
        state: state.clone(),
        engine: Arc::clone(state.arbitrage_engine_handle()),
        scan_lock: Arc::clone(state.arbitrage_scan_lock()),
        hub: state.ws_hub().clone(),
        metrics: Arc::clone(state.metrics()),
        market_data: Arc::clone(state.market_data()),
        instrument_registry: Arc::clone(state.instrument_registry()),
        sinks: SnapshotSinks {
            history: Arc::clone(state.history_store()),
            watchlist: Arc::clone(state.watchlist()),
            alert_rules: Arc::clone(state.alert_rules()),
            alert_cooldowns: Arc::clone(state.alert_cooldowns()),
            watchlist_alert_store: Arc::clone(state.watchlist_alert_store()),
            watchlist_alert_mutation_lock: Arc::clone(state.watchlist_alert_mutation_lock()),
        },
        slow_threshold,
    };

    let registry = state.task_registry().clone();
    let mut top_window = TopWindowState::default();
    let mut history_schedule = OpportunityHistorySchedule::default();
    let mut last_scan_started = std::time::Instant::now();
    let started_at_ms = common::time::now_ms();
    let result = run_once(&runtime, true, &mut top_window, &mut history_schedule).await;
    let mut last_scan_elapsed = last_scan_started.elapsed();
    registry.record_result_timed("arbitrage_snapshot", started_at_ms, result);

    let mut tick = tokio::time::interval(interval);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    tick.tick().await;
    let refresh = Arc::clone(state.arbitrage_refresh_signal());
    loop {
        let trigger = tokio::select! {
            biased;
            () = shutdown.cancelled() => break,
            () = refresh.notified() => ScanTrigger::Market,
            _ = tick.tick() => ScanTrigger::Periodic,
        };
        if trigger == ScanTrigger::Market {
            let delay = event_scan_delay(
                interval,
                last_scan_started,
                last_scan_elapsed,
                std::time::Instant::now(),
            );
            if !delay.is_zero() {
                tokio::select! {
                    biased;
                    () = shutdown.cancelled() => break,
                    () = tokio::time::sleep(delay) => {}
                }
            }
        }
        last_scan_started = std::time::Instant::now();
        let started_at_ms = common::time::now_ms();
        let result = run_once(&runtime, false, &mut top_window, &mut history_schedule).await;
        last_scan_elapsed = last_scan_started.elapsed();
        registry.record_result_timed("arbitrage_snapshot", started_at_ms, result);
        tick.reset();
    }
}

fn event_scan_delay(
    periodic_interval: std::time::Duration,
    last_scan_started: std::time::Instant,
    last_scan_elapsed: std::time::Duration,
    now: std::time::Instant,
) -> std::time::Duration {
    let event_interval = last_scan_elapsed
        .saturating_mul(MARKET_EVENT_SCAN_DUTY_MULTIPLIER)
        .max(MARKET_EVENT_SCAN_FLOOR)
        .min(periodic_interval);
    event_interval.saturating_sub(now.saturating_duration_since(last_scan_started))
}

async fn run_once(
    runtime: &SnapshotRuntime,
    prewarm: bool,
    top_window: &mut TopWindowState,
    history_schedule: &mut OpportunityHistorySchedule,
) -> Result<(), String> {
    let guard = acquire_scan_guard(runtime, prewarm)?;
    let (mut report, scan_elapsed) = build_scan_report(runtime).await?;
    let count = report.opportunities.len();

    let publish_start = std::time::Instant::now();
    let history_append_ok = append_history_if_due(
        &runtime.sinks,
        &report.opportunities,
        history_schedule,
        common::time::now_ms(),
    )
    .await;
    report.meta.history_append_ok = Some(history_append_ok);
    let alert_publish_result = fire_matching_alerts(
        &runtime.hub,
        &runtime.metrics,
        &runtime.sinks,
        &report.opportunities,
    )
    .await;
    let publish_elapsed = publish_start.elapsed();
    report.meta.publish_ms = publish_elapsed.as_millis() as u64;
    attach_market_data_problems(&runtime.market_data, &mut report.meta);
    let scan_ms = report.meta.scan_ms;
    runtime.state.cache_arbitrage_report(report);
    let notice = runtime
        .state
        .opportunity_index()
        .read()
        .and_then(|snapshot| {
            snapshot_notice_if_subscribed(
                &runtime.hub,
                snapshot.report(),
                snapshot.cached_at(),
                snapshot.snapshot_id(),
                top_window,
            )
        });
    let stream_publish_result = notice.as_ref().map_or(Ok(()), |notice| {
        publish_snapshot(&runtime.hub, &runtime.metrics, notice)
    });
    let publish_result = stream_publish_result.and(alert_publish_result);

    runtime
        .metrics
        .record_arb_scan(scan_ms, count, common::time::now_ms());
    log_scan_timing(
        scan_elapsed,
        publish_elapsed,
        runtime.slow_threshold,
        count,
        prewarm,
    );
    drop(guard);
    snapshot_task_outcome(history_append_ok, publish_result)
}

fn acquire_scan_guard(
    runtime: &SnapshotRuntime,
    prewarm: bool,
) -> Result<tokio::sync::OwnedMutexGuard<()>, String> {
    Arc::clone(&runtime.scan_lock)
        .try_lock_owned()
        .map_err(|_| {
            warn!(
                prewarm,
                "arbitrage scan skipped because a scan is already in flight"
            );
            "scan skipped because another scan is already in flight".to_owned()
        })
}

async fn build_scan_report(
    runtime: &SnapshotRuntime,
) -> Result<(shared_types::OpportunityScanReport, std::time::Duration), String> {
    let scan_started_at = chrono::Utc::now();
    let scan_start = std::time::Instant::now();
    let engine = Arc::clone(&runtime.engine);
    let mut report = tokio::task::spawn_blocking(move || engine.run_scan_report())
        .await
        .map_err(|error| format!("arbitrage scan worker failed: {error}"))?;
    let scan_elapsed = scan_start.elapsed();
    let duplicate_count = opportunity::normalize_scan_report(&mut report, scan_started_at);
    if duplicate_count > 0 {
        warn!(
            duplicate_count,
            "duplicate opportunity identities were collapsed and blocked"
        );
    }
    let now_ms = common::time::now_ms();
    runtime
        .instrument_registry
        .apply_listing_gate(&mut report.opportunities, now_ms);
    super::instruments::request_transfer_networks_for_opportunities(
        &runtime.state,
        &report.opportunities,
    );
    report.meta.scan_ms = scan_elapsed.as_millis() as u64;
    Ok((report, scan_elapsed))
}

fn snapshot_task_outcome(
    history_append_ok: bool,
    publish_result: Result<(), String>,
) -> Result<(), String> {
    let mut errors = Vec::new();
    if !history_append_ok {
        errors.push("opportunity history append failed".to_owned());
    }
    if let Err(error) = publish_result {
        errors.push(error);
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}
