#[path = "../events.rs"]
mod events;
#[path = "../ledger_store.rs"]
mod ledger_store;
#[path = "../order_store.rs"]
mod order_store;
#[path = "../orderbook.rs"]
mod orderbook;
#[path = "../sql_events.rs"]
mod sql_events;
use crate::ledger::{
    ExecutionLedger, ExecutionLedgerOrderContext, ExecutionLedgerQuery, FillLedgerEventContext,
    FillLedgerInput, FundingLedgerInput, SlippageLedgerInput,
};
use crate::sql_ledger::{
    SqlBalanceLedgerEvent, SqlLedgerInit, SqlLedgerReplay, SqlLedgerReplayHealth,
    SqlLedgerStorageSnapshot, SqlLedgerStore, SqlRealizedWindow, SqlRunFinalityLedgerEvent,
};
use crate::state_machine::transition;
use dashmap::DashMap;
use events::{append_jsonl, event_from_ack_state, event_from_target_state};
use parking_lot::{Mutex, RwLock};
use shared_types::{
    ExecutionLedgerEvent, ExecutionLedgerPayload, FundingPaymentIngestSkipReason, LiveOrderState,
    OrderAck, OrderEventRecord, OrderInfo, OrderIntent, OrderLifecycleEvent, OrderRecord,
    OrderSide, OrderStatus, OrderTransportMetadata, OrderUpdateSource, RiskDecision,
    VenueOrderIdentity,
};
use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionLedgerStorageSnapshot {
    pub configured: bool,
    pub path: Option<String>,
    pub event_count: usize,
    pub replayed_events: usize,
    pub replay_failures: usize,
    pub append_successes: usize,
    pub append_failures: usize,
    pub last_append_at_ms: Option<i64>,
    pub query_successes: usize,
    pub query_failures: usize,
    pub last_query_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderSnapshotStorageSnapshot {
    pub configured: bool,
    pub path: Option<String>,
    pub record_count: usize,
    pub replayed_records: usize,
    pub replay_failures: usize,
    pub append_successes: usize,
    pub append_failures: usize,
    pub last_append_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Copy)]
pub struct FillOrderIdentity<'a> {
    pub venue: Option<&'a str>,
    pub exchange_order_id: Option<&'a str>,
    pub client_order_id: Option<&'a str>,
    pub symbol: Option<&'a str>,
    pub side: Option<OrderSide>,
}

#[derive(Clone, Copy)]
struct FillSlippageRecord<'a> {
    record: &'a OrderRecord,
    fill_event: &'a ExecutionLedgerEvent,
    context: Option<&'a ExecutionLedgerOrderContext>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LedgerSqlWriteMode {
    BestEffort,
    DeferredDurable,
}

#[derive(Clone, Copy)]
struct FillLedgerRecordRequest<'a> {
    identity: FillOrderIdentity<'a>,
    input: &'a FillLedgerInput,
    source: OrderUpdateSource,
    captured_at_ms: i64,
    transport_metadata: Option<&'a OrderTransportMetadata>,
    sql_mode: LedgerSqlWriteMode,
}

#[derive(Clone, Copy)]
struct FundingLedgerRecordRequest<'a> {
    venue: &'a str,
    symbol: &'a str,
    input: &'a FundingLedgerInput,
    source: OrderUpdateSource,
    captured_at_ms: i64,
    sql_mode: LedgerSqlWriteMode,
}

#[derive(Debug)]
pub struct OrderJournal {
    records: DashMap<String, OrderRecord>,
    client_index: DashMap<String, String>,
    exchange_index: DashMap<String, String>,
    ledger_contexts: DashMap<String, ExecutionLedgerOrderContext>,
    execution_ledger: ExecutionLedger,
    events: RwLock<Vec<OrderEventRecord>>,
    ledger_append_lock: Mutex<()>,
    order_snapshot_append_lock: Mutex<()>,
    sql_ledger_store: Option<SqlLedgerStore>,
    sql_ledger_migration_health: crate::sql_ledger::SqlLedgerMigrationHealth,
    sql_ledger_replay_health: SqlLedgerReplayHealth,
    open_count: AtomicUsize,
    audit_path: Option<PathBuf>,
    ledger_path: Option<PathBuf>,
    order_snapshot_path: Option<PathBuf>,
    ledger_replayed_events: AtomicUsize,
    ledger_replay_failures: AtomicUsize,
    ledger_append_successes: AtomicUsize,
    ledger_append_failures: AtomicUsize,
    ledger_last_append_at_ms: AtomicI64,
    ledger_query_successes: AtomicUsize,
    ledger_query_failures: AtomicUsize,
    ledger_last_query_at_ms: AtomicI64,
    order_snapshot_replayed_records: AtomicUsize,
    order_snapshot_replay_failures: AtomicUsize,
    order_snapshot_append_successes: AtomicUsize,
    order_snapshot_append_failures: AtomicUsize,
    order_snapshot_last_append_at_ms: AtomicI64,
}

struct TransitionUpdate<'a> {
    internal_order_id: &'a str,
    event: OrderLifecycleEvent,
    source: OrderUpdateSource,
    at_ms: i64,
    message: Option<String>,
    payload: serde_json::Value,
}

impl Default for OrderJournal {
    fn default() -> Self {
        let audit_path = order_audit_path_from_env();
        let ledger_path = execution_ledger_path_from_env();
        let order_snapshot_path = order_snapshot_path_from_env();
        Self::new_with_paths(audit_path, ledger_path, order_snapshot_path)
    }
}

fn order_audit_path_from_env() -> Option<PathBuf> {
    let value = std::env::var_os("APP_ORDER_AUDIT_PATH")?;
    resolve_runtime_file_path(
        value.to_string_lossy().as_ref(),
        std::env::var_os("APP_STORAGE__DATA_DIR")
            .map(|value| value.to_string_lossy().trim().to_owned()),
    )
}

fn execution_ledger_path_from_env() -> Option<PathBuf> {
    let value = std::env::var_os("APP_STORAGE__EXECUTION_LEDGER_PATH")
        .or_else(|| std::env::var_os("APP_EXECUTION_LEDGER_PATH"))?;
    resolve_runtime_file_path(
        value.to_string_lossy().as_ref(),
        std::env::var_os("APP_STORAGE__DATA_DIR")
            .map(|value| value.to_string_lossy().trim().to_owned()),
    )
}

fn order_snapshot_path_from_env() -> Option<PathBuf> {
    let value = std::env::var_os("APP_STORAGE__ORDER_SNAPSHOT_PATH")
        .or_else(|| std::env::var_os("APP_ORDER_SNAPSHOT_PATH"))?;
    resolve_runtime_file_path(
        value.to_string_lossy().as_ref(),
        std::env::var_os("APP_STORAGE__DATA_DIR")
            .map(|value| value.to_string_lossy().trim().to_owned()),
    )
}

#[cfg(test)]
fn resolve_order_audit_path(value: &str, data_dir: Option<String>) -> Option<PathBuf> {
    resolve_runtime_file_path(value, data_dir)
}

#[cfg(test)]
fn resolve_execution_ledger_path(value: &str, data_dir: Option<String>) -> Option<PathBuf> {
    resolve_runtime_file_path(value, data_dir)
}

fn resolve_runtime_file_path(value: &str, data_dir: Option<String>) -> Option<PathBuf> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let path = PathBuf::from(value);
    if path.is_absolute() {
        return Some(path);
    }
    let mut storage = common::config::StorageConfig::default();
    if let Some(data_dir) = data_dir {
        storage.data_dir = data_dir;
    }
    Some(storage.resolve_runtime_path(value))
}

fn execution_ledger_from_path(path: &Path) -> (ExecutionLedger, usize, usize) {
    match ledger_store::read_jsonl(path) {
        Ok(events) => {
            let replayed_events = events.len();
            (ExecutionLedger::from_events(events), replayed_events, 0)
        }
        Err(err) => {
            tracing::warn!(path = %path.display(), error = %err, "failed to replay execution ledger");
            (ExecutionLedger::default(), 0, 1)
        }
    }
}

impl OrderJournal {
    fn restore_ledger_contexts(&self) {
        let mut latest = BTreeMap::new();
        for event in self.execution_ledger.list() {
            let Some(context) = execution_ledger_context(&event) else {
                continue;
            };
            let internal_order_id = event.order.identity.internal_order_id.clone();
            let timestamp = (event.occurred_at_ms, event.captured_at_ms);
            if latest
                .get(&internal_order_id)
                .is_none_or(|(current, _)| timestamp >= *current)
            {
                latest.insert(internal_order_id, (timestamp, context));
            }
        }
        for (internal_order_id, (_, context)) in latest {
            self.ledger_contexts.insert(internal_order_id, context);
        }
    }
}

fn execution_ledger_context(event: &ExecutionLedgerEvent) -> Option<ExecutionLedgerOrderContext> {
    let run_id = event.order.run_id.as_deref()?.trim();
    let ticket_id = event.order.ticket_id.as_deref()?.trim();
    let leg_role = event.order.leg_role?;
    (!run_id.is_empty() && !ticket_id.is_empty()).then(|| {
        ExecutionLedgerOrderContext::new(run_id.to_owned(), ticket_id.to_owned(), leg_role)
    })
}

fn order_snapshots_from_path(path: &Path) -> order_store::OrderSnapshotReplay {
    match order_store::read_jsonl(path) {
        Ok(replay) => replay,
        Err(err) => {
            tracing::warn!(path = %path.display(), error = %err, "failed to replay order snapshots");
            order_store::OrderSnapshotReplay {
                rows: Vec::new(),
                failed_lines: 1,
            }
        }
    }
}
