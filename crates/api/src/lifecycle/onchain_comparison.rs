use super::tasks::{tick_or_shutdown, BackgroundTasks, ShutdownToken};
use crate::state::AppState;
use tokio::time::MissedTickBehavior;

const QUOTE_SCHEDULER_INTERVAL: std::time::Duration = std::time::Duration::from_millis(250);
const PROJECTION_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);
const BATCH_PROJECTION_INTERVAL: std::time::Duration = std::time::Duration::from_millis(250);
const CEX_RECOVERY_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);
const CEX_INSTRUMENT_RECOVERY_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);
const CEX_INSTRUMENT_RETRY_AFTER_MS: i64 = 30_000;
const WALLET_INVENTORY_SCHEDULER_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);
const REPLENISHMENT_RECONCILIATION_INTERVAL: std::time::Duration =
    std::time::Duration::from_secs(5);

pub(super) fn spawn_worker(state: &AppState, tasks: &mut BackgroundTasks) {
    spawn_quote_worker(state, tasks);
    spawn_projection_worker(state, tasks);
    spawn_batch_projection_worker(state, tasks);
    spawn_cex_recovery_worker(state, tasks);
    spawn_cex_instrument_recovery_worker(state, tasks);
    spawn_wallet_inventory_worker(state, tasks);
    spawn_replenishment_reconciliation_worker(state, tasks);
}

fn spawn_replenishment_reconciliation_worker(state: &AppState, tasks: &mut BackgroundTasks) {
    let shutdown = tasks.shutdown_token();
    let state = state.clone();
    tasks.supervise(
        "onchain-replenishment-reconciliation",
        REPLENISHMENT_RECONCILIATION_INTERVAL.as_millis() as i64,
        move || {
            let state = state.clone();
            let shutdown = shutdown.clone();
            async move { run_replenishment_reconciliation_worker(state, shutdown).await }
        },
    );
}

fn spawn_wallet_inventory_worker(state: &AppState, tasks: &mut BackgroundTasks) {
    let shutdown = tasks.shutdown_token();
    let state = state.clone();
    tasks.supervise(
        "onchain-wallet-inventory",
        WALLET_INVENTORY_SCHEDULER_INTERVAL.as_millis() as i64,
        move || {
            let state = state.clone();
            let shutdown = shutdown.clone();
            async move { run_wallet_inventory_worker(state, shutdown).await }
        },
    );
}

fn spawn_batch_projection_worker(state: &AppState, tasks: &mut BackgroundTasks) {
    let shutdown = tasks.shutdown_token();
    let state = state.clone();
    tasks.supervise(
        "onchain-batch-projection",
        BATCH_PROJECTION_INTERVAL.as_millis() as i64,
        move || {
            let state = state.clone();
            let shutdown = shutdown.clone();
            async move { run_batch_projection_worker(state, shutdown).await }
        },
    );
}

fn spawn_cex_recovery_worker(state: &AppState, tasks: &mut BackgroundTasks) {
    let shutdown = tasks.shutdown_token();
    let state = state.clone();
    tasks.supervise(
        "onchain-cex-recovery",
        CEX_RECOVERY_INTERVAL.as_millis() as i64,
        move || {
            let state = state.clone();
            let shutdown = shutdown.clone();
            async move { run_cex_recovery_worker(state, shutdown).await }
        },
    );
}

fn spawn_cex_instrument_recovery_worker(state: &AppState, tasks: &mut BackgroundTasks) {
    let shutdown = tasks.shutdown_token();
    let state = state.clone();
    tasks.supervise(
        "onchain-cex-instrument-recovery",
        CEX_INSTRUMENT_RECOVERY_INTERVAL.as_millis() as i64,
        move || {
            let state = state.clone();
            let shutdown = shutdown.clone();
            async move { run_cex_instrument_recovery_worker(state, shutdown).await }
        },
    );
}

fn spawn_quote_worker(state: &AppState, tasks: &mut BackgroundTasks) {
    let shutdown = tasks.shutdown_token();
    let state = state.clone();
    tasks.supervise(
        "onchain-cex-comparison",
        QUOTE_SCHEDULER_INTERVAL.as_millis() as i64,
        move || {
            let state = state.clone();
            let shutdown = shutdown.clone();
            async move { run_quote_worker(state, shutdown).await }
        },
    );
}

fn spawn_projection_worker(state: &AppState, tasks: &mut BackgroundTasks) {
    let shutdown = tasks.shutdown_token();
    let state = state.clone();
    tasks.supervise(
        "onchain-cex-projection",
        PROJECTION_INTERVAL.as_millis() as i64,
        move || {
            let state = state.clone();
            let shutdown = shutdown.clone();
            async move { run_projection_worker(state, shutdown).await }
        },
    );
}

async fn run_quote_worker(state: AppState, shutdown: ShutdownToken) {
    let registry = state.task_registry().clone();
    let mut tick = tokio::time::interval(QUOTE_SCHEDULER_INTERVAL);
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    while tick_or_shutdown(&mut tick, &shutdown).await {
        let started_at_ms = common::time::now_ms();
        crate::services::onchain_comparison::refresh_if_due(&state, started_at_ms).await;
        registry.record_result_timed(
            "onchain-cex-comparison",
            started_at_ms,
            Ok::<(), String>(()),
        );
    }
}

async fn run_projection_worker(state: AppState, shutdown: ShutdownToken) {
    let registry = state.task_registry().clone();
    let mut tick = tokio::time::interval(PROJECTION_INTERVAL);
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    while tick_or_shutdown(&mut tick, &shutdown).await {
        let started_at_ms = common::time::now_ms();
        crate::services::onchain_comparison::project_latest_from_ws(&state, started_at_ms).await;
        registry.record_result_timed(
            "onchain-cex-projection",
            started_at_ms,
            Ok::<(), String>(()),
        );
    }
}

async fn run_batch_projection_worker(state: AppState, shutdown: ShutdownToken) {
    let registry = state.task_registry().clone();
    let mut tick = tokio::time::interval(BATCH_PROJECTION_INTERVAL);
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    while tick_or_shutdown(&mut tick, &shutdown).await {
        let started_at_ms = common::time::now_ms();
        crate::services::onchain_comparison::project_batch_latest_from_ws(&state, started_at_ms)
            .await;
        registry.record_result_timed(
            "onchain-batch-projection",
            started_at_ms,
            Ok::<(), String>(()),
        );
    }
}

async fn run_cex_recovery_worker(state: AppState, shutdown: ShutdownToken) {
    let registry = state.task_registry().clone();
    let mut tick = tokio::time::interval(CEX_RECOVERY_INTERVAL);
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    while tick_or_shutdown(&mut tick, &shutdown).await {
        let started_at_ms = common::time::now_ms();
        crate::services::onchain_comparison::recover_batch_cex(&state, started_at_ms).await;
        registry.record_result_timed("onchain-cex-recovery", started_at_ms, Ok::<(), String>(()));
    }
}

async fn run_cex_instrument_recovery_worker(state: AppState, shutdown: ShutdownToken) {
    let task_registry = state.task_registry().clone();
    let mut tick = tokio::time::interval(CEX_INSTRUMENT_RECOVERY_INTERVAL);
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    while tick_or_shutdown(&mut tick, &shutdown).await {
        let started_at_ms = common::time::now_ms();
        let result = recover_selected_cex_instruments(&state, started_at_ms).await;
        task_registry.record_result_timed("onchain-cex-instrument-recovery", started_at_ms, result);
    }
}

async fn recover_selected_cex_instruments(state: &AppState, now_ms: i64) -> Result<(), String> {
    let config = state.onchain_monitor().snapshot().config.clone();
    if !config.enabled {
        return Ok(());
    }
    let Some((base, quote)) = crate::services::spot::split_spot_pair(&config.cex_symbol) else {
        return Err(format!(
            "{} selected CEX symbol is not an explicit Base/Quote pair",
            config.cex_symbol
        ));
    };
    let registry = state.instrument_registry();
    if registry
        .exact_spot_listing_evidence(&config.cex_venue, &base, &quote, now_ms)
        .is_some()
        || !registry.spot_instrument_refresh_retry_due(
            &config.cex_venue,
            now_ms,
            CEX_INSTRUMENT_RETRY_AFTER_MS,
        )
    {
        return Ok(());
    }
    crate::services::instrument_registry::refresh_spot_venue(
        &config.cex_venue,
        state.aggregator(),
        registry,
    )
    .await
}

async fn run_wallet_inventory_worker(state: AppState, shutdown: ShutdownToken) {
    let registry = state.task_registry().clone();
    let mut tick = tokio::time::interval(WALLET_INVENTORY_SCHEDULER_INTERVAL);
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    while tick_or_shutdown(&mut tick, &shutdown).await {
        let started_at_ms = common::time::now_ms();
        crate::services::onchain_comparison::refresh_wallet_inventory_if_due(&state, started_at_ms)
            .await;
        registry.record_result_timed(
            "onchain-wallet-inventory",
            started_at_ms,
            Ok::<(), String>(()),
        );
    }
}

async fn run_replenishment_reconciliation_worker(state: AppState, shutdown: ShutdownToken) {
    let registry = state.task_registry().clone();
    let mut tick = tokio::time::interval(REPLENISHMENT_RECONCILIATION_INTERVAL);
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    while tick_or_shutdown(&mut tick, &shutdown).await {
        let started_at_ms = common::time::now_ms();
        crate::services::onchain_comparison::reconcile_replenishment(&state, started_at_ms).await;
        crate::services::onchain_comparison::reconcile_cross_chain(&state, started_at_ms).await;
        crate::services::onchain_comparison::refresh_cross_chain_accounting(&state, started_at_ms).await;
        crate::services::onchain_comparison::refresh_execution_accounting(&state, started_at_ms)
            .await;
        crate::services::onchain_comparison::refresh_token_approval_receipts(&state, started_at_ms).await;
        registry.record_result_timed(
            "onchain-replenishment-reconciliation",
            started_at_ms,
            Ok::<(), String>(()),
        );
    }
}
