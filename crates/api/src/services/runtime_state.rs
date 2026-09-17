use crate::state::AppState;
use shared_types::RuntimeProblem;

const MEMORY: &str = "memory";
const JSONL_SNAPSHOT: &str = "jsonl_snapshot";
const JSONL_DEGRADED: &str = "jsonl_degraded";
const SQLITE_SNAPSHOT: &str = "sqlite_snapshot";
const SQLITE_DEGRADED: &str = "sqlite_degraded";
const STORE_VOLATILE: &str = "VOLATILE_STATE_STORE";
const STORE_DEGRADED: &str = "DEGRADED_STATE_STORE";
const HISTORY_FALLBACK: &str = "MEMORY_HISTORY_FALLBACK";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuntimeStateInventory {
    pub(crate) history_backend: &'static str,
    pub(crate) history_rows: Option<u64>,
    pub(crate) stores: Vec<RuntimeStateStore>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuntimeStateStore {
    pub(crate) name: &'static str,
    pub(crate) persistence: &'static str,
    pub(crate) count: u64,
}

pub(crate) async fn inventory(state: &AppState) -> RuntimeStateInventory {
    let watchlist_persistence = watchlist_persistence(state);
    let execution_store = state.execution_run_store();
    let close_store = state.close_run_store();
    let stores = vec![
        store("hedge_tickets", state.hedge_tickets().len()),
        store("hedge_previews", state.hedge_previews().len()),
        run_store(
            "execution_runs",
            state.execution_runs().len(),
            execution_store.persistence_configured(),
            execution_store.persistence_degraded(),
        ),
        run_store(
            "close_runs",
            state.close_runs().len(),
            close_store.persistence_configured(),
            close_store.persistence_degraded(),
        ),
        store("missed_opportunities", state.missed_opportunities().len()),
        store_with_persistence(
            "watchlist",
            watchlist_persistence,
            watchlist_count(state).await,
        ),
        store_with_persistence(
            "alert_rules",
            watchlist_persistence,
            alert_rule_count(state).await,
        ),
        store_with_persistence(
            "alert_cooldowns",
            watchlist_persistence,
            state.alert_cooldowns().len(),
        ),
    ];

    RuntimeStateInventory {
        history_backend: state.history_store().backend_name(),
        history_rows: state
            .history_store()
            .approximate_row_count()
            .await
            .map(as_u64),
        stores,
    }
}

pub(crate) fn problems(inventory: &RuntimeStateInventory, now_ms: i64) -> Vec<RuntimeProblem> {
    let mut problems = Vec::new();
    if memory_history_rows(inventory) > 0 {
        problems.push(RuntimeProblem {
            scope: "runtime_state".to_owned(),
            operation: "history_store".to_owned(),
            code: HISTORY_FALLBACK.to_owned(),
            message: format!(
                "history store is using in-memory backend with {} rows; rows will be lost on restart",
                memory_history_rows(inventory)
            ),
            venue: None,
            retry_after_ms: None,
            problem: None,
            observed_at_ms: now_ms,
        });
    }
    problems.extend(
        inventory
            .stores
            .iter()
            .filter_map(|store| store_problem(store, now_ms)),
    );
    problems
}

fn memory_history_rows(inventory: &RuntimeStateInventory) -> u64 {
    if inventory.history_backend == MEMORY {
        inventory.history_rows.unwrap_or(0)
    } else {
        0
    }
}

fn store(name: &'static str, count: usize) -> RuntimeStateStore {
    store_with_persistence(name, MEMORY, count)
}

fn run_store(
    name: &'static str,
    count: usize,
    persistence_configured: bool,
    persistence_degraded: bool,
) -> RuntimeStateStore {
    let persistence = match (persistence_configured, persistence_degraded) {
        (false, _) => MEMORY,
        (true, true) => JSONL_DEGRADED,
        (true, false) => JSONL_SNAPSHOT,
    };
    store_with_persistence(name, persistence, count)
}

fn store_problem(store: &RuntimeStateStore, now_ms: i64) -> Option<RuntimeProblem> {
    let (code, message) = match store.persistence {
        MEMORY if store.count > 0 => (
            STORE_VOLATILE,
            format!(
                "runtime state store '{}' holds {} in-memory records; records will be lost on restart",
                store.name, store.count
            ),
        ),
        JSONL_DEGRADED => (
            STORE_DEGRADED,
            format!(
                "runtime state store '{}' has JSONL replay or append failures; persisted state may be incomplete",
                store.name
            ),
        ),
        _ => return None,
    };
    Some(RuntimeProblem {
        scope: "runtime_state".to_owned(),
        operation: store.name.to_owned(),
        code: code.to_owned(),
        message,
        venue: None,
        retry_after_ms: None,
        problem: None,
        observed_at_ms: now_ms,
    })
}

fn watchlist_persistence(state: &AppState) -> &'static str {
    match state.watchlist_alert_store().health().status {
        shared_types::WatchlistStorageStatus::Ready => SQLITE_SNAPSHOT,
        shared_types::WatchlistStorageStatus::Degraded => SQLITE_DEGRADED,
        shared_types::WatchlistStorageStatus::Disabled => MEMORY,
    }
}

fn store_with_persistence(
    name: &'static str,
    persistence: &'static str,
    count: usize,
) -> RuntimeStateStore {
    RuntimeStateStore {
        name,
        persistence,
        count: as_u64(count),
    }
}

fn as_u64(value: usize) -> u64 {
    value as u64
}

async fn watchlist_count(state: &AppState) -> usize {
    state.watchlist().read().await.len()
}

async fn alert_rule_count(state: &AppState) -> usize {
    state.alert_rules().read().await.len()
}

#[cfg(test)]
#[path = "runtime_state/tests.rs"]
mod tests;
