use super::*;
use common::config::AppConfig;
use realtime::WatchlistItem;
use shared_types::{CloseRun, CloseRunScope, CloseRunStatus};

#[tokio::test]
async fn empty_memory_history_does_not_degrade_fresh_state() {
    let state = new_state().await;
    let inv = inventory(&state).await;

    assert_eq!(inv.history_backend, "memory");
    assert_eq!(inv.history_rows, Some(0));
    assert!(problems(&inv, 1_700_000_000_000).is_empty());
}

#[tokio::test]
async fn non_empty_memory_store_becomes_runtime_problem() {
    let state = new_state().await;
    state.watchlist().write().await.push(WatchlistItem {
        id: 1,
        symbol: "BTCUSDT".to_owned(),
        venue_long: None,
        venue_short: None,
        min_net_yield: None,
        min_volume_24h: None,
        enabled: true,
        created_at_ms: 1_700_000_000_000,
        persistence: shared_types::WatchlistPersistence::default(),
        runtime: shared_types::WatchlistItemRuntime::default(),
    });

    let inv = inventory(&state).await;
    let problems = problems(&inv, 1_700_000_000_001);

    assert!(problems
        .iter()
        .any(|problem| { problem.operation == "watchlist" && problem.code == STORE_VOLATILE }));
}

#[tokio::test]
async fn unconfigured_close_runs_are_reported_as_volatile_runtime_state() {
    let state = new_state().await;
    state
        .close_runs()
        .insert("close-1".to_owned(), close_run("close-1"));

    let inv = inventory(&state).await;

    assert_eq!(
        inv.stores
            .iter()
            .find(|store| store.name == "close_runs")
            .map(|store| (store.persistence, store.count)),
        Some((MEMORY, 1))
    );
    assert!(problems(&inv, 1_700_000_000_001)
        .iter()
        .any(|problem| { problem.operation == "close_runs" && problem.code == STORE_VOLATILE }));
}

#[tokio::test]
async fn configured_close_runs_are_reported_as_durable_runtime_state() -> anyhow::Result<()> {
    let path = temp_path("close-runs", "jsonl");
    let mut config = base_config();
    config.storage.close_run_ledger_path = Some(path.display().to_string());
    let state = AppState::new(config).await?;
    state
        .close_runs()
        .insert("close-1".to_owned(), close_run("close-1"));

    let inv = inventory(&state).await;

    assert_eq!(
        inv.stores
            .iter()
            .find(|store| store.name == "close_runs")
            .map(|store| (store.persistence, store.count)),
        Some((JSONL_SNAPSHOT, 1))
    );
    assert!(problems(&inv, 1_700_000_000_001)
        .iter()
        .all(|problem| problem.operation != "close_runs"));
    drop(state);
    let _ = std::fs::remove_file(path);
    Ok(())
}

#[tokio::test]
async fn degraded_execution_run_store_becomes_runtime_problem() -> anyhow::Result<()> {
    let path = temp_path("execution-runs-degraded", "jsonl");
    std::fs::write(&path, b"{bad json}\n")?;
    let mut config = base_config();
    config.storage.execution_run_ledger_path = Some(path.display().to_string());
    let state = AppState::new(config).await?;

    let inv = inventory(&state).await;

    assert_eq!(
        inv.stores
            .iter()
            .find(|store| store.name == "execution_runs")
            .map(|store| store.persistence),
        Some(JSONL_DEGRADED)
    );
    assert!(problems(&inv, 1_700_000_000_001).iter().any(|problem| {
        problem.operation == "execution_runs" && problem.code == STORE_DEGRADED
    }));
    drop(state);
    let _ = std::fs::remove_file(path);
    Ok(())
}

#[tokio::test]
async fn persisted_watchlist_state_is_not_reported_as_volatile() -> anyhow::Result<()> {
    let path = std::env::temp_dir().join(format!(
        "crossline-runtime-watchlist-alerts-{}-{}.sqlite",
        std::process::id(),
        common::time::now_ms()
    ));
    let mut config = AppConfig::default();
    config.api_surface.watchlist_alerts = true;
    config.storage.watchlist_alerts_path = Some(path.display().to_string());
    let state = AppState::new(config).await?;
    crate::services::watchlist_alerts::create_watchlist_item(
        &state,
        WatchlistItem {
            id: 0,
            symbol: "BTCUSDT".to_owned(),
            venue_long: Some("binance".to_owned()),
            venue_short: Some("okx".to_owned()),
            min_net_yield: None,
            min_volume_24h: None,
            enabled: true,
            created_at_ms: 0,
            persistence: shared_types::WatchlistPersistence::default(),
            runtime: shared_types::WatchlistItemRuntime::default(),
        },
        "api-token:test",
    )
    .await?;

    let inv = inventory(&state).await;
    assert_eq!(
        inv.stores
            .iter()
            .find(|store| store.name == "watchlist")
            .map(|store| (store.persistence, store.count)),
        Some((SQLITE_SNAPSHOT, 1))
    );
    assert!(problems(&inv, common::time::now_ms())
        .iter()
        .all(|problem| problem.operation != "watchlist"));

    drop(state);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
    let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    Ok(())
}

#[test]
fn durable_store_does_not_become_volatile_problem() {
    let inv = RuntimeStateInventory {
        history_backend: MEMORY,
        history_rows: Some(0),
        stores: vec![run_store("execution_runs", 1, true, false)],
    };

    let problems = problems(&inv, 1_700_000_000_001);

    assert!(problems.is_empty());
}

#[allow(clippy::panic)]
async fn new_state() -> AppState {
    let config = base_config();
    match AppState::new(config).await {
        Ok(state) => state,
        Err(error) => panic!("state init failed: {error:?}"),
    }
}

fn base_config() -> AppConfig {
    let mut config = AppConfig::default();
    config.history.enabled = true;
    config.storage.portfolio_nav_path = None;
    config.storage.execution_run_ledger_path = None;
    config.storage.close_run_ledger_path = None;
    config
}

fn temp_path(label: &str, extension: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "crossline-runtime-{label}-{}-{}.{}",
        std::process::id(),
        common::time::now_ms(),
        extension
    ))
}

fn close_run(id: &str) -> CloseRun {
    CloseRun {
        id: id.to_owned(),
        scope: CloseRunScope::Single,
        status: CloseRunStatus::Submitted,
        action_run_id: None,
        request_id: None,
        idempotency_key: None,
        snapshot_version: "pos-1".to_owned(),
        expected_leg_count: 0,
        reason: None,
        legs: Vec::new(),
        submitted_order_count: 0,
        failed_leg_count: 0,
        naked_exposure_usd: 0.0,
        message: "submitted".to_owned(),
        problem: None,
        finality_problem: None,
        finality_checked_at_ms: None,
        unwind_plan: None,
        cost_events: Vec::new(),
        cost_reconciliation: None,
        started_at_ms: 1,
        updated_at_ms: 1,
    }
}
