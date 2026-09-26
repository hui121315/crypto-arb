use crate::ledger::ExecutionLedger;
use parking_lot::RwLock;
use shared_types::{
    CloseRun, ExecutionLedgerEvent, ExecutionLedgerPayload, ExecutionRun, FeeLedgerSnapshot,
    FillLedgerSnapshot, FundingPaymentLedgerRecord, OrderRecord, OrderUpdateSource,
    OrderbookDepthLedgerRecord, SlippageLedgerRecord, StorageBackendKind, StorageDegradedReason,
    StorageMigrationAuthority, StorageRuntimeContract, VenueBalanceInfo, VenueOrderIdentity,
};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tokio_postgres::{GenericClient, NoTls};

mod json_canonical;
mod migrations;
mod projection_jobs;
mod run_cost;
mod run_cost_rebuild;
mod run_finality;
mod writer;

use json_canonical::parse_json_with_exact_floats;
pub use projection_jobs::{
    SqlProjectionJob, SqlProjectionJobAck, CLOSE_RUN_PROJECTOR, EXECUTION_RUN_PROJECTOR,
    RUN_COST_PROJECTOR,
};
pub use run_cost::SqlRunCostFact;
pub use run_cost_rebuild::SqlRunCostRebuildReport;
use writer::{EventWriteError, SqlLedgerWrite};
pub use writer::{SqlLedgerPersistAck, SqlLedgerWriteError};

pub const SQL_LEDGER_SCHEMA_VERSION: u32 = 5;
pub const SQL_LEDGER_MIGRATION_VERSION: u32 = migrations::latest().schema_version as u32;
pub const SQL_LEDGER_MIGRATION_ID: &str = migrations::latest().id;
pub const SQL_LEDGER_MIGRATION_PATH: &str = migrations::latest().path;
pub const SQL_LEDGER_SCHEMA_NAME: &str = migrations::latest().schema_name;

#[cfg(test)]
const SQL_LEDGER_SCHEMA_SQL: &str = migrations::ordered()[0].sql;
const SQL_LEDGER_MIGRATION_TIMEOUT: Duration = Duration::from_millis(3_500);
const SQL_LEDGER_REPLAY_TIMEOUT: Duration = Duration::from_millis(3_500);
const SQL_LEDGER_WRITE_QUEUE_CAPACITY: usize = 4096;
const SQL_LEDGER_REPLAY_LIMIT: i64 = 50_000;
const SQL_ORDER_EVENTS_REPLAY_QUERY: &str = "\
    SELECT event_id, internal_order_id, client_order_id, exchange_order_id, \
           public_client_order_id, venue_client_order_id, run_id, ticket_id, leg_role, \
           exchange, symbol, side, order_ref, event_type, source, state, event, payload, \
           payload::text AS payload_text, payload_hash, schema_version, occurred_at_ms, captured_at_ms \
    FROM ( \
        SELECT id, event_id, internal_order_id, client_order_id, exchange_order_id, \
               public_client_order_id, venue_client_order_id, run_id, ticket_id, leg_role, \
               exchange, symbol, side, order_ref, event_type, source, state, event, payload, \
               payload_hash, schema_version, occurred_at_ms, captured_at_ms \
        FROM order_events \
        ORDER BY occurred_at_ms DESC, id DESC LIMIT $1 \
    ) latest_order_events \
    ORDER BY occurred_at_ms ASC, id ASC";
const SQL_ORDER_SNAPSHOTS_REPLAY_QUERY: &str = "\
    SELECT internal_order_id, public_client_order_id, venue_client_order_id, exchange_order_id, \
           state, last_update_source, exchange, symbol, side, record, updated_at_ms, \
           record::text AS record_text, record_hash, schema_version, captured_at_ms \
    FROM ( \
        SELECT internal_order_id, public_client_order_id, venue_client_order_id, \
               exchange_order_id, state, last_update_source, exchange, symbol, side, record, \
               updated_at_ms, record_hash, schema_version, captured_at_ms \
        FROM order_snapshots \
        ORDER BY updated_at_ms DESC, internal_order_id DESC LIMIT $1 \
    ) latest_order_snapshots \
    ORDER BY updated_at_ms ASC, internal_order_id ASC";
const SQL_BALANCE_EVENTS_REPLAY_QUERY: &str = "\
    SELECT event_id, exchange, asset, balance_kind, payload, payload::text AS payload_text, payload_hash, \
           observed_at_ms, captured_at_ms \
    FROM ( \
        SELECT event_id, exchange, asset, balance_kind, payload, payload_hash, \
               observed_at_ms, captured_at_ms \
        FROM balance_events \
        ORDER BY observed_at_ms DESC, event_id DESC LIMIT $1 \
    ) latest_balance_events \
    ORDER BY observed_at_ms ASC, event_id ASC";
const SQL_RUN_FINALITY_REPLAY_QUERY: &str = "\
    SELECT event_id, run_kind, run_id, source_event_id, source_order_event_id, \
           source, state, payload, payload::text AS payload_text, payload_hash, schema_version, \
           occurred_at_ms, captured_at_ms \
    FROM ( \
        SELECT id, event_id, run_kind, run_id, source_event_id, source_order_event_id, \
               source, state, payload, payload_hash, schema_version, occurred_at_ms, captured_at_ms \
        FROM run_finality_events \
        ORDER BY occurred_at_ms DESC, id DESC LIMIT $1 \
    ) latest_run_finality_events \
    ORDER BY occurred_at_ms ASC, id ASC";
const SQL_REALIZED_CLOSE_RUNS_QUERY: &str = "\
    SELECT event_id, run_kind, run_id, source_event_id, source_order_event_id, \
           source, state, payload, payload::text AS payload_text, payload_hash, schema_version, \
           occurred_at_ms, captured_at_ms \
    FROM ( \
        SELECT id, event_id, run_kind, run_id, source_event_id, source_order_event_id, \
               source, state, payload, payload_hash, schema_version, occurred_at_ms, captured_at_ms \
        FROM run_finality_events \
        WHERE run_kind = 'close_run' AND occurred_at_ms < $1 \
        ORDER BY occurred_at_ms DESC, id DESC LIMIT $2 \
    ) latest_realized_close_runs \
    ORDER BY occurred_at_ms ASC, id ASC";
const FNV64_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
const FNV64_PRIME: u64 = 0x100000001b3;
const RUN_PROJECTORS: &[&str] = &[EXECUTION_RUN_PROJECTOR, CLOSE_RUN_PROJECTOR];
const RUN_AND_COST_PROJECTORS: &[&str] = &[
    EXECUTION_RUN_PROJECTOR,
    CLOSE_RUN_PROJECTOR,
    RUN_COST_PROJECTOR,
];
const CLOSE_RUN_PROJECTORS: &[&str] = &[CLOSE_RUN_PROJECTOR];
const CLOSE_RUN_AND_COST_PROJECTORS: &[&str] = &[CLOSE_RUN_PROJECTOR, RUN_COST_PROJECTOR];
const NO_PROJECTORS: &[&str] = &[];
const INSERT_PROJECTION_JOB_SQL: &str = "\
    INSERT INTO ledger_projection_jobs \
    (event_id, projector, payload_hash, available_at_ms, created_at_ms, updated_at_ms) \
    VALUES ($1, $2, $3, $4, $4, $4) \
    ON CONFLICT (event_id, projector) DO NOTHING";

#[derive(Debug, Clone)]
pub struct SqlLedgerInit {
    pub migration_health: SqlLedgerMigrationHealth,
    pub replay: SqlLedgerReplay,
    pub store: Option<SqlLedgerStore>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqlLedgerMigrationHealth {
    pub configured: bool,
    pub migration_id: &'static str,
    pub migration_path: &'static str,
    pub migration_checksum: String,
    pub schema_version: Option<u32>,
    pub applied: bool,
    pub degraded_reason: Option<StorageDegradedReason>,
    pub last_success_at_ms: Option<i64>,
    pub last_error_at_ms: Option<i64>,
    pub last_error: Option<String>,
    pub observed_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqlLedgerStorageSnapshot {
    pub migration: SqlLedgerMigrationHealth,
    pub writer_configured: bool,
    pub replay_query_successes: usize,
    pub replay_query_failures: usize,
    pub replay_event_rows: usize,
    pub replayed_events: usize,
    pub replay_event_failures: usize,
    pub replay_snapshot_rows: usize,
    pub replayed_order_snapshots: usize,
    pub replay_snapshot_failures: usize,
    pub replay_balance_rows: usize,
    pub replayed_balance_events: usize,
    pub replay_balance_failures: usize,
    pub replay_run_finality_rows: usize,
    pub replayed_run_finality_events: usize,
    pub replay_run_finality_failures: usize,
    pub replay_limited: bool,
    pub replay_limit: usize,
    pub last_replay_at_ms: Option<i64>,
    pub last_replay_error_at_ms: Option<i64>,
    pub last_replay_error: Option<String>,
    pub event_append_successes: usize,
    pub event_append_failures: usize,
    pub snapshot_append_successes: usize,
    pub snapshot_append_failures: usize,
    pub balance_append_successes: usize,
    pub balance_append_failures: usize,
    pub run_finality_append_successes: usize,
    pub run_finality_append_failures: usize,
    pub dropped_writes: usize,
    pub last_append_at_ms: Option<i64>,
    pub last_error_at_ms: Option<i64>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SqlLedgerReplay {
    pub events: Vec<ExecutionLedgerEvent>,
    pub order_snapshots: Vec<OrderRecord>,
    pub balance_events: Vec<SqlBalanceLedgerReplayEvent>,
    pub run_finality_events: Vec<SqlRunFinalityReplayEvent>,
    pub health: SqlLedgerReplayHealth,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SqlRealizedWindow {
    pub events: Vec<ExecutionLedgerEvent>,
    pub order_snapshots: Vec<OrderRecord>,
    pub close_runs: Vec<CloseRun>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SqlLedgerReplayHealth {
    pub query_successes: usize,
    pub query_failures: usize,
    pub event_rows: usize,
    pub replayed_events: usize,
    pub event_decode_failures: usize,
    pub snapshot_rows: usize,
    pub replayed_order_snapshots: usize,
    pub snapshot_decode_failures: usize,
    pub balance_rows: usize,
    pub replayed_balance_events: usize,
    pub balance_decode_failures: usize,
    pub run_finality_rows: usize,
    pub replayed_run_finality_events: usize,
    pub run_finality_decode_failures: usize,
    pub replay_limited: bool,
    pub replay_limit: usize,
    pub last_query_at_ms: Option<i64>,
    pub last_error_at_ms: Option<i64>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SqlLedgerStore {
    url: Arc<str>,
    sender: mpsc::Sender<SqlLedgerWrite>,
    control: Arc<writer::WriterControl>,
    stats: Arc<SqlLedgerWriteStats>,
    migration_health: SqlLedgerMigrationHealth,
    replay_health: SqlLedgerReplayHealth,
    /// realized window 读路径的常驻连接（懒建、断线自愈）。此前每次调用
    /// `tokio_postgres::connect` 新建 TCP 连接，而该查询被 2s portfolio loop
    /// 每 tick 触发——等于对 Postgres 的持续连接风暴。
    reader: Arc<tokio::sync::Mutex<Option<Arc<tokio_postgres::Client>>>>,
}

#[derive(Debug, Default)]
struct SqlLedgerWriteStats {
    event_append_successes: AtomicUsize,
    event_append_failures: AtomicUsize,
    snapshot_append_successes: AtomicUsize,
    snapshot_append_failures: AtomicUsize,
    balance_append_successes: AtomicUsize,
    balance_append_failures: AtomicUsize,
    run_finality_append_successes: AtomicUsize,
    run_finality_append_failures: AtomicUsize,
    dropped_writes: AtomicUsize,
    last_append_at_ms: AtomicI64,
    last_error_at_ms: AtomicI64,
    last_error: RwLock<Option<String>>,
}

#[derive(Clone, Debug)]
struct SqlEventRow {
    event_id: String,
    internal_order_id: String,
    client_order_id: String,
    exchange_order_id: Option<String>,
    public_client_order_id: String,
    venue_client_order_id: Option<String>,
    run_id: Option<String>,
    ticket_id: Option<String>,
    leg_role: Option<String>,
    exchange: String,
    symbol: String,
    side: String,
    order_ref: serde_json::Value,
    event_type: String,
    source: String,
    state: Option<String>,
    lifecycle_event: Option<String>,
    payload: serde_json::Value,
    payload_text: Option<String>,
    payload_hash: String,
    schema_version: i32,
    occurred_at_ms: i64,
    captured_at_ms: i64,
}

#[derive(Clone)]
struct SqlOrderSnapshotRow {
    internal_order_id: String,
    public_client_order_id: String,
    venue_client_order_id: Option<String>,
    exchange_order_id: Option<String>,
    state: String,
    last_update_source: String,
    exchange: String,
    symbol: String,
    side: String,
    record: serde_json::Value,
    record_text: Option<String>,
    updated_at_ms: i64,
    record_hash: String,
    schema_version: i32,
    captured_at_ms: i64,
}

struct SqlFillRow {
    event_id: String,
    internal_order_id: String,
    exchange: String,
    symbol: String,
    side: String,
    run_id: Option<String>,
    ticket_id: Option<String>,
    leg_role: Option<String>,
    source: String,
    fill_kind: String,
    quantity: f64,
    average_price: f64,
    quote_value: f64,
    quality: String,
    fill_confidence: String,
    fill_confidence_score: f64,
    fee_amount: Option<f64>,
    fee_currency: Option<String>,
    fee_quality: Option<String>,
    payload_hash: String,
    occurred_at_ms: i64,
    captured_at_ms: i64,
}

struct SqlFeeRow {
    event_id: String,
    internal_order_id: String,
    exchange: String,
    symbol: String,
    side: String,
    run_id: Option<String>,
    ticket_id: Option<String>,
    leg_role: Option<String>,
    source: String,
    fee_origin: &'static str,
    amount: f64,
    currency: Option<String>,
    quality: String,
    payload_hash: String,
    occurred_at_ms: i64,
    captured_at_ms: i64,
}

struct SqlFundingPaymentRow {
    event_id: String,
    internal_order_id: String,
    exchange: String,
    symbol: String,
    side: String,
    run_id: Option<String>,
    ticket_id: Option<String>,
    leg_role: Option<String>,
    source: String,
    amount: f64,
    currency: String,
    funding_time_ms: i64,
    quality: String,
    payload_hash: String,
    occurred_at_ms: i64,
    captured_at_ms: i64,
}

struct SqlSlippageRow {
    event_id: String,
    internal_order_id: String,
    exchange: String,
    symbol: String,
    side: String,
    run_id: Option<String>,
    ticket_id: Option<String>,
    leg_role: Option<String>,
    source: String,
    amount_usd: f64,
    reference_price: f64,
    fill_price: f64,
    quantity: f64,
    quality: String,
    payload_hash: String,
    occurred_at_ms: i64,
    captured_at_ms: i64,
}

struct SqlOrderbookEvidenceRow {
    event_id: String,
    internal_order_id: String,
    exchange: String,
    symbol: String,
    side: String,
    run_id: Option<String>,
    ticket_id: Option<String>,
    leg_role: Option<String>,
    source: String,
    reference_price: Option<f64>,
    bid: Option<f64>,
    ask: Option<f64>,
    mid: Option<f64>,
    open_vwap_price: Option<f64>,
    open_slippage_bps: Option<f64>,
    close_vwap_price: Option<f64>,
    close_slippage_bps: Option<f64>,
    depth_usd_5bps: Option<f64>,
    depth_usd_10bps: Option<f64>,
    depth_usd_20bps: Option<f64>,
    max_notional_usd: Option<f64>,
    market_timestamp_ms: Option<i64>,
    health: Option<serde_json::Value>,
    reason: Option<String>,
    evidence_quality: String,
    payload_hash: String,
    occurred_at_ms: i64,
    captured_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SqlBalanceLedgerEvent {
    event_id: String,
    exchange: String,
    asset: String,
    balance_kind: String,
    payload: serde_json::Value,
    payload_text: Option<String>,
    payload_hash: String,
    observed_at_ms: i64,
    captured_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SqlBalanceLedgerReplayEvent {
    pub event_id: String,
    pub balance_kind: String,
    pub row: VenueBalanceInfo,
    pub observed_at_ms: i64,
    pub captured_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SqlRunFinalityLedgerEvent {
    event_id: String,
    run_kind: String,
    run_id: String,
    source_event_id: Option<String>,
    source_order_event_id: Option<String>,
    source: String,
    state: String,
    payload: serde_json::Value,
    payload_hash: String,
    occurred_at_ms: i64,
    captured_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SqlRunFinalityReplayEvent {
    pub event_id: String,
    pub run_kind: String,
    pub run_id: String,
    pub source_event_id: Option<String>,
    pub source_order_event_id: Option<String>,
    pub source: String,
    pub state: String,
    pub payload: serde_json::Value,
    pub payload_hash: String,
    pub schema_version: i32,
    pub occurred_at_ms: i64,
    pub captured_at_ms: i64,
}

pub fn sql_ledger_schema_hash() -> String {
    migrations::latest().checksum.to_owned()
}

pub async fn init_sql_ledger_store(database_url: Option<&str>) -> SqlLedgerInit {
    let migration_health = run_sql_ledger_migration(database_url).await;
    let Some(url) = normalized_database_url(database_url) else {
        return SqlLedgerInit {
            migration_health,
            replay: SqlLedgerReplay::default(),
            store: None,
        };
    };
    if !migration_health.applied {
        return SqlLedgerInit {
            migration_health,
            replay: SqlLedgerReplay::default(),
            store: None,
        };
    }
    let replay = replay_sql_ledger(url).await;
    let store = SqlLedgerStore::connect(url, migration_health.clone(), replay.health.clone()).await;
    SqlLedgerInit {
        migration_health,
        replay,
        store,
    }
}

pub async fn run_sql_ledger_migration(database_url: Option<&str>) -> SqlLedgerMigrationHealth {
    let Some(url) = normalized_database_url(database_url) else {
        return SqlLedgerMigrationHealth::unconfigured(common::time::now_ms());
    };
    let applied_at_ms = common::time::now_ms();
    let result = tokio::time::timeout(
        SQL_LEDGER_MIGRATION_TIMEOUT,
        apply_sql_ledger_migration(url, applied_at_ms),
    )
    .await;
    match result {
        Ok(Ok(())) => SqlLedgerMigrationHealth::applied(common::time::now_ms()),
        Ok(Err(error)) => SqlLedgerMigrationHealth::failed(common::time::now_ms(), error),
        Err(_) => SqlLedgerMigrationHealth::failed(
            common::time::now_ms(),
            migrations::MigrationFailure::migration("trading SQL migration timed out"),
        ),
    }
}

fn normalized_database_url(database_url: Option<&str>) -> Option<&str> {
    database_url.map(str::trim).filter(|url| !url.is_empty())
}

async fn apply_sql_ledger_migration(
    url: &str,
    applied_at_ms: i64,
) -> Result<(), migrations::MigrationFailure> {
    let (mut client, connection) = tokio_postgres::connect(url, NoTls).await.map_err(|error| {
        migrations::MigrationFailure::unavailable(format!("postgres connect failed: {error}"))
    })?;
    tokio::spawn(async move {
        if let Err(error) = connection.await {
            tracing::warn!(%error, "trading SQL ledger postgres connection failed");
        }
    });
    migrations::apply_all(&mut client, applied_at_ms).await
}

impl SqlLedgerInit {
    pub fn unconfigured(observed_at_ms: i64) -> Self {
        Self {
            migration_health: SqlLedgerMigrationHealth::unconfigured(observed_at_ms),
            replay: SqlLedgerReplay::default(),
            store: None,
        }
    }
}

impl SqlLedgerStore {
    async fn connect(
        url: &str,
        migration_health: SqlLedgerMigrationHealth,
        replay_health: SqlLedgerReplayHealth,
    ) -> Option<Self> {
        let (client, connection) = match tokio_postgres::connect(url, NoTls).await {
            Ok(pair) => pair,
            Err(error) => {
                tracing::warn!(%error, "trading SQL ledger writer connect failed");
                return None;
            }
        };
        tokio::spawn(async move {
            if let Err(error) = connection.await {
                tracing::warn!(%error, "trading SQL ledger writer postgres connection failed");
            }
        });
        let (sender, receiver) = mpsc::channel(SQL_LEDGER_WRITE_QUEUE_CAPACITY);
        let stats = Arc::new(SqlLedgerWriteStats::default());
        let control = Arc::new(writer::WriterControl::new());
        tokio::spawn(writer::run(
            client,
            receiver,
            Arc::clone(&stats),
            Arc::clone(&control),
        ));
        Some(Self {
            url: Arc::from(url.to_owned()),
            sender,
            control,
            stats,
            migration_health,
            replay_health,
            reader: Arc::new(tokio::sync::Mutex::new(None)),
        })
    }

    async fn reader_client(&self) -> Result<Arc<tokio_postgres::Client>, String> {
        let mut guard = self.reader.lock().await;
        if let Some(client) = guard.as_ref() {
            if !client.is_closed() {
                return Ok(Arc::clone(client));
            }
        }
        let (client, connection) = tokio_postgres::connect(self.url.as_ref(), NoTls)
            .await
            .map_err(|error| format!("postgres realized query connect failed: {error}"))?;
        tokio::spawn(async move {
            if let Err(error) = connection.await {
                tracing::warn!(%error, "trading SQL ledger reader connection failed");
            }
        });
        let client = Arc::new(client);
        *guard = Some(Arc::clone(&client));
        Ok(client)
    }

    pub fn append_event(&self, event: &ExecutionLedgerEvent) {
        self.try_send(SqlLedgerWrite::Event(Box::new(event.clone())));
    }

    pub async fn persist_event(
        &self,
        event: &ExecutionLedgerEvent,
    ) -> Result<SqlLedgerPersistAck, SqlLedgerWriteError> {
        let (ack, receiver) = oneshot::channel();
        self.send_durable(SqlLedgerWrite::DurableEvent {
            event: Box::new(event.clone()),
            ack,
        })
        .await?;
        match tokio::time::timeout(writer::ACK_TIMEOUT, receiver).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(SqlLedgerWriteError::AckClosed),
            Err(_) => Err(SqlLedgerWriteError::AckTimeout),
        }
    }

    pub async fn persist_event_group(
        &self,
        events: &[ExecutionLedgerEvent],
    ) -> Result<Vec<SqlLedgerPersistAck>, SqlLedgerWriteError> {
        if events.is_empty() {
            return Ok(Vec::new());
        }
        let (ack, receiver) = oneshot::channel();
        self.send_durable(SqlLedgerWrite::DurableEventGroup {
            events: events.to_vec(),
            ack,
        })
        .await?;
        match tokio::time::timeout(writer::ACK_TIMEOUT, receiver).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(SqlLedgerWriteError::AckClosed),
            Err(_) => Err(SqlLedgerWriteError::AckTimeout),
        }
    }

    pub async fn claim_projection_jobs(
        &self,
        projector: &str,
        now_ms: i64,
        limit: usize,
        lease_ms: i64,
    ) -> Result<Vec<SqlProjectionJob>, SqlLedgerWriteError> {
        let limit = projection_jobs::validate_claim_request(projector, now_ms, limit, lease_ms)?;
        let (ack, receiver) = oneshot::channel();
        self.send_durable(SqlLedgerWrite::ClaimProjectionJobs {
            projector: projector.to_owned(),
            now_ms,
            limit,
            lease_ms,
            ack,
        })
        .await?;
        wait_for_writer_ack(receiver).await
    }

    pub async fn complete_projection_job(
        &self,
        job: &SqlProjectionJob,
        completed_at_ms: i64,
    ) -> Result<SqlProjectionJobAck, SqlLedgerWriteError> {
        let (ack, receiver) = oneshot::channel();
        self.send_durable(SqlLedgerWrite::CompleteProjectionJob {
            event_id: job.event_id.clone(),
            projector: job.projector.clone(),
            claim_token: job.claim_token.clone(),
            completed_at_ms,
            ack,
        })
        .await?;
        wait_for_writer_ack(receiver).await
    }

    pub async fn retry_projection_job(
        &self,
        job: &SqlProjectionJob,
        available_at_ms: i64,
        last_error: &str,
    ) -> Result<SqlProjectionJobAck, SqlLedgerWriteError> {
        let (ack, receiver) = oneshot::channel();
        self.send_durable(SqlLedgerWrite::RetryProjectionJob {
            event_id: job.event_id.clone(),
            projector: job.projector.clone(),
            claim_token: job.claim_token.clone(),
            available_at_ms,
            last_error: last_error.to_owned(),
            updated_at_ms: common::time::now_ms(),
            ack,
        })
        .await?;
        wait_for_writer_ack(receiver).await
    }

    pub fn append_order_snapshot(&self, record: &OrderRecord) {
        self.try_send(SqlLedgerWrite::OrderSnapshot(Box::new(record.clone())));
    }

    pub fn append_balance_event(&self, event: SqlBalanceLedgerEvent) {
        self.try_send(SqlLedgerWrite::BalanceEvent(Box::new(event)));
    }

    pub fn append_run_finality_event(&self, event: SqlRunFinalityLedgerEvent) {
        self.try_send(SqlLedgerWrite::RunFinality(Box::new(event)));
    }

    pub async fn persist_run_finality_event(
        &self,
        event: &SqlRunFinalityLedgerEvent,
    ) -> Result<SqlLedgerPersistAck, SqlLedgerWriteError> {
        let (ack, receiver) = oneshot::channel();
        self.send_durable(SqlLedgerWrite::DurableRunFinality {
            event: Box::new(event.clone()),
            ack,
        })
        .await?;
        wait_for_writer_ack(receiver).await
    }

    pub async fn rebuild_run_cost_facts(
        &self,
        page_size: usize,
    ) -> Result<SqlRunCostRebuildReport, SqlLedgerWriteError> {
        run_cost_rebuild::rebuild(self.url.as_ref(), page_size).await
    }

    pub async fn project_run_cost_event(
        &self,
        event: &ExecutionLedgerEvent,
    ) -> Result<usize, SqlLedgerWriteError> {
        let (ack, receiver) = oneshot::channel();
        self.send_durable(SqlLedgerWrite::ProjectRunCost {
            event: Box::new(event.clone()),
            ack,
        })
        .await?;
        wait_for_writer_ack(receiver).await
    }

    pub async fn query_run_cost_facts(
        &self,
        run_kind: &str,
        run_id: &str,
    ) -> Result<Vec<SqlRunCostFact>, SqlLedgerWriteError> {
        run_cost::query(self.url.as_ref(), run_kind, run_id).await
    }

    pub async fn drain(&self) -> Result<(), SqlLedgerWriteError> {
        let _drain = self.control.begin_drain()?;
        let (ack, receiver) = oneshot::channel();
        self.send_and_wait_barrier(SqlLedgerWrite::Drain(ack), receiver)
            .await
    }

    pub async fn shutdown(&self) -> Result<(), SqlLedgerWriteError> {
        loop {
            match self.control.begin_shutdown() {
                writer::ShutdownStart::Done => return Ok(()),
                writer::ShutdownStart::Lead => return self.shutdown_writer().await,
                writer::ShutdownStart::WaitForDrain => self.control.wait_for_drain().await?,
                writer::ShutdownStart::WaitForStop => self.control.wait_for_shutdown().await?,
            }
        }
    }

    pub async fn query_realized_window(
        &self,
        from_ms: i64,
        to_ms: i64,
    ) -> Result<SqlRealizedWindow, String> {
        let client = self.reader_client().await?;
        query_sql_realized_window(&client, from_ms, to_ms).await
    }

    pub fn snapshot(&self) -> SqlLedgerStorageSnapshot {
        self.stats.snapshot(
            self.migration_health.clone(),
            self.replay_health.clone(),
            true,
        )
    }

    fn try_send(&self, write: SqlLedgerWrite) {
        if let Err(error) = self.control.try_send(&self.sender, write) {
            self.stats
                .record_dropped_write(&format!("sql writer queue failed: {error}"));
        }
    }

    async fn send_durable(&self, write: SqlLedgerWrite) -> Result<(), SqlLedgerWriteError> {
        self.control
            .send(&self.sender, write)
            .await
            .inspect_err(|error| {
                self.stats.record_dropped_write(&error.to_string());
            })
    }

    async fn shutdown_writer(&self) -> Result<(), SqlLedgerWriteError> {
        let mut leader = self.control.shutdown_leader_guard();
        let (ack, receiver) = oneshot::channel();
        if let Err(error) = writer::send_barrier(&self.sender, SqlLedgerWrite::Shutdown(ack)).await
        {
            self.stats.record_dropped_write(&error.to_string());
            return Err(error);
        }
        leader.command_enqueued();
        match tokio::time::timeout(writer::ACK_TIMEOUT, receiver).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(_)) if self.control.begin_shutdown() == writer::ShutdownStart::Done => Ok(()),
            Ok(Err(_)) => Err(SqlLedgerWriteError::AckClosed),
            Err(_) => Err(SqlLedgerWriteError::AckTimeout),
        }
    }

    async fn send_and_wait_barrier(
        &self,
        write: SqlLedgerWrite,
        receiver: oneshot::Receiver<()>,
    ) -> Result<(), SqlLedgerWriteError> {
        writer::send_barrier(&self.sender, write).await?;
        match tokio::time::timeout(writer::ACK_TIMEOUT, receiver).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(_)) => Err(SqlLedgerWriteError::AckClosed),
            Err(_) => Err(SqlLedgerWriteError::AckTimeout),
        }
    }
}

async fn wait_for_writer_ack<T>(
    receiver: oneshot::Receiver<Result<T, SqlLedgerWriteError>>,
) -> Result<T, SqlLedgerWriteError> {
    match tokio::time::timeout(writer::ACK_TIMEOUT, receiver).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err(SqlLedgerWriteError::AckClosed),
        Err(_) => Err(SqlLedgerWriteError::AckTimeout),
    }
}

impl SqlLedgerStorageSnapshot {
    pub fn unconfigured(migration: SqlLedgerMigrationHealth) -> Self {
        Self::without_writer(migration, SqlLedgerReplayHealth::default())
    }

    pub fn without_writer(
        migration: SqlLedgerMigrationHealth,
        replay_health: SqlLedgerReplayHealth,
    ) -> Self {
        SqlLedgerWriteStats::default().snapshot(migration, replay_health, false)
    }
}

impl SqlLedgerWriteStats {
    fn snapshot(
        &self,
        migration: SqlLedgerMigrationHealth,
        replay: SqlLedgerReplayHealth,
        writer_configured: bool,
    ) -> SqlLedgerStorageSnapshot {
        SqlLedgerStorageSnapshot {
            migration,
            writer_configured,
            replay_query_successes: replay.query_successes,
            replay_query_failures: replay.query_failures,
            replay_event_rows: replay.event_rows,
            replayed_events: replay.replayed_events,
            replay_event_failures: replay.event_decode_failures,
            replay_snapshot_rows: replay.snapshot_rows,
            replayed_order_snapshots: replay.replayed_order_snapshots,
            replay_snapshot_failures: replay.snapshot_decode_failures,
            replay_balance_rows: replay.balance_rows,
            replayed_balance_events: replay.replayed_balance_events,
            replay_balance_failures: replay.balance_decode_failures,
            replay_run_finality_rows: replay.run_finality_rows,
            replayed_run_finality_events: replay.replayed_run_finality_events,
            replay_run_finality_failures: replay.run_finality_decode_failures,
            replay_limited: replay.replay_limited,
            replay_limit: replay.replay_limit,
            last_replay_at_ms: replay.last_query_at_ms,
            last_replay_error_at_ms: replay.last_error_at_ms,
            last_replay_error: replay.last_error,
            event_append_successes: self.event_append_successes.load(Ordering::Acquire),
            event_append_failures: self.event_append_failures.load(Ordering::Acquire),
            snapshot_append_successes: self.snapshot_append_successes.load(Ordering::Acquire),
            snapshot_append_failures: self.snapshot_append_failures.load(Ordering::Acquire),
            balance_append_successes: self.balance_append_successes.load(Ordering::Acquire),
            balance_append_failures: self.balance_append_failures.load(Ordering::Acquire),
            run_finality_append_successes: self
                .run_finality_append_successes
                .load(Ordering::Acquire),
            run_finality_append_failures: self.run_finality_append_failures.load(Ordering::Acquire),
            dropped_writes: self.dropped_writes.load(Ordering::Acquire),
            last_append_at_ms: non_zero_ms(self.last_append_at_ms.load(Ordering::Acquire)),
            last_error_at_ms: non_zero_ms(self.last_error_at_ms.load(Ordering::Acquire)),
            last_error: self.last_error.read().clone(),
        }
    }

    fn record_event_success(&self) {
        self.event_append_successes.fetch_add(1, Ordering::AcqRel);
        self.record_success_time();
    }

    fn record_snapshot_success(&self) {
        self.snapshot_append_successes
            .fetch_add(1, Ordering::AcqRel);
        self.record_success_time();
    }

    fn record_event_error(&self, error: &str) {
        self.record_event_errors(1, error);
    }

    fn record_event_errors(&self, count: usize, error: &str) {
        self.event_append_failures
            .fetch_add(count, Ordering::AcqRel);
        self.record_error(error);
    }

    fn record_snapshot_error(&self, error: &str) {
        self.snapshot_append_failures.fetch_add(1, Ordering::AcqRel);
        self.record_error(error);
    }

    fn record_balance_success(&self) {
        self.balance_append_successes.fetch_add(1, Ordering::AcqRel);
        self.record_success_time();
    }

    fn record_balance_error(&self, error: &str) {
        self.balance_append_failures.fetch_add(1, Ordering::AcqRel);
        self.record_error(error);
    }

    fn record_run_finality_success(&self) {
        self.run_finality_append_successes
            .fetch_add(1, Ordering::AcqRel);
        self.record_success_time();
    }

    fn record_run_finality_error(&self, error: &str) {
        self.run_finality_append_failures
            .fetch_add(1, Ordering::AcqRel);
        self.record_error(error);
    }

    fn record_dropped_write(&self, error: &str) {
        self.dropped_writes.fetch_add(1, Ordering::AcqRel);
        self.record_error(error);
    }

    fn record_success_time(&self) {
        self.last_append_at_ms
            .store(common::time::now_ms(), Ordering::Release);
    }

    fn record_error(&self, error: &str) {
        self.last_error_at_ms
            .store(common::time::now_ms(), Ordering::Release);
        *self.last_error.write() = Some(error.to_owned());
    }
}

impl SqlLedgerReplay {
    fn with_limit(limit: usize) -> Self {
        Self {
            events: Vec::new(),
            order_snapshots: Vec::new(),
            balance_events: Vec::new(),
            run_finality_events: Vec::new(),
            health: SqlLedgerReplayHealth {
                replay_limit: limit,
                ..SqlLedgerReplayHealth::default()
            },
        }
    }

    fn failed(error: String) -> Self {
        let mut replay = Self::with_limit(SQL_LEDGER_REPLAY_LIMIT as usize);
        replay.health.record_query_error(error);
        replay
    }
}

impl SqlLedgerReplayHealth {
    fn record_query_success(&mut self) {
        self.query_successes = self.query_successes.saturating_add(1);
        self.last_query_at_ms = Some(common::time::now_ms());
    }

    fn record_query_error(&mut self, error: String) {
        self.query_failures = self.query_failures.saturating_add(1);
        self.last_error_at_ms = Some(common::time::now_ms());
        self.last_error = Some(error);
    }

    fn record_event_decode_error(&mut self, error: String) {
        self.event_decode_failures = self.event_decode_failures.saturating_add(1);
        self.last_error_at_ms = Some(common::time::now_ms());
        self.last_error = Some(error);
    }

    fn record_snapshot_decode_error(&mut self, error: String) {
        self.snapshot_decode_failures = self.snapshot_decode_failures.saturating_add(1);
        self.last_error_at_ms = Some(common::time::now_ms());
        self.last_error = Some(error);
    }

    fn record_balance_decode_error(&mut self, error: String) {
        self.balance_decode_failures = self.balance_decode_failures.saturating_add(1);
        self.last_error_at_ms = Some(common::time::now_ms());
        self.last_error = Some(error);
    }

    fn record_run_finality_decode_error(&mut self, error: String) {
        self.run_finality_decode_failures = self.run_finality_decode_failures.saturating_add(1);
        self.last_error_at_ms = Some(common::time::now_ms());
        self.last_error = Some(error);
    }
}

async fn replay_sql_ledger(url: &str) -> SqlLedgerReplay {
    match tokio::time::timeout(SQL_LEDGER_REPLAY_TIMEOUT, replay_sql_ledger_inner(url)).await {
        Ok(replay) => replay,
        Err(_) => SqlLedgerReplay::failed("trading SQL ledger replay timed out".to_owned()),
    }
}

async fn replay_sql_ledger_inner(url: &str) -> SqlLedgerReplay {
    let (client, connection) = match tokio_postgres::connect(url, NoTls).await {
        Ok(pair) => pair,
        Err(error) => {
            return SqlLedgerReplay::failed(format!("postgres replay connect failed: {error}"));
        }
    };
    tokio::spawn(async move {
        if let Err(error) = connection.await {
            tracing::warn!(%error, "trading SQL ledger replay postgres connection failed");
        }
    });
    let mut replay = SqlLedgerReplay::with_limit(SQL_LEDGER_REPLAY_LIMIT as usize);
    read_replay_events(&client, &mut replay).await;
    read_replay_order_snapshots(&client, &mut replay).await;
    read_replay_balance_events(&client, &mut replay).await;
    read_replay_run_finality_events(&client, &mut replay).await;
    replay
}

async fn read_replay_events(client: &tokio_postgres::Client, replay: &mut SqlLedgerReplay) {
    let rows = query_replay_payloads(client, SQL_ORDER_EVENTS_REPLAY_QUERY).await;
    let rows = match rows {
        Ok(rows) => rows,
        Err(error) => {
            replay
                .health
                .record_query_error(format!("order_events replay query failed: {error}"));
            return;
        }
    };
    replay.health.record_query_success();
    replay.health.event_rows = replay_rows_len(rows.len());
    replay.health.replay_limited |= replay_rows_limited(rows.len());
    let skip = newest_replay_rows_to_skip(rows.len());
    for row in rows.into_iter().skip(skip) {
        push_replay_event_row(replay, sql_event_replay_row(&row));
    }
}

async fn read_replay_order_snapshots(
    client: &tokio_postgres::Client,
    replay: &mut SqlLedgerReplay,
) {
    let rows = query_replay_payloads(client, SQL_ORDER_SNAPSHOTS_REPLAY_QUERY).await;
    let rows = match rows {
        Ok(rows) => rows,
        Err(error) => {
            replay
                .health
                .record_query_error(format!("order_snapshots replay query failed: {error}"));
            return;
        }
    };
    replay.health.record_query_success();
    replay.health.snapshot_rows = replay_rows_len(rows.len());
    replay.health.replay_limited |= replay_rows_limited(rows.len());
    let skip = newest_replay_rows_to_skip(rows.len());
    for row in rows.into_iter().skip(skip) {
        push_replay_order_snapshot_row(replay, sql_order_snapshot_replay_row(&row));
    }
}

async fn read_replay_balance_events(client: &tokio_postgres::Client, replay: &mut SqlLedgerReplay) {
    let rows = query_replay_payloads(client, SQL_BALANCE_EVENTS_REPLAY_QUERY).await;
    let rows = match rows {
        Ok(rows) => rows,
        Err(error) => {
            replay
                .health
                .record_query_error(format!("balance_events replay query failed: {error}"));
            return;
        }
    };
    replay.health.record_query_success();
    replay.health.balance_rows = replay_rows_len(rows.len());
    replay.health.replay_limited |= replay_rows_limited(rows.len());
    let skip = newest_replay_rows_to_skip(rows.len());
    for row in rows.into_iter().skip(skip) {
        push_replay_balance_event(replay, &row);
    }
}

async fn read_replay_run_finality_events(
    client: &tokio_postgres::Client,
    replay: &mut SqlLedgerReplay,
) {
    let rows = query_replay_payloads(client, SQL_RUN_FINALITY_REPLAY_QUERY).await;
    let rows = match rows {
        Ok(rows) => rows,
        Err(error) => {
            replay
                .health
                .record_query_error(format!("run_finality_events replay query failed: {error}"));
            return;
        }
    };
    replay.health.record_query_success();
    replay.health.run_finality_rows = replay_rows_len(rows.len());
    replay.health.replay_limited |= replay_rows_limited(rows.len());
    let skip = newest_replay_rows_to_skip(rows.len());
    for row in rows.into_iter().skip(skip) {
        push_replay_run_finality_event(replay, &row);
    }
}

async fn query_replay_payloads(
    client: &tokio_postgres::Client,
    sql: &str,
) -> Result<Vec<tokio_postgres::Row>, tokio_postgres::Error> {
    let limit = SQL_LEDGER_REPLAY_LIMIT.saturating_add(1);
    client.query(sql, &[&limit]).await
}

async fn query_sql_realized_window(
    client: &tokio_postgres::Client,
    from_ms: i64,
    to_ms: i64,
) -> Result<SqlRealizedWindow, String> {
    if to_ms <= from_ms {
        return Ok(SqlRealizedWindow::default());
    }
    let limit = SQL_LEDGER_REPLAY_LIMIT.saturating_add(1);
    // occurred_at_ms 同时带上下界：让索引做范围扫描，而不是固定取最新 5 万条
    // 再在内存里丢弃窗口外的行（活跃系统里每次白白反序列化几万条 JSONB）。
    let rows = client
        .query(
            "SELECT payload FROM order_events \
             WHERE occurred_at_ms < $1 \
             AND occurred_at_ms >= $2 \
             AND event_type IN ('fill_snapshot', 'fill_event', 'fee_snapshot', 'funding_payment', 'slippage', 'orderbook_evidence') \
             ORDER BY occurred_at_ms DESC, id DESC LIMIT $3",
            &[&to_ms, &from_ms, &limit],
        )
        .await
        .map_err(|error| format!("order_events realized query failed: {error}"))?;
    let mut events = Vec::with_capacity(replay_rows_len(rows.len()));
    for row in rows.into_iter().take(SQL_LEDGER_REPLAY_LIMIT as usize) {
        events.push(sql_event_payload_from_row(&row)?);
    }
    let events = sql_realized_window_events_from_events(events, from_ms, to_ms);
    let order_snapshots = query_sql_realized_order_snapshots(client, &events).await?;
    let close_runs = query_sql_realized_close_runs(client, &events, to_ms).await?;
    Ok(SqlRealizedWindow {
        events,
        order_snapshots,
        close_runs,
    })
}

async fn query_sql_realized_order_snapshots(
    client: &tokio_postgres::Client,
    events: &[ExecutionLedgerEvent],
) -> Result<Vec<OrderRecord>, String> {
    let order_ids = event_order_ids(events).into_iter().collect::<Vec<_>>();
    if order_ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows = client
        .query(
            "SELECT record FROM order_snapshots \
             WHERE internal_order_id = ANY($1) \
             ORDER BY updated_at_ms DESC, internal_order_id ASC",
            &[&order_ids],
        )
        .await
        .map_err(|error| format!("order_snapshots realized query failed: {error}"))?;
    let mut snapshots = Vec::with_capacity(rows.len());
    for row in rows {
        snapshots.push(sql_order_snapshot_record_from_row(&row)?);
    }
    Ok(snapshots)
}

async fn query_sql_realized_close_runs(
    client: &tokio_postgres::Client,
    events: &[ExecutionLedgerEvent],
    to_ms: i64,
) -> Result<Vec<CloseRun>, String> {
    if realized_close_run_link_keys(events).is_empty() {
        return Ok(Vec::new());
    }
    let limit = SQL_LEDGER_REPLAY_LIMIT.saturating_add(1);
    let rows = client
        .query(SQL_REALIZED_CLOSE_RUNS_QUERY, &[&to_ms, &limit])
        .await
        .map_err(|error| format!("run_finality_events realized query failed: {error}"))?;
    let finality_events = rows
        .into_iter()
        .take(SQL_LEDGER_REPLAY_LIMIT as usize)
        .map(|row| sql_run_finality_replay_event(&row));
    let finality_events = collect_realized_run_finality_events(finality_events)?;
    Ok(sql_realized_close_runs_from_finality_events(
        events,
        finality_events,
    ))
}

fn collect_realized_run_finality_events(
    events: impl IntoIterator<Item = Result<SqlRunFinalityReplayEvent, String>>,
) -> Result<Vec<SqlRunFinalityReplayEvent>, String> {
    events.into_iter().collect()
}

fn sql_event_payload_from_row(row: &tokio_postgres::Row) -> Result<ExecutionLedgerEvent, String> {
    let payload = row
        .try_get::<_, serde_json::Value>("payload")
        .map_err(|error| format!("order_events payload read failed: {error}"))?;
    serde_json::from_value::<ExecutionLedgerEvent>(payload)
        .map_err(|error| format!("order_events payload decode failed: {error}"))
}

fn sql_order_snapshot_record_from_row(row: &tokio_postgres::Row) -> Result<OrderRecord, String> {
    let record = row
        .try_get::<_, serde_json::Value>("record")
        .map_err(|error| format!("order_snapshots record read failed: {error}"))?;
    serde_json::from_value::<OrderRecord>(record)
        .map_err(|error| format!("order_snapshots record decode failed: {error}"))
}

fn sql_realized_window_events_from_events(
    events: Vec<ExecutionLedgerEvent>,
    from_ms: i64,
    to_ms: i64,
) -> Vec<ExecutionLedgerEvent> {
    ExecutionLedger::from_events(events).realized_window_events(from_ms, to_ms)
}

fn sql_realized_close_runs_from_finality_events(
    events: &[ExecutionLedgerEvent],
    finality_events: Vec<SqlRunFinalityReplayEvent>,
) -> Vec<CloseRun> {
    let link_keys = realized_close_run_link_keys(events);
    if link_keys.is_empty() {
        return Vec::new();
    }
    let mut runs = BTreeMap::<String, CloseRun>::new();
    for event in finality_events {
        if event.run_kind != "close_run" {
            continue;
        }
        let Ok(run) = serde_json::from_value::<CloseRun>(event.payload) else {
            continue;
        };
        if close_run_matches_keys(&run, &link_keys) {
            upsert_latest_close_run(&mut runs, run);
        }
    }
    runs.into_values().collect()
}

fn realized_close_run_link_keys(events: &[ExecutionLedgerEvent]) -> BTreeSet<(String, String)> {
    events
        .iter()
        .filter_map(|event| Some((event.order.run_id.clone()?, event.order.ticket_id.clone()?)))
        .collect()
}

fn close_run_matches_keys(run: &CloseRun, keys: &BTreeSet<(String, String)>) -> bool {
    run.legs
        .iter()
        .filter_map(|leg| leg.pair_evidence.as_ref())
        .any(|pair| keys.contains(&(pair.run_id.clone(), pair.ticket_id.clone())))
}

fn upsert_latest_close_run(runs: &mut BTreeMap<String, CloseRun>, candidate: CloseRun) {
    match runs.get(&candidate.id) {
        Some(current) if current.updated_at_ms > candidate.updated_at_ms => {}
        _ => {
            runs.insert(candidate.id.clone(), candidate);
        }
    }
}

#[cfg(test)]
fn sql_realized_window_from_parts(
    events: Vec<ExecutionLedgerEvent>,
    order_snapshots: Vec<OrderRecord>,
    from_ms: i64,
    to_ms: i64,
) -> SqlRealizedWindow {
    let events = sql_realized_window_events_from_events(events, from_ms, to_ms);
    let order_ids = event_order_ids(&events);
    let order_snapshots = order_snapshots
        .into_iter()
        .filter(|record| order_ids.contains(record.intent.id.as_str()))
        .collect();
    SqlRealizedWindow {
        events,
        order_snapshots,
        close_runs: Vec::new(),
    }
}

fn event_order_ids(events: &[ExecutionLedgerEvent]) -> BTreeSet<String> {
    events
        .iter()
        .map(|event| event.order.identity.internal_order_id.clone())
        .collect()
}

fn replay_rows_len(len: usize) -> usize {
    len.min(SQL_LEDGER_REPLAY_LIMIT as usize)
}

fn replay_rows_limited(len: usize) -> bool {
    len > SQL_LEDGER_REPLAY_LIMIT as usize
}

fn newest_replay_rows_to_skip(len: usize) -> usize {
    len.saturating_sub(SQL_LEDGER_REPLAY_LIMIT as usize)
}

fn push_replay_event_row(replay: &mut SqlLedgerReplay, row: Result<SqlEventRow, String>) {
    let event = row.and_then(|row| sql_execution_event_from_replay_row(&row));
    match event {
        Ok(event) => {
            replay.events.push(event);
            replay.health.replayed_events = replay.events.len();
        }
        Err(error) => replay.health.record_event_decode_error(error),
    }
}

fn push_replay_order_snapshot_row(
    replay: &mut SqlLedgerReplay,
    row: Result<SqlOrderSnapshotRow, String>,
) {
    let record = row.and_then(|row| sql_order_snapshot_from_replay_row(&row));
    match record {
        Ok(record) => {
            replay.order_snapshots.push(record);
            replay.health.replayed_order_snapshots = replay.order_snapshots.len();
        }
        Err(error) => replay.health.record_snapshot_decode_error(error),
    }
}

fn sql_event_replay_row(row: &tokio_postgres::Row) -> Result<SqlEventRow, String> {
    Ok(SqlEventRow {
        event_id: replay_row_string(row, "order_events", "event_id")?,
        internal_order_id: replay_row_string(row, "order_events", "internal_order_id")?,
        client_order_id: replay_row_string(row, "order_events", "client_order_id")?,
        exchange_order_id: replay_row_optional_string(row, "order_events", "exchange_order_id")?,
        public_client_order_id: replay_row_string(row, "order_events", "public_client_order_id")?,
        venue_client_order_id: replay_row_optional_string(
            row,
            "order_events",
            "venue_client_order_id",
        )?,
        run_id: replay_row_optional_string(row, "order_events", "run_id")?,
        ticket_id: replay_row_optional_string(row, "order_events", "ticket_id")?,
        leg_role: replay_row_optional_string(row, "order_events", "leg_role")?,
        exchange: replay_row_string(row, "order_events", "exchange")?,
        symbol: replay_row_string(row, "order_events", "symbol")?,
        side: replay_row_string(row, "order_events", "side")?,
        order_ref: replay_row_json(row, "order_events", "order_ref")?,
        event_type: replay_row_string(row, "order_events", "event_type")?,
        source: replay_row_string(row, "order_events", "source")?,
        state: replay_row_optional_string(row, "order_events", "state")?,
        lifecycle_event: replay_row_optional_string(row, "order_events", "event")?,
        payload: replay_row_json(row, "order_events", "payload")?,
        payload_text: Some(replay_row_string(row, "order_events", "payload_text")?),
        payload_hash: replay_row_string(row, "order_events", "payload_hash")?,
        schema_version: replay_row_i32(row, "order_events", "schema_version")?,
        occurred_at_ms: replay_row_i64(row, "order_events", "occurred_at_ms")?,
        captured_at_ms: replay_row_i64(row, "order_events", "captured_at_ms")?,
    })
}

fn sql_execution_event_from_replay_row(row: &SqlEventRow) -> Result<ExecutionLedgerEvent, String> {
    let event = decode_typed_replay_payload::<ExecutionLedgerEvent>(
        "order_events",
        &row.payload,
        row.payload_text.as_deref(),
        &row.payload_hash,
    )?;
    let expected = sql_event_row(&event)?;
    ensure_replay_schema("order_events", row.schema_version)?;
    if !sql_event_identity_matches(row, &expected) {
        return Err("order_events payload identity does not match stored columns".to_owned());
    }
    Ok(event)
}

fn sql_event_identity_matches(row: &SqlEventRow, expected: &SqlEventRow) -> bool {
    row.event_id == expected.event_id
        && row.internal_order_id == expected.internal_order_id
        && row.client_order_id == expected.client_order_id
        && row.exchange_order_id == expected.exchange_order_id
        && row.public_client_order_id == expected.public_client_order_id
        && row.venue_client_order_id == expected.venue_client_order_id
        && row.run_id == expected.run_id
        && row.ticket_id == expected.ticket_id
        && row.leg_role == expected.leg_role
        && row.exchange == expected.exchange
        && row.symbol == expected.symbol
        && row.side == expected.side
        && row.order_ref == expected.order_ref
        && row.event_type == expected.event_type
        && row.source == expected.source
        && row.state == expected.state
        && row.lifecycle_event == expected.lifecycle_event
        && row.occurred_at_ms == expected.occurred_at_ms
        && row.captured_at_ms == expected.captured_at_ms
}

fn sql_order_snapshot_replay_row(row: &tokio_postgres::Row) -> Result<SqlOrderSnapshotRow, String> {
    Ok(SqlOrderSnapshotRow {
        internal_order_id: replay_row_string(row, "order_snapshots", "internal_order_id")?,
        public_client_order_id: replay_row_string(
            row,
            "order_snapshots",
            "public_client_order_id",
        )?,
        venue_client_order_id: replay_row_optional_string(
            row,
            "order_snapshots",
            "venue_client_order_id",
        )?,
        exchange_order_id: replay_row_optional_string(row, "order_snapshots", "exchange_order_id")?,
        state: replay_row_string(row, "order_snapshots", "state")?,
        last_update_source: replay_row_string(row, "order_snapshots", "last_update_source")?,
        exchange: replay_row_string(row, "order_snapshots", "exchange")?,
        symbol: replay_row_string(row, "order_snapshots", "symbol")?,
        side: replay_row_string(row, "order_snapshots", "side")?,
        record: replay_row_json(row, "order_snapshots", "record")?,
        record_text: Some(replay_row_string(row, "order_snapshots", "record_text")?),
        updated_at_ms: replay_row_i64(row, "order_snapshots", "updated_at_ms")?,
        record_hash: replay_row_string(row, "order_snapshots", "record_hash")?,
        schema_version: replay_row_i32(row, "order_snapshots", "schema_version")?,
        captured_at_ms: replay_row_i64(row, "order_snapshots", "captured_at_ms")?,
    })
}

fn sql_order_snapshot_from_replay_row(row: &SqlOrderSnapshotRow) -> Result<OrderRecord, String> {
    let record = decode_typed_replay_payload::<OrderRecord>(
        "order_snapshots",
        &row.record,
        row.record_text.as_deref(),
        &row.record_hash,
    )?;
    let expected = sql_order_snapshot_row(&record)?;
    ensure_replay_schema("order_snapshots", row.schema_version)?;
    if !sql_order_snapshot_identity_matches(row, &expected) {
        return Err("order_snapshots record identity does not match stored columns".to_owned());
    }
    Ok(record)
}

fn sql_order_snapshot_identity_matches(
    row: &SqlOrderSnapshotRow,
    expected: &SqlOrderSnapshotRow,
) -> bool {
    row.internal_order_id == expected.internal_order_id
        && row.public_client_order_id == expected.public_client_order_id
        && row.venue_client_order_id == expected.venue_client_order_id
        && row.exchange_order_id == expected.exchange_order_id
        && row.state == expected.state
        && row.last_update_source == expected.last_update_source
        && row.exchange == expected.exchange
        && row.symbol == expected.symbol
        && row.side == expected.side
        && row.updated_at_ms == expected.updated_at_ms
}

fn push_replay_balance_event(replay: &mut SqlLedgerReplay, row: &tokio_postgres::Row) {
    let event = sql_balance_replay_row(row).and_then(sql_balance_replay_event);
    push_replay_balance_result(replay, event);
}

fn push_replay_balance_result(
    replay: &mut SqlLedgerReplay,
    event: Result<SqlBalanceLedgerReplayEvent, String>,
) {
    match event {
        Ok(event) => {
            replay.balance_events.push(event);
            replay.health.replayed_balance_events = replay.balance_events.len();
        }
        Err(error) => replay.health.record_balance_decode_error(error),
    }
}

fn push_replay_run_finality_event(replay: &mut SqlLedgerReplay, row: &tokio_postgres::Row) {
    push_replay_run_finality_result(replay, sql_run_finality_replay_event(row));
}

fn push_replay_run_finality_result(
    replay: &mut SqlLedgerReplay,
    event: Result<SqlRunFinalityReplayEvent, String>,
) {
    match event {
        Ok(event) => {
            replay.run_finality_events.push(event);
            replay.health.replayed_run_finality_events = replay.run_finality_events.len();
        }
        Err(error) => replay.health.record_run_finality_decode_error(error),
    }
}

fn sql_run_finality_replay_event(
    row: &tokio_postgres::Row,
) -> Result<SqlRunFinalityReplayEvent, String> {
    let payload_text = replay_row_string(row, "run_finality_events", "payload_text")?;
    let event = SqlRunFinalityReplayEvent {
        event_id: replay_row_string(row, "run_finality_events", "event_id")?,
        run_kind: replay_row_string(row, "run_finality_events", "run_kind")?,
        run_id: replay_row_string(row, "run_finality_events", "run_id")?,
        source_event_id: replay_row_optional_string(row, "run_finality_events", "source_event_id")?,
        source_order_event_id: replay_row_optional_string(
            row,
            "run_finality_events",
            "source_order_event_id",
        )?,
        source: replay_row_string(row, "run_finality_events", "source")?,
        state: replay_row_string(row, "run_finality_events", "state")?,
        payload: replay_row_json(row, "run_finality_events", "payload")?,
        payload_hash: replay_row_string(row, "run_finality_events", "payload_hash")?,
        schema_version: replay_row_i32(row, "run_finality_events", "schema_version")?,
        occurred_at_ms: replay_row_i64(row, "run_finality_events", "occurred_at_ms")?,
        captured_at_ms: replay_row_i64(row, "run_finality_events", "captured_at_ms")?,
    };
    validate_run_finality_replay_event_with_text(event, Some(&payload_text))
}

fn replay_row_string(
    row: &tokio_postgres::Row,
    table: &str,
    column: &str,
) -> Result<String, String> {
    row.try_get::<_, String>(column)
        .map_err(|error| format!("{table} {column} read failed: {error}"))
}

fn replay_row_optional_string(
    row: &tokio_postgres::Row,
    table: &str,
    column: &str,
) -> Result<Option<String>, String> {
    row.try_get::<_, Option<String>>(column)
        .map_err(|error| format!("{table} {column} read failed: {error}"))
}

fn replay_row_json(
    row: &tokio_postgres::Row,
    table: &str,
    column: &str,
) -> Result<serde_json::Value, String> {
    row.try_get::<_, serde_json::Value>(column)
        .map_err(|error| format!("{table} {column} read failed: {error}"))
}

fn replay_row_i32(row: &tokio_postgres::Row, table: &str, column: &str) -> Result<i32, String> {
    row.try_get::<_, i32>(column)
        .map_err(|error| format!("{table} {column} read failed: {error}"))
}

fn replay_row_i64(row: &tokio_postgres::Row, table: &str, column: &str) -> Result<i64, String> {
    row.try_get::<_, i64>(column)
        .map_err(|error| format!("{table} {column} read failed: {error}"))
}

fn sql_balance_replay_row(row: &tokio_postgres::Row) -> Result<SqlBalanceLedgerEvent, String> {
    Ok(SqlBalanceLedgerEvent {
        event_id: replay_row_string(row, "balance_events", "event_id")?,
        exchange: replay_row_string(row, "balance_events", "exchange")?,
        asset: replay_row_string(row, "balance_events", "asset")?,
        balance_kind: replay_row_string(row, "balance_events", "balance_kind")?,
        payload: replay_row_json(row, "balance_events", "payload")?,
        payload_text: Some(replay_row_string(row, "balance_events", "payload_text")?),
        payload_hash: replay_row_string(row, "balance_events", "payload_hash")?,
        observed_at_ms: replay_row_i64(row, "balance_events", "observed_at_ms")?,
        captured_at_ms: replay_row_i64(row, "balance_events", "captured_at_ms")?,
    })
}

fn sql_balance_replay_event(
    event: SqlBalanceLedgerEvent,
) -> Result<SqlBalanceLedgerReplayEvent, String> {
    let row = decode_typed_replay_payload::<VenueBalanceInfo>(
        "balance_events",
        &event.payload,
        event.payload_text.as_deref(),
        &event.payload_hash,
    )?;
    let expected_event_id = balance_event_id(
        &row,
        &event.balance_kind,
        event.observed_at_ms,
        &event.payload_hash,
    );
    if row.venue != event.exchange
        || row.currency != event.asset
        || event.event_id != expected_event_id
    {
        return Err("balance_events payload identity does not match stored columns".to_owned());
    }
    Ok(SqlBalanceLedgerReplayEvent {
        event_id: event.event_id,
        balance_kind: event.balance_kind,
        row,
        observed_at_ms: event.observed_at_ms,
        captured_at_ms: event.captured_at_ms,
    })
}

#[cfg(test)]
fn validate_run_finality_replay_event(
    event: SqlRunFinalityReplayEvent,
) -> Result<SqlRunFinalityReplayEvent, String> {
    validate_run_finality_replay_event_with_text(event, None)
}

fn validate_run_finality_replay_event_with_text(
    mut event: SqlRunFinalityReplayEvent,
    payload_text: Option<&str>,
) -> Result<SqlRunFinalityReplayEvent, String> {
    ensure_replay_schema("run_finality_events", event.schema_version)?;
    let (identity_matches, canonical_payload) = match event.run_kind.as_str() {
        "execution_run" => {
            let run = decode_typed_replay_payload::<ExecutionRun>(
                "run_finality_events",
                &event.payload,
                payload_text,
                &event.payload_hash,
            )?;
            let matches =
                run.run_id == event.run_id && serde_label(&run.state) == Ok(event.state.clone());
            let payload = serde_json::to_value(&run)
                .map_err(|error| format!("run_finality_events payload encode failed: {error}"))?;
            (matches, payload)
        }
        "close_run" => {
            let run = decode_typed_replay_payload::<CloseRun>(
                "run_finality_events",
                &event.payload,
                payload_text,
                &event.payload_hash,
            )?;
            let matches =
                run.id == event.run_id && serde_label(&run.status) == Ok(event.state.clone());
            let payload = serde_json::to_value(&run)
                .map_err(|error| format!("run_finality_events payload encode failed: {error}"))?;
            (matches, payload)
        }
        _ => return Err("run_finality_events run kind is unsupported".to_owned()),
    };
    let event_id_input = RunFinalityEventIdInput {
        run_kind: &event.run_kind,
        run_id: &event.run_id,
        state: &event.state,
        source: &event.source,
        source_event_id: event.source_event_id.as_deref(),
        source_order_event_id: event.source_order_event_id.as_deref(),
        occurred_at_ms: event.occurred_at_ms,
        payload_hash: &event.payload_hash,
    };
    let expected_event_id = run_finality_event_id(&event_id_input);
    let legacy_event_id = legacy_run_finality_event_id(&event_id_input);
    let unlinked_legacy_id_matches = event.source_event_id.is_none()
        && event.source_order_event_id.is_none()
        && event.event_id == legacy_event_id;
    if !identity_matches || (event.event_id != expected_event_id && !unlinked_legacy_id_matches) {
        return Err(
            "run_finality_events payload identity does not match stored columns".to_owned(),
        );
    }
    event.payload = canonical_payload;
    Ok(event)
}

fn ensure_replay_hash(
    table: &str,
    payload: &serde_json::Value,
    stored_hash: &str,
) -> Result<(), String> {
    if json_hash(payload)? == stored_hash {
        Ok(())
    } else {
        Err(format!("{table} stored hash does not match payload"))
    }
}

fn decode_typed_replay_payload<T>(
    table: &str,
    payload: &serde_json::Value,
    payload_text: Option<&str>,
    stored_hash: &str,
) -> Result<T, String>
where
    T: serde::de::DeserializeOwned + serde::Serialize,
{
    if json_hash(payload)? == stored_hash {
        return serde_json::from_value::<T>(payload.clone())
            .map_err(|error| format!("{table} payload decode failed: {error}"));
    }
    if let Some(payload_text) = payload_text {
        let normalized = parse_json_with_exact_floats(payload_text)
            .map_err(|error| format!("{table} payload normalization failed: {error}"))?;
        let decoded = serde_json::from_value::<T>(normalized.clone())
            .map_err(|error| format!("{table} payload decode failed: {error}"))?;
        let canonical = serde_json::to_value(&decoded)
            .map_err(|error| format!("{table} payload canonical encode failed: {error}"))?;
        if !json_semantically_equal(&normalized, &canonical) {
            return Err(format!("{table} stored hash does not match payload"));
        }
        ensure_replay_hash(table, &canonical, stored_hash)?;
        return Ok(decoded);
    }
    let decoded = serde_json::from_value::<T>(payload.clone())
        .map_err(|error| format!("{table} payload decode failed: {error}"))?;
    let canonical = serde_json::to_value(&decoded)
        .map_err(|error| format!("{table} payload canonical encode failed: {error}"))?;
    if !json_semantically_equal(payload, &canonical) {
        return Err(format!("{table} stored hash does not match payload"));
    }
    ensure_replay_hash(table, &canonical, stored_hash)?;
    Ok(decoded)
}

fn json_semantically_equal(left: &serde_json::Value, right: &serde_json::Value) -> bool {
    match (left, right) {
        (serde_json::Value::Number(left), serde_json::Value::Number(right)) => {
            left == right
                || matches!(
                    (left.as_f64(), right.as_f64()),
                    (Some(left), Some(right)) if left == right
                )
        }
        (serde_json::Value::Array(left), serde_json::Value::Array(right)) => {
            left.len() == right.len()
                && left
                    .iter()
                    .zip(right)
                    .all(|(left, right)| json_semantically_equal(left, right))
        }
        (serde_json::Value::Object(left), serde_json::Value::Object(right)) => {
            left.len() == right.len()
                && left.iter().all(|(key, left)| {
                    right
                        .get(key)
                        .is_some_and(|right| json_semantically_equal(left, right))
                })
        }
        _ => left == right,
    }
}

fn ensure_replay_schema(table: &str, stored_version: i32) -> Result<(), String> {
    if stored_version == SQL_LEDGER_SCHEMA_VERSION as i32 {
        Ok(())
    } else {
        Err(format!(
            "{table} schema version {stored_version} does not match {SQL_LEDGER_SCHEMA_VERSION}"
        ))
    }
}

async fn write_order_snapshot(
    client: &tokio_postgres::Client,
    record: &OrderRecord,
    stats: &SqlLedgerWriteStats,
) {
    let row = match sql_order_snapshot_row(record) {
        Ok(row) => row,
        Err(error) => {
            stats.record_snapshot_error(&error);
            return;
        }
    };
    let result = client
        .execute(
            "INSERT INTO order_snapshots \
             (internal_order_id, public_client_order_id, venue_client_order_id, exchange_order_id, \
              state, last_update_source, exchange, symbol, side, record, updated_at_ms, \
              record_hash, schema_version, captured_at_ms) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14) \
             ON CONFLICT (internal_order_id) DO UPDATE SET \
             public_client_order_id = EXCLUDED.public_client_order_id, \
             venue_client_order_id = EXCLUDED.venue_client_order_id, \
             exchange_order_id = EXCLUDED.exchange_order_id, \
             state = EXCLUDED.state, \
             last_update_source = EXCLUDED.last_update_source, \
             exchange = EXCLUDED.exchange, \
             symbol = EXCLUDED.symbol, \
             side = EXCLUDED.side, \
             record = EXCLUDED.record, \
             updated_at_ms = EXCLUDED.updated_at_ms, \
             record_hash = EXCLUDED.record_hash, \
             schema_version = EXCLUDED.schema_version, \
             captured_at_ms = EXCLUDED.captured_at_ms \
             WHERE order_snapshots.updated_at_ms <= EXCLUDED.updated_at_ms",
            &[
                &row.internal_order_id,
                &row.public_client_order_id,
                &row.venue_client_order_id,
                &row.exchange_order_id,
                &row.state,
                &row.last_update_source,
                &row.exchange,
                &row.symbol,
                &row.side,
                &row.record,
                &row.updated_at_ms,
                &row.record_hash,
                &row.schema_version,
                &row.captured_at_ms,
            ],
        )
        .await;
    match result {
        Ok(_) => stats.record_snapshot_success(),
        Err(error) => {
            stats.record_snapshot_error(&format!("order_snapshots upsert failed: {error}"))
        }
    }
}

async fn write_balance_event(
    client: &tokio_postgres::Client,
    event: &SqlBalanceLedgerEvent,
    stats: &SqlLedgerWriteStats,
) {
    let result = client
        .execute(
            "INSERT INTO balance_events \
             (event_id, exchange, asset, balance_kind, payload, payload_hash, observed_at_ms, \
              captured_at_ms) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
             ON CONFLICT (event_id) DO NOTHING",
            &[
                &event.event_id,
                &event.exchange,
                &event.asset,
                &event.balance_kind,
                &event.payload,
                &event.payload_hash,
                &event.observed_at_ms,
                &event.captured_at_ms,
            ],
        )
        .await;
    match result {
        Ok(_) => stats.record_balance_success(),
        Err(error) => stats.record_balance_error(&format!("balance_events insert failed: {error}")),
    }
}

async fn write_event_transaction(
    client: &mut tokio_postgres::Client,
    event: &ExecutionLedgerEvent,
    row: &SqlEventRow,
) -> Result<SqlLedgerPersistAck, EventWriteError> {
    let transaction = client.transaction().await?;
    let ack = write_event_in_transaction(&transaction, event, row).await?;
    transaction.commit().await?;
    Ok(ack)
}

async fn write_event_group_transaction(
    client: &mut tokio_postgres::Client,
    events: &[ExecutionLedgerEvent],
) -> Result<Vec<SqlLedgerPersistAck>, EventWriteError> {
    let transaction = client.transaction().await?;
    let mut acks = Vec::with_capacity(events.len());
    for event in events {
        let row = sql_event_row(event).map_err(EventWriteError::Encoding)?;
        acks.push(write_event_in_transaction(&transaction, event, &row).await?);
    }
    transaction.commit().await?;
    Ok(acks)
}

async fn write_event_in_transaction(
    transaction: &tokio_postgres::Transaction<'_>,
    event: &ExecutionLedgerEvent,
    row: &SqlEventRow,
) -> Result<SqlLedgerPersistAck, EventWriteError> {
    let inserted = insert_order_event(transaction, row).await?;
    let ack = if inserted == 0 {
        let existing = read_existing_order_event(transaction, &row.event_id).await?;
        writer::classify_event_write(false, &existing, row)?
    } else {
        writer::classify_event_write(true, row, row)?
    };
    write_event_fact_rows(transaction, event, row).await?;
    insert_projection_jobs(transaction, event, row).await?;
    Ok(ack)
}

async fn insert_projection_jobs<C>(
    client: &C,
    event: &ExecutionLedgerEvent,
    row: &SqlEventRow,
) -> Result<(), tokio_postgres::Error>
where
    C: GenericClient + Sync,
{
    let queued_at_ms = common::time::now_ms();
    for &projector in projection_jobs_for_event(event) {
        client
            .execute(
                INSERT_PROJECTION_JOB_SQL,
                &[&row.event_id, &projector, &row.payload_hash, &queued_at_ms],
            )
            .await?;
    }
    Ok(())
}

fn projection_jobs_for_event(event: &ExecutionLedgerEvent) -> &'static [&'static str] {
    let run_linked = event.order.run_id.is_some();
    match &event.payload {
        ExecutionLedgerPayload::FillSnapshot(fill) if run_linked && fill.fee.is_some() => {
            RUN_AND_COST_PROJECTORS
        }
        ExecutionLedgerPayload::FillSnapshot(_) | ExecutionLedgerPayload::OrderState { .. } => {
            RUN_PROJECTORS
        }
        ExecutionLedgerPayload::FeeSnapshot(_) if run_linked => CLOSE_RUN_AND_COST_PROJECTORS,
        ExecutionLedgerPayload::FeeSnapshot(_) => CLOSE_RUN_PROJECTORS,
        ExecutionLedgerPayload::FundingPayment(_) | ExecutionLedgerPayload::Slippage(_)
            if run_linked =>
        {
            RUN_AND_COST_PROJECTORS
        }
        ExecutionLedgerPayload::FundingPayment(_) | ExecutionLedgerPayload::Slippage(_) => {
            RUN_PROJECTORS
        }
        ExecutionLedgerPayload::OrderbookEvidence(_) => NO_PROJECTORS,
    }
}

async fn insert_order_event<C>(client: &C, row: &SqlEventRow) -> Result<u64, tokio_postgres::Error>
where
    C: GenericClient + Sync,
{
    client
        .execute(
            "INSERT INTO order_events \
             (event_id, internal_order_id, client_order_id, exchange_order_id, \
              public_client_order_id, venue_client_order_id, run_id, ticket_id, leg_role, \
              exchange, symbol, side, order_ref, event_type, source, state, event, payload, \
              payload_hash, schema_version, occurred_at_ms, captured_at_ms) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, \
                     $14, $15, $16, $17, $18, $19, $20, $21, $22) \
             ON CONFLICT (event_id) DO NOTHING",
            &[
                &row.event_id,
                &row.internal_order_id,
                &row.client_order_id,
                &row.exchange_order_id,
                &row.public_client_order_id,
                &row.venue_client_order_id,
                &row.run_id,
                &row.ticket_id,
                &row.leg_role,
                &row.exchange,
                &row.symbol,
                &row.side,
                &row.order_ref,
                &row.event_type,
                &row.source,
                &row.state,
                &row.lifecycle_event,
                &row.payload,
                &row.payload_hash,
                &row.schema_version,
                &row.occurred_at_ms,
                &row.captured_at_ms,
            ],
        )
        .await
}

async fn read_existing_order_event<C>(
    client: &C,
    event_id: &str,
) -> Result<SqlEventRow, EventWriteError>
where
    C: GenericClient + Sync,
{
    let row = client
        .query_one(
            "SELECT event_id, internal_order_id, client_order_id, exchange_order_id, \
             public_client_order_id, venue_client_order_id, run_id, ticket_id, leg_role, \
             exchange, symbol, side, order_ref, event_type, source, state, event, payload, \
             payload::text AS payload_text, payload_hash, schema_version, occurred_at_ms, \
             captured_at_ms \
             FROM order_events WHERE event_id = $1",
            &[&event_id],
        )
        .await?;
    sql_event_replay_row(&row).map_err(EventWriteError::Encoding)
}

async fn write_event_fact_rows(
    client: &(impl GenericClient + Sync),
    event: &ExecutionLedgerEvent,
    event_row: &SqlEventRow,
) -> Result<(), EventWriteError> {
    match &event.payload {
        ExecutionLedgerPayload::FillSnapshot(fill) => {
            let fill_row =
                sql_fill_row(event, event_row, fill).map_err(EventWriteError::Encoding)?;
            insert_fill(client, &fill_row).await?;
            if let Some(fee) = &fill.fee {
                let fee_row = sql_fee_row(event_row, fee, "embedded_fill")
                    .map_err(EventWriteError::Encoding)?;
                insert_fee(client, &fee_row).await?;
            }
        }
        ExecutionLedgerPayload::FeeSnapshot(fee) => {
            let fee_row =
                sql_fee_row(event_row, fee, "fee_snapshot").map_err(EventWriteError::Encoding)?;
            insert_fee(client, &fee_row).await?;
        }
        ExecutionLedgerPayload::FundingPayment(payment) => {
            let payment_row =
                sql_funding_payment_row(event_row, payment).map_err(EventWriteError::Encoding)?;
            insert_funding_payment(client, &payment_row).await?;
        }
        ExecutionLedgerPayload::Slippage(slippage) => {
            let slippage_row =
                sql_slippage_row(event_row, slippage).map_err(EventWriteError::Encoding)?;
            insert_slippage(client, &slippage_row).await?;
        }
        ExecutionLedgerPayload::OrderbookEvidence(record) => {
            let evidence_row =
                sql_orderbook_evidence_row(event_row, record).map_err(EventWriteError::Encoding)?;
            insert_orderbook_evidence(client, &evidence_row).await?;
        }
        ExecutionLedgerPayload::OrderState { .. } => {}
    }
    Ok(())
}

async fn insert_fill<C>(client: &C, row: &SqlFillRow) -> Result<(), tokio_postgres::Error>
where
    C: GenericClient + Sync,
{
    client
        .execute(
            "INSERT INTO fills \
             (event_id, internal_order_id, exchange, symbol, side, run_id, ticket_id, leg_role, \
              source, fill_kind, quantity, average_price, quote_value, quality, fill_confidence, \
              fill_confidence_score, fee_amount, fee_currency, fee_quality, payload_hash, \
              occurred_at_ms, captured_at_ms) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, \
                     $14, $15, $16, $17, $18, $19, $20, $21, $22) \
             ON CONFLICT (event_id) DO NOTHING",
            &[
                &row.event_id,
                &row.internal_order_id,
                &row.exchange,
                &row.symbol,
                &row.side,
                &row.run_id,
                &row.ticket_id,
                &row.leg_role,
                &row.source,
                &row.fill_kind,
                &row.quantity,
                &row.average_price,
                &row.quote_value,
                &row.quality,
                &row.fill_confidence,
                &row.fill_confidence_score,
                &row.fee_amount,
                &row.fee_currency,
                &row.fee_quality,
                &row.payload_hash,
                &row.occurred_at_ms,
                &row.captured_at_ms,
            ],
        )
        .await
        .map(|_| ())
}

async fn insert_fee<C>(client: &C, row: &SqlFeeRow) -> Result<(), tokio_postgres::Error>
where
    C: GenericClient + Sync,
{
    client
        .execute(
            "INSERT INTO fees \
             (event_id, internal_order_id, exchange, symbol, side, run_id, ticket_id, leg_role, \
              source, fee_origin, amount, currency, quality, payload_hash, occurred_at_ms, \
              captured_at_ms) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, \
                     $14, $15, $16) \
             ON CONFLICT (event_id) DO NOTHING",
            &[
                &row.event_id,
                &row.internal_order_id,
                &row.exchange,
                &row.symbol,
                &row.side,
                &row.run_id,
                &row.ticket_id,
                &row.leg_role,
                &row.source,
                &row.fee_origin,
                &row.amount,
                &row.currency,
                &row.quality,
                &row.payload_hash,
                &row.occurred_at_ms,
                &row.captured_at_ms,
            ],
        )
        .await
        .map(|_| ())
}

async fn insert_funding_payment(
    client: &(impl GenericClient + Sync),
    row: &SqlFundingPaymentRow,
) -> Result<(), tokio_postgres::Error> {
    client
        .execute(
            "INSERT INTO funding_payments \
             (event_id, internal_order_id, exchange, symbol, side, run_id, ticket_id, leg_role, \
              source, amount, currency, funding_time_ms, quality, payload_hash, occurred_at_ms, \
              captured_at_ms) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, \
                     $14, $15, $16) \
             ON CONFLICT (event_id) DO NOTHING",
            &[
                &row.event_id,
                &row.internal_order_id,
                &row.exchange,
                &row.symbol,
                &row.side,
                &row.run_id,
                &row.ticket_id,
                &row.leg_role,
                &row.source,
                &row.amount,
                &row.currency,
                &row.funding_time_ms,
                &row.quality,
                &row.payload_hash,
                &row.occurred_at_ms,
                &row.captured_at_ms,
            ],
        )
        .await
        .map(|_| ())
}

async fn insert_slippage<C>(client: &C, row: &SqlSlippageRow) -> Result<(), tokio_postgres::Error>
where
    C: GenericClient + Sync,
{
    client
        .execute(
            "INSERT INTO slippage_events \
             (event_id, internal_order_id, exchange, symbol, side, run_id, ticket_id, leg_role, \
              source, amount_usd, reference_price, fill_price, quantity, quality, payload_hash, \
              occurred_at_ms, captured_at_ms) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, \
                     $14, $15, $16, $17) \
             ON CONFLICT (event_id) DO NOTHING",
            &[
                &row.event_id,
                &row.internal_order_id,
                &row.exchange,
                &row.symbol,
                &row.side,
                &row.run_id,
                &row.ticket_id,
                &row.leg_role,
                &row.source,
                &row.amount_usd,
                &row.reference_price,
                &row.fill_price,
                &row.quantity,
                &row.quality,
                &row.payload_hash,
                &row.occurred_at_ms,
                &row.captured_at_ms,
            ],
        )
        .await
        .map(|_| ())
}

async fn insert_orderbook_evidence<C>(
    client: &C,
    row: &SqlOrderbookEvidenceRow,
) -> Result<(), tokio_postgres::Error>
where
    C: GenericClient + Sync,
{
    client
        .execute(
            "INSERT INTO orderbook_evidence \
             (event_id, internal_order_id, exchange, symbol, side, run_id, ticket_id, leg_role, \
              source, reference_price, bid, ask, mid, open_vwap_price, open_slippage_bps, \
              close_vwap_price, close_slippage_bps, depth_usd_5bps, depth_usd_10bps, \
              depth_usd_20bps, max_notional_usd, market_timestamp_ms, health, reason, \
              evidence_quality, payload_hash, occurred_at_ms, captured_at_ms) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, \
                     $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, \
                     $25, $26, $27, $28) \
             ON CONFLICT (event_id) DO NOTHING",
            &[
                &row.event_id,
                &row.internal_order_id,
                &row.exchange,
                &row.symbol,
                &row.side,
                &row.run_id,
                &row.ticket_id,
                &row.leg_role,
                &row.source,
                &row.reference_price,
                &row.bid,
                &row.ask,
                &row.mid,
                &row.open_vwap_price,
                &row.open_slippage_bps,
                &row.close_vwap_price,
                &row.close_slippage_bps,
                &row.depth_usd_5bps,
                &row.depth_usd_10bps,
                &row.depth_usd_20bps,
                &row.max_notional_usd,
                &row.market_timestamp_ms,
                &row.health,
                &row.reason,
                &row.evidence_quality,
                &row.payload_hash,
                &row.occurred_at_ms,
                &row.captured_at_ms,
            ],
        )
        .await
        .map(|_| ())
}

fn sql_event_row(event: &ExecutionLedgerEvent) -> Result<SqlEventRow, String> {
    let payload =
        serde_json::to_value(event).map_err(|error| format!("event encode failed: {error}"))?;
    let order_ref = serde_json::to_value(&event.order)
        .map_err(|error| format!("order ref encode failed: {error}"))?;
    let side = serde_label(&event.order.side)?;
    let leg_role = event.order.leg_role.as_ref().map(serde_label).transpose()?;
    Ok(SqlEventRow {
        event_id: event.event_id.clone(),
        internal_order_id: event.order.identity.internal_order_id.clone(),
        client_order_id: event.order.identity.public_client_order_id.clone(),
        exchange_order_id: event.order.identity.exchange_order_id.clone(),
        public_client_order_id: event.order.identity.public_client_order_id.clone(),
        venue_client_order_id: event.order.identity.venue_client_order_id.clone(),
        run_id: event.order.run_id.clone(),
        ticket_id: event.order.ticket_id.clone(),
        leg_role,
        exchange: event.order.exchange.clone(),
        symbol: event.order.symbol.clone(),
        side,
        order_ref,
        event_type: serde_label(&event.event_type)?,
        source: serde_label(&event.source)?,
        state: event_state_label(event)?,
        lifecycle_event: None,
        payload_text: None,
        payload_hash: json_hash(&payload)?,
        schema_version: SQL_LEDGER_SCHEMA_VERSION as i32,
        payload,
        occurred_at_ms: event.occurred_at_ms,
        captured_at_ms: event.captured_at_ms,
    })
}

fn sql_fill_row(
    event: &ExecutionLedgerEvent,
    event_row: &SqlEventRow,
    fill: &FillLedgerSnapshot,
) -> Result<SqlFillRow, String> {
    Ok(SqlFillRow {
        event_id: event_row.event_id.clone(),
        internal_order_id: event_row.internal_order_id.clone(),
        exchange: event_row.exchange.clone(),
        symbol: event_row.symbol.clone(),
        side: event_row.side.clone(),
        run_id: event_row.run_id.clone(),
        ticket_id: event_row.ticket_id.clone(),
        leg_role: event_row.leg_role.clone(),
        source: event_row.source.clone(),
        fill_kind: serde_label(&event.event_type)?,
        quantity: fill.quantity,
        average_price: fill.average_price,
        quote_value: fill.quote_value,
        quality: serde_label(&fill.quality)?,
        fill_confidence: serde_label(&fill.confidence)?,
        fill_confidence_score: fill.confidence.score(),
        fee_amount: fill.fee.as_ref().map(|fee| fee.amount),
        fee_currency: fill.fee.as_ref().and_then(|fee| fee.currency.clone()),
        fee_quality: fill
            .fee
            .as_ref()
            .map(|fee| serde_label(&fee.quality))
            .transpose()?,
        payload_hash: event_row.payload_hash.clone(),
        occurred_at_ms: event_row.occurred_at_ms,
        captured_at_ms: event_row.captured_at_ms,
    })
}

fn sql_fee_row(
    event_row: &SqlEventRow,
    fee: &FeeLedgerSnapshot,
    fee_origin: &'static str,
) -> Result<SqlFeeRow, String> {
    Ok(SqlFeeRow {
        event_id: event_row.event_id.clone(),
        internal_order_id: event_row.internal_order_id.clone(),
        exchange: event_row.exchange.clone(),
        symbol: event_row.symbol.clone(),
        side: event_row.side.clone(),
        run_id: event_row.run_id.clone(),
        ticket_id: event_row.ticket_id.clone(),
        leg_role: event_row.leg_role.clone(),
        source: event_row.source.clone(),
        fee_origin,
        amount: fee.amount,
        currency: fee.currency.clone(),
        quality: serde_label(&fee.quality)?,
        payload_hash: event_row.payload_hash.clone(),
        occurred_at_ms: event_row.occurred_at_ms,
        captured_at_ms: event_row.captured_at_ms,
    })
}

fn sql_funding_payment_row(
    event_row: &SqlEventRow,
    payment: &FundingPaymentLedgerRecord,
) -> Result<SqlFundingPaymentRow, String> {
    Ok(SqlFundingPaymentRow {
        event_id: event_row.event_id.clone(),
        internal_order_id: event_row.internal_order_id.clone(),
        exchange: event_row.exchange.clone(),
        symbol: event_row.symbol.clone(),
        side: event_row.side.clone(),
        run_id: event_row.run_id.clone(),
        ticket_id: event_row.ticket_id.clone(),
        leg_role: event_row.leg_role.clone(),
        source: event_row.source.clone(),
        amount: payment.amount,
        currency: payment.currency.clone(),
        funding_time_ms: payment.funding_time_ms,
        quality: serde_label(&payment.quality)?,
        payload_hash: event_row.payload_hash.clone(),
        occurred_at_ms: event_row.occurred_at_ms,
        captured_at_ms: event_row.captured_at_ms,
    })
}

fn sql_slippage_row(
    event_row: &SqlEventRow,
    record: &SlippageLedgerRecord,
) -> Result<SqlSlippageRow, String> {
    Ok(SqlSlippageRow {
        event_id: event_row.event_id.clone(),
        internal_order_id: event_row.internal_order_id.clone(),
        exchange: event_row.exchange.clone(),
        symbol: event_row.symbol.clone(),
        side: event_row.side.clone(),
        run_id: event_row.run_id.clone(),
        ticket_id: event_row.ticket_id.clone(),
        leg_role: event_row.leg_role.clone(),
        source: event_row.source.clone(),
        amount_usd: record.amount_usd,
        reference_price: record.reference_price,
        fill_price: record.fill_price,
        quantity: record.quantity,
        quality: serde_label(&record.quality)?,
        payload_hash: event_row.payload_hash.clone(),
        occurred_at_ms: event_row.occurred_at_ms,
        captured_at_ms: event_row.captured_at_ms,
    })
}

fn sql_orderbook_evidence_row(
    event_row: &SqlEventRow,
    record: &OrderbookDepthLedgerRecord,
) -> Result<SqlOrderbookEvidenceRow, String> {
    let health = record
        .health
        .as_ref()
        .map(serde_json::to_value)
        .transpose()
        .map_err(|error| format!("orderbook health encode failed: {error}"))?;
    Ok(SqlOrderbookEvidenceRow {
        event_id: event_row.event_id.clone(),
        internal_order_id: event_row.internal_order_id.clone(),
        exchange: event_row.exchange.clone(),
        symbol: event_row.symbol.clone(),
        side: event_row.side.clone(),
        run_id: event_row.run_id.clone(),
        ticket_id: event_row.ticket_id.clone(),
        leg_role: event_row.leg_role.clone(),
        source: event_row.source.clone(),
        reference_price: record.reference_price,
        bid: record.bid,
        ask: record.ask,
        mid: record.mid,
        open_vwap_price: record.open_vwap_price,
        open_slippage_bps: record.open_slippage_bps,
        close_vwap_price: record.close_vwap_price,
        close_slippage_bps: record.close_slippage_bps,
        depth_usd_5bps: record.depth_usd_5bps,
        depth_usd_10bps: record.depth_usd_10bps,
        depth_usd_20bps: record.depth_usd_20bps,
        max_notional_usd: record.max_notional_usd,
        market_timestamp_ms: record.market_timestamp_ms,
        health,
        reason: record.reason.clone(),
        evidence_quality: serde_label(&record.quality)?,
        payload_hash: event_row.payload_hash.clone(),
        occurred_at_ms: event_row.occurred_at_ms,
        captured_at_ms: event_row.captured_at_ms,
    })
}

fn sql_order_snapshot_row(record: &OrderRecord) -> Result<SqlOrderSnapshotRow, String> {
    let record_value =
        serde_json::to_value(record).map_err(|error| format!("record encode failed: {error}"))?;
    let identity = record.identity_snapshot();
    let exchange_order_id = snapshot_exchange_order_id(record, &identity);
    Ok(SqlOrderSnapshotRow {
        internal_order_id: record.intent.id.clone(),
        public_client_order_id: identity.public_client_order_id,
        exchange_order_id,
        venue_client_order_id: identity.venue_client_order_id,
        state: serde_label(&record.state)?,
        last_update_source: serde_label(&record.last_update_source)?,
        exchange: record.intent.exchange.clone(),
        symbol: record.intent.symbol.clone(),
        side: serde_label(&record.intent.side)?,
        record_text: None,
        updated_at_ms: record.updated_at_ms,
        record_hash: json_hash(&record_value)?,
        record: record_value,
        schema_version: SQL_LEDGER_SCHEMA_VERSION as i32,
        captured_at_ms: common::time::now_ms(),
    })
}

impl SqlBalanceLedgerEvent {
    pub fn from_balance_row(
        balance_kind: &str,
        row: &VenueBalanceInfo,
        observed_at_ms: i64,
    ) -> Result<Self, String> {
        let payload =
            serde_json::to_value(row).map_err(|error| format!("balance encode failed: {error}"))?;
        let payload_hash = json_hash(&payload)?;
        let event_id = balance_event_id(row, balance_kind, observed_at_ms, &payload_hash);
        Ok(Self {
            event_id,
            exchange: row.venue.clone(),
            asset: row.currency.clone(),
            balance_kind: balance_kind.to_owned(),
            payload,
            payload_text: None,
            payload_hash,
            observed_at_ms,
            captured_at_ms: common::time::now_ms(),
        })
    }
}

impl SqlRunFinalityLedgerEvent {
    pub fn event_id(&self) -> &str {
        &self.event_id
    }

    pub fn from_execution_run(
        run: &ExecutionRun,
        source: OrderUpdateSource,
        source_event_id: Option<&str>,
        source_order_event_id: Option<&str>,
        occurred_at_ms: i64,
    ) -> Result<Self, String> {
        run_finality_event(&RunFinalityEventInput {
            run_kind: "execution_run",
            run_id: &run.run_id,
            state: &run.state,
            payload_source: run,
            source,
            source_event_id,
            source_order_event_id,
            occurred_at_ms,
        })
    }

    pub fn from_close_run(
        run: &CloseRun,
        source: OrderUpdateSource,
        source_event_id: Option<&str>,
        source_order_event_id: Option<&str>,
        occurred_at_ms: i64,
    ) -> Result<Self, String> {
        run_finality_event(&RunFinalityEventInput {
            run_kind: "close_run",
            run_id: &run.id,
            state: &run.status,
            payload_source: run,
            source,
            source_event_id,
            source_order_event_id,
            occurred_at_ms,
        })
    }
}

struct RunFinalityEventInput<'a, TState, TPayload> {
    run_kind: &'a str,
    run_id: &'a str,
    state: &'a TState,
    payload_source: &'a TPayload,
    source: OrderUpdateSource,
    source_event_id: Option<&'a str>,
    source_order_event_id: Option<&'a str>,
    occurred_at_ms: i64,
}

fn run_finality_event<TState, TPayload>(
    input: &RunFinalityEventInput<'_, TState, TPayload>,
) -> Result<SqlRunFinalityLedgerEvent, String>
where
    TState: serde::Serialize,
    TPayload: serde::Serialize,
{
    let payload = serde_json::to_value(input.payload_source)
        .map_err(|error| format!("run finality payload encode failed: {error}"))?;
    let payload_hash = json_hash(&payload)?;
    let source = serde_label(&input.source)?;
    let state = serde_label(input.state)?;
    let captured_at_ms = common::time::now_ms();
    let occurred_at_ms = positive_time_or_capture(input.occurred_at_ms, captured_at_ms);
    let event_id = run_finality_event_id(&RunFinalityEventIdInput {
        run_kind: input.run_kind,
        run_id: input.run_id,
        state: &state,
        source: &source,
        source_event_id: input.source_event_id,
        source_order_event_id: input.source_order_event_id,
        occurred_at_ms,
        payload_hash: &payload_hash,
    });
    Ok(SqlRunFinalityLedgerEvent {
        event_id,
        run_kind: input.run_kind.to_owned(),
        run_id: input.run_id.to_owned(),
        source_event_id: input.source_event_id.map(str::to_owned),
        source_order_event_id: input.source_order_event_id.map(str::to_owned),
        source,
        state,
        payload,
        payload_hash,
        occurred_at_ms,
        captured_at_ms,
    })
}

struct RunFinalityEventIdInput<'a> {
    run_kind: &'a str,
    run_id: &'a str,
    state: &'a str,
    source: &'a str,
    source_event_id: Option<&'a str>,
    source_order_event_id: Option<&'a str>,
    occurred_at_ms: i64,
    payload_hash: &'a str,
}

fn run_finality_event_id(input: &RunFinalityEventIdInput<'_>) -> String {
    let legacy_id = legacy_run_finality_event_id(input);
    let source_hash = run_finality_source_hash(input.source_event_id, input.source_order_event_id);
    format!("{legacy_id}:sources:{source_hash:016x}")
}

fn legacy_run_finality_event_id(input: &RunFinalityEventIdInput<'_>) -> String {
    let hash = input
        .payload_hash
        .strip_prefix("fnv1a64:")
        .unwrap_or(input.payload_hash);
    format!(
        "run_finality:{}:{}:{}:{}:{}:{hash}",
        input.run_kind, input.run_id, input.state, input.source, input.occurred_at_ms
    )
}

fn run_finality_source_hash(
    source_event_id: Option<&str>,
    source_order_event_id: Option<&str>,
) -> u64 {
    let mut hash = FNV64_OFFSET_BASIS;
    for source_id in [source_event_id, source_order_event_id] {
        hash ^= u64::from(source_id.is_some());
        hash = hash.wrapping_mul(FNV64_PRIME);
        let bytes = source_id.unwrap_or_default().as_bytes();
        for byte in bytes.len().to_le_bytes().iter().chain(bytes) {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(FNV64_PRIME);
        }
    }
    hash
}

fn positive_time_or_capture(occurred_at_ms: i64, captured_at_ms: i64) -> i64 {
    if occurred_at_ms > 0 {
        occurred_at_ms
    } else {
        captured_at_ms
    }
}

fn balance_event_id(
    row: &VenueBalanceInfo,
    balance_kind: &str,
    observed_at_ms: i64,
    payload_hash: &str,
) -> String {
    let hash = payload_hash
        .strip_prefix("fnv1a64:")
        .unwrap_or(payload_hash);
    format!(
        "balance:{}:{}:{}:{}:{}",
        row.venue, row.currency, balance_kind, observed_at_ms, hash
    )
}

fn snapshot_exchange_order_id(
    record: &OrderRecord,
    identity: &VenueOrderIdentity,
) -> Option<String> {
    record
        .exchange_order_id
        .clone()
        .or_else(|| identity.exchange_order_id.clone())
}

fn event_state_label(event: &ExecutionLedgerEvent) -> Result<Option<String>, String> {
    match &event.payload {
        ExecutionLedgerPayload::OrderState { state, .. } => serde_label(state).map(Some),
        _ => Ok(None),
    }
}

fn serde_label<T>(value: &T) -> Result<String, String>
where
    T: serde::Serialize,
{
    serde_json::to_value(value)
        .map_err(|error| format!("serde label encode failed: {error}"))?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| "serde label was not a string".to_owned())
}

fn json_hash(value: &serde_json::Value) -> Result<String, String> {
    let bytes =
        serde_json::to_vec(value).map_err(|error| format!("json hash encode failed: {error}"))?;
    Ok(format!("fnv1a64:{:016x}", fnv1a64_runtime(&bytes)))
}

fn fnv1a64_runtime(bytes: &[u8]) -> u64 {
    let mut hash = FNV64_OFFSET_BASIS;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV64_PRIME);
    }
    hash
}

fn non_zero_ms(value: i64) -> Option<i64> {
    (value > 0).then_some(value)
}

impl SqlLedgerMigrationHealth {
    pub fn unconfigured(observed_at_ms: i64) -> Self {
        Self {
            configured: false,
            migration_id: SQL_LEDGER_MIGRATION_ID,
            migration_path: SQL_LEDGER_MIGRATION_PATH,
            migration_checksum: sql_ledger_schema_hash(),
            schema_version: None,
            applied: false,
            degraded_reason: Some(StorageDegradedReason::Disabled),
            last_success_at_ms: None,
            last_error_at_ms: None,
            last_error: None,
            observed_at_ms,
        }
    }

    fn applied(observed_at_ms: i64) -> Self {
        Self {
            configured: true,
            migration_id: SQL_LEDGER_MIGRATION_ID,
            migration_path: SQL_LEDGER_MIGRATION_PATH,
            migration_checksum: sql_ledger_schema_hash(),
            schema_version: Some(SQL_LEDGER_MIGRATION_VERSION),
            applied: true,
            degraded_reason: None,
            last_success_at_ms: Some(observed_at_ms),
            last_error_at_ms: None,
            last_error: None,
            observed_at_ms,
        }
    }

    fn failed(observed_at_ms: i64, error: migrations::MigrationFailure) -> Self {
        Self {
            configured: true,
            migration_id: SQL_LEDGER_MIGRATION_ID,
            migration_path: SQL_LEDGER_MIGRATION_PATH,
            migration_checksum: sql_ledger_schema_hash(),
            schema_version: None,
            applied: false,
            degraded_reason: Some(error.reason),
            last_success_at_ms: None,
            last_error_at_ms: Some(observed_at_ms),
            last_error: Some(error.message),
            observed_at_ms,
        }
    }

    pub fn migration_authority(&self) -> StorageMigrationAuthority {
        StorageMigrationAuthority {
            migration_id: self.migration_id.to_owned(),
            schema_name: SQL_LEDGER_SCHEMA_NAME.to_owned(),
            migration_path: self.migration_path.to_owned(),
            schema_version: self.schema_version,
            migration_checksum: Some(self.migration_checksum.clone()),
            applied: self.applied,
            applied_at_ms: self.last_success_at_ms,
        }
    }

    pub fn storage_contract(&self) -> StorageRuntimeContract {
        let degraded_reasons = self.degraded_reason.into_iter().collect();
        StorageRuntimeContract {
            backend_kind: if self.configured {
                StorageBackendKind::Postgres
            } else {
                StorageBackendKind::Disabled
            },
            degraded_reasons,
            migration_authority: Some(self.migration_authority()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod replay_review_tests;

    #[test]
    fn sql_ledger_schema_hash_is_stable_label() {
        assert!(sql_ledger_schema_hash().starts_with("fnv1a64:"));
        assert_eq!(
            sql_ledger_schema_hash().len(),
            "fnv1a64:0000000000000000".len()
        );
    }

    #[test]
    fn sql_ledger_schema_contains_normalized_fact_tables() {
        assert!(SQL_LEDGER_SCHEMA_SQL.contains("CREATE TABLE IF NOT EXISTS fills"));
        assert!(SQL_LEDGER_SCHEMA_SQL.contains("CREATE TABLE IF NOT EXISTS fees"));
        assert!(SQL_LEDGER_SCHEMA_SQL.contains("CREATE TABLE IF NOT EXISTS funding_payments"));
        assert!(SQL_LEDGER_SCHEMA_SQL.contains("CREATE TABLE IF NOT EXISTS slippage_events"));
        assert!(SQL_LEDGER_SCHEMA_SQL.contains("CREATE TABLE IF NOT EXISTS orderbook_evidence"));
        assert!(SQL_LEDGER_SCHEMA_SQL.contains("CREATE TABLE IF NOT EXISTS balance_events"));
        assert!(SQL_LEDGER_SCHEMA_SQL.contains("CREATE TABLE IF NOT EXISTS run_finality_events"));
        assert!(SQL_LEDGER_SCHEMA_SQL.contains("fill_confidence"));
        assert!(SQL_LEDGER_SCHEMA_SQL.contains("fill_confidence_score"));
    }

    #[test]
    fn sql_run_finality_replay_query_keeps_latest_events_in_apply_order() {
        assert!(SQL_RUN_FINALITY_REPLAY_QUERY.contains("FROM run_finality_events"));
        assert!(SQL_RUN_FINALITY_REPLAY_QUERY
            .contains("ORDER BY occurred_at_ms DESC, id DESC LIMIT $1"));
        assert!(SQL_RUN_FINALITY_REPLAY_QUERY.contains("ORDER BY occurred_at_ms ASC, id ASC"));
        assert!(SQL_RUN_FINALITY_REPLAY_QUERY.contains("latest_run_finality_events"));
    }

    #[test]
    fn sql_order_event_replay_query_keeps_latest_events_in_apply_order() {
        assert!(SQL_ORDER_EVENTS_REPLAY_QUERY.contains("FROM order_events"));
        assert!(SQL_ORDER_EVENTS_REPLAY_QUERY
            .contains("ORDER BY occurred_at_ms DESC, id DESC LIMIT $1"));
        assert!(SQL_ORDER_EVENTS_REPLAY_QUERY.contains("ORDER BY occurred_at_ms ASC, id ASC"));
        assert!(SQL_ORDER_EVENTS_REPLAY_QUERY.contains("latest_order_events"));
    }

    #[test]
    fn sql_replay_limit_uses_extra_row_as_observable_oldest_sentinel() {
        let returned_rows = SQL_LEDGER_REPLAY_LIMIT as usize + 1;

        assert!(replay_rows_limited(returned_rows));
        assert_eq!(
            replay_rows_len(returned_rows),
            SQL_LEDGER_REPLAY_LIMIT as usize
        );
        assert_eq!(newest_replay_rows_to_skip(returned_rows), 1);
        assert_eq!(newest_replay_rows_to_skip(returned_rows - 1), 0);
    }

    #[test]
    fn sql_realized_close_runs_query_keeps_latest_close_snapshots() {
        assert!(SQL_REALIZED_CLOSE_RUNS_QUERY.contains("FROM run_finality_events"));
        assert!(SQL_REALIZED_CLOSE_RUNS_QUERY.contains("run_kind = 'close_run'"));
        assert!(SQL_REALIZED_CLOSE_RUNS_QUERY
            .contains("ORDER BY occurred_at_ms DESC, id DESC LIMIT $2"));
        assert!(SQL_REALIZED_CLOSE_RUNS_QUERY.contains("ORDER BY occurred_at_ms ASC, id ASC"));
        assert!(SQL_REALIZED_CLOSE_RUNS_QUERY.contains("latest_realized_close_runs"));
    }

    #[tokio::test]
    async fn sql_ledger_migration_unconfigured_records_health() {
        let health = run_sql_ledger_migration(None).await;

        assert!(!health.configured);
        assert!(!health.applied);
        assert_eq!(health.migration_id, SQL_LEDGER_MIGRATION_ID);
        assert_eq!(health.schema_version, None);
        assert!(health.last_error.is_none());
        assert_eq!(
            health.degraded_reason,
            Some(StorageDegradedReason::Disabled)
        );
        assert_eq!(
            health.storage_contract().backend_kind,
            StorageBackendKind::Disabled
        );
    }

    #[tokio::test]
    async fn sql_ledger_migration_invalid_url_records_error() {
        let health = run_sql_ledger_migration(Some("not-a-postgres-url")).await;

        assert!(health.configured);
        assert!(!health.applied);
        assert!(health.last_error.is_some());
        assert!(health.last_error_at_ms.is_some());
        assert_eq!(
            health.degraded_reason,
            Some(StorageDegradedReason::Unavailable)
        );
    }

    #[test]
    fn sql_event_row_preserves_identity_and_hash() {
        let event = execution_event();
        let row = sql_event_row(&event).expect("event row");

        assert_eq!(row.event_id, "evt-1");
        assert_eq!(row.event_type, "order_state");
        assert_eq!(row.source, "private_ws");
        assert_eq!(row.state.as_deref(), Some("accepted"));
        assert_eq!(row.exchange, "mock");
        assert_eq!(row.symbol, "BTC");
        assert_eq!(row.side, "buy");
        assert!(row.payload_hash.starts_with("fnv1a64:"));
        assert_eq!(row.internal_order_id, "ord-1");
    }

    #[test]
    fn sql_order_snapshot_row_preserves_record_hash() {
        let record = order_record();
        let row = sql_order_snapshot_row(&record).expect("snapshot row");

        assert_eq!(row.internal_order_id, "ord-1");
        assert_eq!(row.public_client_order_id, "client-1");
        assert_eq!(row.exchange_order_id.as_deref(), Some("ex-1"));
        assert_eq!(row.state, "accepted");
        assert_eq!(row.updated_at_ms, 10);
        assert_eq!(row.schema_version, SQL_LEDGER_SCHEMA_VERSION as i32);
        assert!(row.record_hash.starts_with("fnv1a64:"));
    }

    #[test]
    fn sql_fill_row_preserves_actual_fill_and_embedded_fee() {
        let event = fill_event();
        let event_row = sql_event_row(&event).expect("event row");
        let ExecutionLedgerPayload::FillSnapshot(fill) = &event.payload else {
            panic!("fill payload");
        };

        let row = sql_fill_row(&event, &event_row, fill).expect("fill row");

        assert_eq!(row.event_id, "fill-1");
        assert_eq!(row.fill_kind, "fill_event");
        assert_eq!(row.quantity, 2.0);
        assert_eq!(row.average_price, 101.5);
        assert_eq!(row.fill_confidence, "venue_fill");
        assert_eq!(row.fill_confidence_score, 1.0);
        assert_eq!(row.fee_amount, Some(-0.12));
        assert_eq!(row.fee_currency.as_deref(), Some("USDC"));
        assert_eq!(row.fee_quality.as_deref(), Some("actual"));
    }

    #[test]
    fn projection_mapping_enqueues_only_run_linked_cost_events() {
        let fill = fill_event();
        let fee = fee_event();
        let funding = funding_event();
        let slippage = slippage_event_for_order("ord-1", 9, 0.44);
        let orderbook = orderbook_event();
        let order_state = execution_event();

        assert_eq!(projection_jobs_for_event(&fill), RUN_AND_COST_PROJECTORS);
        assert_eq!(
            projection_jobs_for_event(&fee),
            CLOSE_RUN_AND_COST_PROJECTORS
        );
        for event in [&funding, &slippage] {
            assert_eq!(projection_jobs_for_event(event), RUN_AND_COST_PROJECTORS);
        }
        assert!(projection_jobs_for_event(&orderbook).is_empty());
        assert_eq!(projection_jobs_for_event(&order_state), RUN_PROJECTORS);

        let mut unlinked = fee;
        unlinked.order.run_id = None;
        assert_eq!(projection_jobs_for_event(&unlinked), CLOSE_RUN_PROJECTORS);
    }

    #[test]
    fn event_transaction_persists_projection_jobs_before_commit() {
        assert!(INSERT_PROJECTION_JOB_SQL.contains("INSERT INTO ledger_projection_jobs"));
        assert!(INSERT_PROJECTION_JOB_SQL.contains("ON CONFLICT (event_id, projector) DO NOTHING"));
        let source = include_str!("sql_ledger.rs");
        let transaction_body = source
            .split_once("async fn write_event_transaction")
            .and_then(|(_, rest)| rest.split_once("async fn write_event_group_transaction"))
            .map(|(body, _)| body)
            .expect("event transaction body");
        let transaction_write = transaction_body
            .find("write_event_in_transaction(&transaction")
            .expect("transactional event write");
        let commit = transaction_body
            .find("transaction.commit().await?")
            .expect("transaction commit");
        assert!(transaction_write < commit);
        let event_write_body = source
            .split_once("async fn write_event_in_transaction")
            .and_then(|(_, rest)| rest.split_once("async fn insert_projection_jobs"))
            .map(|(body, _)| body)
            .expect("event write body");
        assert!(event_write_body.contains("insert_projection_jobs(transaction, event, row).await?"));
    }

    #[test]
    fn sql_fee_row_preserves_standalone_fee_snapshot() {
        let event = fee_event();
        let event_row = sql_event_row(&event).expect("event row");
        let ExecutionLedgerPayload::FeeSnapshot(fee) = &event.payload else {
            panic!("fee payload");
        };

        let row = sql_fee_row(&event_row, fee, "fee_snapshot").expect("fee row");

        assert_eq!(row.event_id, "fee-1");
        assert_eq!(row.fee_origin, "fee_snapshot");
        assert_eq!(row.amount, -0.33);
        assert_eq!(row.currency.as_deref(), Some("USDT"));
        assert_eq!(row.quality, "actual");
    }

    #[test]
    fn sql_funding_payment_row_preserves_funding_time() {
        let event = funding_event();
        let event_row = sql_event_row(&event).expect("event row");
        let ExecutionLedgerPayload::FundingPayment(payment) = &event.payload else {
            panic!("funding payload");
        };

        let row = sql_funding_payment_row(&event_row, payment).expect("funding payment row");

        assert_eq!(row.event_id, "funding-1");
        assert_eq!(row.amount, 1.25);
        assert_eq!(row.currency, "USDC");
        assert_eq!(row.funding_time_ms, 123_000);
        assert_eq!(row.quality, "actual");
    }

    #[test]
    fn sql_replay_keeps_cross_venue_funding_fact_links() {
        let mut long = funding_event_for_order("hedge-1-long", 20, -0.12);
        long.order.exchange = "hyperliquid".to_owned();
        long.order.symbol = "BTC-USDC".to_owned();
        let mut short = funding_event_for_order("hedge-1-short", 20, 0.08);
        short.order.exchange = "okx".to_owned();
        short.order.symbol = "BTC-USDT".to_owned();
        short.order.leg_role = Some(shared_types::HedgeLegRole::Short);

        let mut replay = SqlLedgerReplay::with_limit(10);
        for event in [&long, &short] {
            push_replay_event_row(&mut replay, sql_event_row(event));
        }

        assert_eq!(replay.health.replayed_events, 2);
        assert!(replay.events.iter().any(|event| {
            event.event_id == "funding-hedge-1-long"
                && event.source == OrderUpdateSource::PrivateWs
                && event.order.exchange == "hyperliquid"
                && event.order.run_id.as_deref() == Some("run-1")
                && event.order.ticket_id.as_deref() == Some("ticket-1")
                && event.order.leg_role == Some(shared_types::HedgeLegRole::Long)
                && event.order.identity.internal_order_id == "hedge-1-long"
        }));
        assert!(replay.events.iter().any(|event| {
            event.event_id == "funding-hedge-1-short"
                && event.source == OrderUpdateSource::PrivateWs
                && event.order.exchange == "okx"
                && event.order.run_id.as_deref() == Some("run-1")
                && event.order.ticket_id.as_deref() == Some("ticket-1")
                && event.order.leg_role == Some(shared_types::HedgeLegRole::Short)
                && event.order.identity.internal_order_id == "hedge-1-short"
        }));
    }

    #[test]
    fn sql_slippage_row_preserves_actual_cost_fact() {
        let event = slippage_event_for_order("ord-1", 9, 0.44);
        let event_row = sql_event_row(&event).expect("event row");
        let ExecutionLedgerPayload::Slippage(record) = &event.payload else {
            panic!("slippage payload");
        };

        let row = sql_slippage_row(&event_row, record).expect("slippage row");

        assert_eq!(row.event_id, "slippage-fill-ord-1");
        assert_eq!(row.amount_usd, 0.44);
        assert_eq!(row.reference_price, 101.0);
        assert_eq!(row.fill_price, 101.5);
        assert_eq!(row.quality, "actual");
    }

    #[test]
    fn sql_orderbook_evidence_row_preserves_depth_fact() {
        let event = orderbook_event();
        let event_row = sql_event_row(&event).expect("event row");
        let ExecutionLedgerPayload::OrderbookEvidence(record) = &event.payload else {
            panic!("orderbook payload");
        };

        let row = sql_orderbook_evidence_row(&event_row, record.as_ref()).expect("orderbook row");

        assert_eq!(row.event_id, "orderbook-1");
        assert_eq!(row.reference_price, Some(100.0));
        assert_eq!(row.depth_usd_20bps, Some(1500.0));
        assert_eq!(row.max_notional_usd, Some(1500.0));
        assert_eq!(row.market_timestamp_ms, Some(123));
        assert_eq!(row.evidence_quality, "actual");
        assert!(row.health.is_some());
    }

    #[test]
    fn sql_balance_event_preserves_balance_payload_identity() {
        let row = shared_types::VenueBalanceInfo {
            venue: "okx".to_owned(),
            currency: "USDT".to_owned(),
            total: 100.0,
            available: 80.0,
            frozen: 20.0,
            unrealized_pnl: 1.5,
        };

        let event =
            SqlBalanceLedgerEvent::from_balance_row("snapshot", &row, 123).expect("balance event");

        assert_eq!(event.exchange, "okx");
        assert_eq!(event.asset, "USDT");
        assert_eq!(event.balance_kind, "snapshot");
        assert_eq!(event.observed_at_ms, 123);
        assert!(event.event_id.starts_with("balance:okx:USDT:snapshot:123:"));
        assert!(event.payload_hash.starts_with("fnv1a64:"));
    }

    #[test]
    fn sql_execution_run_finality_event_preserves_state_and_source() {
        let run = execution_run();

        let event = SqlRunFinalityLedgerEvent::from_execution_run(
            &run,
            shared_types::OrderUpdateSource::PrivateWs,
            Some("fill-event-1"),
            Some("long-order"),
            123,
        )
        .expect("run finality event");

        assert_eq!(event.run_kind, "execution_run");
        assert_eq!(event.run_id, "run-1");
        assert_eq!(event.state, "hedged");
        assert_eq!(event.source, "private_ws");
        assert_eq!(event.source_event_id.as_deref(), Some("fill-event-1"));
        assert_eq!(event.source_order_event_id.as_deref(), Some("long-order"));
        assert_eq!(event.occurred_at_ms, 123);
        assert!(event
            .event_id
            .starts_with("run_finality:execution_run:run-1:hedged:private_ws:123:"));
        assert!(event.payload_hash.starts_with("fnv1a64:"));
        assert_eq!(
            event
                .payload
                .get("runId")
                .and_then(serde_json::Value::as_str),
            Some("run-1")
        );
    }

    #[test]
    fn sql_close_run_finality_event_preserves_status_and_payload() {
        let run = close_run();

        let event = SqlRunFinalityLedgerEvent::from_close_run(
            &run,
            shared_types::OrderUpdateSource::OrderQuery,
            None,
            Some("close-order"),
            456,
        )
        .expect("close run finality event");

        assert_eq!(event.run_kind, "close_run");
        assert_eq!(event.run_id, "close-1");
        assert_eq!(event.state, "succeeded");
        assert_eq!(event.source, "order_query");
        assert_eq!(event.source_event_id, None);
        assert_eq!(event.source_order_event_id.as_deref(), Some("close-order"));
        assert_eq!(event.occurred_at_ms, 456);
        assert!(event
            .event_id
            .starts_with("run_finality:close_run:close-1:succeeded:order_query:456:"));
        assert_eq!(
            event.payload.get("id").and_then(serde_json::Value::as_str),
            Some("close-1")
        );
    }

    #[test]
    fn sql_replay_decodes_balance_event_payload() {
        let balance = shared_types::VenueBalanceInfo {
            venue: "okx".to_owned(),
            currency: "USDT".to_owned(),
            total: 100.0,
            available: 90.0,
            frozen: 10.0,
            unrealized_pnl: 1.0,
        };
        let event =
            SqlBalanceLedgerEvent::from_balance_row("snapshot", &balance, 123).expect("event");
        let replay_event = sql_balance_replay_event(event).expect("replay event");
        let mut replay = SqlLedgerReplay::with_limit(10);

        replay.balance_events.push(replay_event);
        replay.health.replayed_balance_events = replay.balance_events.len();

        assert_eq!(replay.balance_events.len(), 1);
        assert_eq!(replay.balance_events[0].row, balance);
        assert_eq!(replay.balance_events[0].observed_at_ms, 123);
        assert_eq!(replay.health.replayed_balance_events, 1);
        assert_eq!(replay.health.balance_decode_failures, 0);
    }

    #[test]
    fn sql_ledger_init_unconfigured_has_no_writer() {
        let init = SqlLedgerInit::unconfigured(10);

        assert!(!init.migration_health.configured);
        assert!(init.store.is_none());
        assert!(init.replay.events.is_empty());
        assert!(init.replay.order_snapshots.is_empty());
        assert!(init.replay.run_finality_events.is_empty());
        assert_eq!(init.replay.health.query_successes, 0);
    }

    #[test]
    fn sql_replay_decodes_order_event_payload() {
        let event = execution_event();
        let mut replay = SqlLedgerReplay::with_limit(10);

        push_replay_event_row(&mut replay, sql_event_row(&event));

        assert_eq!(replay.events, vec![event]);
        assert_eq!(replay.health.replayed_events, 1);
        assert_eq!(replay.health.event_decode_failures, 0);
    }

    #[test]
    fn sql_replay_counts_bad_order_event_payload() {
        let mut row = sql_event_row(&execution_event()).expect("event row");
        row.payload = serde_json::json!({ "event_id": 1 });
        row.payload_hash = json_hash(&row.payload).expect("payload hash");
        let mut replay = SqlLedgerReplay::with_limit(10);

        push_replay_event_row(&mut replay, Ok(row));

        assert!(replay.events.is_empty());
        assert_eq!(replay.health.replayed_events, 0);
        assert_eq!(replay.health.event_decode_failures, 1);
        assert!(replay
            .health
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("payload decode failed")));
    }

    #[test]
    fn sql_replay_decodes_order_snapshot_record() {
        let record = order_record();
        let mut replay = SqlLedgerReplay::with_limit(10);

        push_replay_order_snapshot_row(&mut replay, sql_order_snapshot_row(&record));

        assert_eq!(replay.order_snapshots, vec![record]);
        assert_eq!(replay.health.replayed_order_snapshots, 1);
        assert_eq!(replay.health.snapshot_decode_failures, 0);
    }

    #[test]
    fn sql_replay_rejects_tampered_event_hash_schema_and_identity() {
        let row = sql_event_row(&execution_event()).expect("event row");
        let mut bad_hash = row.clone();
        bad_hash.payload_hash = "fnv1a64:0000000000000000".to_owned();
        let mut bad_schema = row.clone();
        bad_schema.schema_version -= 1;
        let mut bad_identity = row;
        bad_identity.internal_order_id = "tampered-order".to_owned();
        let mut replay = SqlLedgerReplay::with_limit(10);

        for row in [bad_hash, bad_schema, bad_identity] {
            push_replay_event_row(&mut replay, Ok(row));
        }

        assert!(replay.events.is_empty());
        assert_eq!(replay.health.event_decode_failures, 3);
    }

    #[test]
    fn sql_replay_rejects_tampered_snapshot_hash_schema_and_identity() {
        let row = sql_order_snapshot_row(&order_record()).expect("snapshot row");
        let mut bad_hash = row.clone();
        bad_hash.record_hash = "fnv1a64:0000000000000000".to_owned();
        let mut bad_schema = row.clone();
        bad_schema.schema_version -= 1;
        let mut bad_identity = row;
        bad_identity.internal_order_id = "tampered-order".to_owned();
        let mut replay = SqlLedgerReplay::with_limit(10);

        for row in [bad_hash, bad_schema, bad_identity] {
            push_replay_order_snapshot_row(&mut replay, Ok(row));
        }

        assert!(replay.order_snapshots.is_empty());
        assert_eq!(replay.health.snapshot_decode_failures, 3);
    }

    #[test]
    fn sql_replay_rejects_tampered_balance_hash_and_identity() {
        let balance = shared_types::VenueBalanceInfo {
            venue: "okx".to_owned(),
            currency: "USDT".to_owned(),
            total: 100.0,
            available: 90.0,
            frozen: 10.0,
            unrealized_pnl: 1.0,
        };
        let event =
            SqlBalanceLedgerEvent::from_balance_row("snapshot", &balance, 123).expect("event");
        let mut bad_hash = event.clone();
        bad_hash.payload_hash = "fnv1a64:0000000000000000".to_owned();
        let mut bad_identity = event;
        bad_identity.exchange = "tampered".to_owned();
        let mut replay = SqlLedgerReplay::with_limit(10);

        for event in [bad_hash, bad_identity] {
            push_replay_balance_result(&mut replay, sql_balance_replay_event(event));
        }

        assert!(replay.balance_events.is_empty());
        assert_eq!(replay.health.balance_decode_failures, 2);
    }

    #[test]
    fn sql_replay_rejects_tampered_run_finality_hash_schema_and_identity() {
        let row = replay_event_from_close_run(&close_run(), 456);
        let mut bad_hash = row.clone();
        bad_hash.payload_hash = "fnv1a64:0000000000000000".to_owned();
        let mut bad_schema = row.clone();
        bad_schema.schema_version -= 1;
        let mut bad_identity = row;
        bad_identity.run_id = "tampered-run".to_owned();
        let mut replay = SqlLedgerReplay::with_limit(10);

        for event in [bad_hash, bad_schema, bad_identity] {
            push_replay_run_finality_result(&mut replay, validate_run_finality_replay_event(event));
        }

        assert!(replay.run_finality_events.is_empty());
        assert_eq!(replay.health.run_finality_decode_failures, 3);
    }

    #[test]
    fn typed_replay_hash_accepts_postgres_expanded_numeric() {
        #[derive(Debug, PartialEq, serde::Deserialize, serde::Serialize)]
        struct FloatPayload {
            value: f64,
        }

        let canonical = FloatPayload {
            value: -1.136_868_377_216_160_3e-13,
        };
        let canonical_value = serde_json::to_value(&canonical).expect("canonical payload");
        let stored_hash = json_hash(&canonical_value).expect("canonical hash");
        let expanded_text = r#"{"value":-0.00000000000011368683772161603}"#;
        let expanded: serde_json::Value =
            serde_json::from_str(expanded_text).expect("postgres expanded payload");
        let parsed = "-0.00000000000011368683772161603"
            .parse::<f64>()
            .expect("expanded float");

        assert_eq!(parsed, canonical.value);
        assert_ne!(
            expanded, canonical_value,
            "the regression fixture must exercise the JSONB representation drift",
        );
        assert_ne!(json_hash(&expanded).expect("expanded hash"), stored_hash);
        let normalized = parse_json_with_exact_floats(expanded_text).expect("normalized payload");
        assert_eq!(normalized["value"].as_f64(), Some(canonical.value));
        assert_eq!(
            json_hash(&normalized).expect("normalized hash"),
            stored_hash
        );
        assert_eq!(
            decode_typed_replay_payload::<FloatPayload>(
                "run_finality_events",
                &expanded,
                Some(expanded_text),
                &stored_hash,
            ),
            Ok(canonical)
        );
    }

    #[test]
    fn typed_replay_hash_rejects_unknown_payload_fields() {
        #[derive(Debug, serde::Deserialize, serde::Serialize)]
        struct FloatPayload {
            value: f64,
        }

        let canonical =
            serde_json::to_value(FloatPayload { value: 1.0 }).expect("canonical payload");
        let stored_hash = json_hash(&canonical).expect("canonical hash");
        let tampered = serde_json::json!({"value": 1.0, "extra": true});
        let tampered_text = serde_json::to_string(&tampered).expect("tampered payload text");

        let error = decode_typed_replay_payload::<FloatPayload>(
            "run_finality_events",
            &tampered,
            Some(&tampered_text),
            &stored_hash,
        )
        .expect_err("unknown payload fields must fail integrity validation");

        assert!(error.contains("stored hash does not match payload"));
    }

    #[test]
    fn sql_realized_query_projects_candidate_group_history() {
        let stale_long = fill_event_for_order("stale-hedge-long", 1);
        let stale_short = fill_event_for_order("stale-hedge-short", 2);
        let stale_funding = funding_event_for_order("stale-hedge-long", 4, -0.25);
        let kept_long = fill_event_for_order("kept-hedge-long", 3);
        let kept_funding = funding_event_for_order("kept-hedge-long", 4, 0.33);
        let kept_slippage = slippage_event_for_order("kept-hedge-long", 5, 0.12);
        let kept_short = fill_event_for_order("kept-hedge-short", 10);

        let rows = sql_realized_window_events_from_events(
            vec![
                stale_long,
                stale_short,
                stale_funding,
                kept_long,
                kept_funding,
                kept_slippage,
                kept_short,
            ],
            5,
            20,
        );
        let facts = rows
            .iter()
            .map(|event| {
                (
                    event.order.identity.internal_order_id.as_str(),
                    event.event_type,
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(
            facts,
            [
                (
                    "kept-hedge-long",
                    shared_types::ExecutionLedgerEventType::FillEvent
                ),
                (
                    "kept-hedge-long",
                    shared_types::ExecutionLedgerEventType::FundingPayment
                ),
                (
                    "kept-hedge-long",
                    shared_types::ExecutionLedgerEventType::Slippage
                ),
                (
                    "kept-hedge-short",
                    shared_types::ExecutionLedgerEventType::FillEvent
                )
            ]
        );
    }

    #[test]
    fn sql_realized_window_projects_matching_order_snapshots() {
        let stale_long = fill_event_for_order("stale-hedge-long", 1);
        let stale_short = fill_event_for_order("stale-hedge-short", 2);
        let kept_long = fill_event_for_order("kept-hedge-long", 3);
        let kept_short = fill_event_for_order("kept-hedge-short", 10);

        let window = sql_realized_window_from_parts(
            vec![stale_long, stale_short, kept_long, kept_short],
            vec![
                order_record_for("stale-hedge-long"),
                order_record_for("kept-hedge-long"),
                order_record_for("kept-hedge-short"),
            ],
            5,
            20,
        );
        let snapshot_ids = window
            .order_snapshots
            .iter()
            .map(|record| record.intent.id.as_str())
            .collect::<Vec<_>>();

        assert_eq!(window.events.len(), 2);
        assert_eq!(snapshot_ids, ["kept-hedge-long", "kept-hedge-short"]);
    }

    #[test]
    fn sql_realized_close_runs_filter_linked_pairs_and_keep_latest() {
        let linked_event = fill_event_for_order("linked-hedge-long", 10);
        let stale = close_run_with_pair(
            "close-linked",
            "run-1",
            "ticket-1",
            100,
            shared_types::CloseRunStatus::Submitted,
        );
        let latest = close_run_with_pair(
            "close-linked",
            "run-1",
            "ticket-1",
            200,
            shared_types::CloseRunStatus::Compensated,
        );
        let unrelated = close_run_with_pair(
            "close-other",
            "run-other",
            "ticket-other",
            300,
            shared_types::CloseRunStatus::Succeeded,
        );

        let close_runs = sql_realized_close_runs_from_finality_events(
            &[linked_event],
            vec![
                replay_event_from_close_run(&stale, 100),
                replay_event_from_close_run(&unrelated, 150),
                replay_event_from_close_run(&latest, 200),
            ],
        );

        assert_eq!(close_runs.len(), 1);
        assert_eq!(close_runs[0].id, "close-linked");
        assert_eq!(
            close_runs[0].status,
            shared_types::CloseRunStatus::Compensated
        );
        assert_eq!(close_runs[0].updated_at_ms, 200);
    }

    #[test]
    fn sql_realized_close_runs_reject_any_invalid_finality_row() {
        let valid = replay_event_from_close_run(&close_run(), 456);

        let result = collect_realized_run_finality_events([
            Ok(valid),
            Err("run_finality_events payload hash mismatch".to_owned()),
        ]);

        assert_eq!(
            result.as_ref().err().map(String::as_str),
            Some("run_finality_events payload hash mismatch")
        );
    }

    fn execution_event() -> ExecutionLedgerEvent {
        event_with_payload(
            "evt-1",
            shared_types::ExecutionLedgerEventType::OrderState,
            ExecutionLedgerPayload::OrderState {
                state: shared_types::LiveOrderState::Accepted,
                message: None,
            },
            "ord-1",
            9,
        )
    }

    fn execution_run() -> ExecutionRun {
        ExecutionRun {
            run_id: "run-1".to_owned(),
            ticket_id: "ticket-1".to_owned(),
            opportunity_id: "opp-1".to_owned(),
            state: shared_types::ExecutionRunState::Hedged,
            long_leg: execution_run_leg(shared_types::HedgeLegRole::Long),
            short_leg: execution_run_leg(shared_types::HedgeLegRole::Short),
            net_exposure_usd: 0.0,
            cost_reconciliation: None,
            valuation_problem: None,
            unwind_problem: None,
            finality_problem: None,
            finality_checked_at_ms: None,
            evidence: Default::default(),
            recovery_action: None,
            status_reason: "hedged".to_owned(),
            created_at_ms: 1,
            updated_at_ms: 123,
        }
    }

    fn execution_run_leg(role: shared_types::HedgeLegRole) -> shared_types::ExecutionRunLeg {
        shared_types::ExecutionRunLeg {
            role,
            exchange: "mock".to_owned(),
            symbol: "BTC".to_owned(),
            order_ids: Vec::new(),
            identity: None,
            finality_source: None,
            confirmed_filled_at_ms: None,
            state: shared_types::LiveOrderState::Filled,
            target_quantity: 1.0,
            filled_quantity: Some(1.0),
            target_notional_usd: 100.0,
            filled_notional_usd: Some(100.0),
            filled_fee: Some(0.1),
        }
    }

    fn close_run() -> CloseRun {
        CloseRun {
            id: "close-1".to_owned(),
            scope: shared_types::CloseRunScope::Single,
            status: shared_types::CloseRunStatus::Succeeded,
            action_run_id: None,
            request_id: None,
            idempotency_key: None,
            snapshot_version: "pos-1".to_owned(),
            expected_leg_count: 0,
            reason: None,
            legs: Vec::new(),
            submitted_order_count: 1,
            failed_leg_count: 0,
            naked_exposure_usd: 0.0,
            message: "done".to_owned(),
            problem: None,
            finality_problem: None,
            finality_checked_at_ms: None,
            unwind_plan: None,
            cost_reconciliation: None,
            cost_events: Vec::new(),
            started_at_ms: 1,
            updated_at_ms: 456,
        }
    }

    fn close_run_with_pair(
        id: &str,
        run_id: &str,
        ticket_id: &str,
        updated_at_ms: i64,
        status: shared_types::CloseRunStatus,
    ) -> CloseRun {
        CloseRun {
            id: id.to_owned(),
            scope: shared_types::CloseRunScope::Pair,
            status,
            action_run_id: None,
            request_id: None,
            idempotency_key: None,
            snapshot_version: "pos-1".to_owned(),
            expected_leg_count: 1,
            reason: None,
            legs: vec![shared_types::CloseLeg {
                venue: "mock".to_owned(),
                symbol: "BTC".to_owned(),
                side: shared_types::PositionSide::Long,
                status: shared_types::CloseLegStatus::Filled,
                quantity: 1.0,
                mark_price: 100.0,
                notional_usd: 100.0,
                order: None,
                finality_source: Some(shared_types::OrderUpdateSource::PrivateWs),
                confirmed_filled_at_ms: Some(updated_at_ms),
                problem: None,
                pair_evidence: Some(shared_types::PositionPairEvidence {
                    source: shared_types::PositionPairEvidenceSource::ExecutionRun,
                    run_id: run_id.to_owned(),
                    ticket_id: ticket_id.to_owned(),
                    opportunity_id: "opp-1".to_owned(),
                    venue: "mock".to_owned(),
                    symbol: "BTC".to_owned(),
                    side: shared_types::PositionSide::Long,
                    partner_venue: "okx".to_owned(),
                    partner_symbol: "BTC".to_owned(),
                    partner_side: shared_types::PositionSide::Short,
                    leg_filled_quantity: 1.0,
                    partner_filled_quantity: 1.0,
                    matched_notional_usd: 100.0,
                    updated_at_ms,
                }),
                cost_events: Vec::new(),
            }],
            submitted_order_count: 1,
            failed_leg_count: 0,
            naked_exposure_usd: 0.0,
            message: "linked close".to_owned(),
            problem: None,
            finality_problem: None,
            finality_checked_at_ms: Some(updated_at_ms),
            unwind_plan: None,
            cost_reconciliation: None,
            cost_events: Vec::new(),
            started_at_ms: 1,
            updated_at_ms,
        }
    }

    fn replay_event_from_close_run(
        run: &CloseRun,
        occurred_at_ms: i64,
    ) -> SqlRunFinalityReplayEvent {
        let event = SqlRunFinalityLedgerEvent::from_close_run(
            run,
            shared_types::OrderUpdateSource::OrderQuery,
            None,
            None,
            occurred_at_ms,
        )
        .expect("close run finality event");
        SqlRunFinalityReplayEvent {
            event_id: event.event_id,
            run_kind: event.run_kind,
            run_id: event.run_id,
            source_event_id: event.source_event_id,
            source_order_event_id: event.source_order_event_id,
            source: event.source,
            state: event.state,
            payload: event.payload,
            payload_hash: event.payload_hash,
            schema_version: SQL_LEDGER_SCHEMA_VERSION as i32,
            occurred_at_ms: event.occurred_at_ms,
            captured_at_ms: event.captured_at_ms,
        }
    }

    fn fill_event() -> ExecutionLedgerEvent {
        fill_event_with_order("fill-1", "ord-1", 9)
    }

    fn fill_event_for_order(internal_order_id: &str, occurred_at_ms: i64) -> ExecutionLedgerEvent {
        let event_id = format!("fill-{internal_order_id}");
        fill_event_with_order(&event_id, internal_order_id, occurred_at_ms)
    }

    fn fill_event_with_order(
        event_id: &str,
        internal_order_id: &str,
        occurred_at_ms: i64,
    ) -> ExecutionLedgerEvent {
        event_with_payload(
            event_id,
            shared_types::ExecutionLedgerEventType::FillEvent,
            ExecutionLedgerPayload::FillSnapshot(FillLedgerSnapshot {
                quantity: 2.0,
                average_price: 101.5,
                quote_value: 203.0,
                quality: shared_types::ExecutionLedgerQuality::Actual,
                confidence: shared_types::ExecutionFillConfidence::VenueFill,
                fee: Some(FeeLedgerSnapshot {
                    amount: -0.12,
                    currency: Some("USDC".to_owned()),
                    quality: shared_types::ExecutionLedgerQuality::Actual,
                }),
            }),
            internal_order_id,
            occurred_at_ms,
        )
    }

    fn slippage_event_for_order(
        internal_order_id: &str,
        occurred_at_ms: i64,
        amount_usd: f64,
    ) -> ExecutionLedgerEvent {
        let event_id = format!("slippage-fill-{internal_order_id}");
        event_with_payload(
            &event_id,
            shared_types::ExecutionLedgerEventType::Slippage,
            ExecutionLedgerPayload::Slippage(shared_types::SlippageLedgerRecord {
                amount_usd,
                reference_price: 101.0,
                fill_price: 101.5,
                quantity: 0.24,
                quality: shared_types::ExecutionLedgerQuality::Actual,
            }),
            internal_order_id,
            occurred_at_ms,
        )
    }

    fn fee_event() -> ExecutionLedgerEvent {
        event_with_payload(
            "fee-1",
            shared_types::ExecutionLedgerEventType::FeeSnapshot,
            ExecutionLedgerPayload::FeeSnapshot(FeeLedgerSnapshot {
                amount: -0.33,
                currency: Some("USDT".to_owned()),
                quality: shared_types::ExecutionLedgerQuality::Actual,
            }),
            "ord-1",
            9,
        )
    }

    fn funding_event() -> ExecutionLedgerEvent {
        event_with_payload(
            "funding-1",
            shared_types::ExecutionLedgerEventType::FundingPayment,
            ExecutionLedgerPayload::FundingPayment(FundingPaymentLedgerRecord {
                amount: 1.25,
                currency: "USDC".to_owned(),
                funding_time_ms: 123_000,
                quality: shared_types::ExecutionLedgerQuality::Actual,
            }),
            "ord-1",
            9,
        )
    }

    fn funding_event_for_order(
        internal_order_id: &str,
        occurred_at_ms: i64,
        amount: f64,
    ) -> ExecutionLedgerEvent {
        let event_id = format!("funding-{internal_order_id}");
        event_with_payload(
            &event_id,
            shared_types::ExecutionLedgerEventType::FundingPayment,
            ExecutionLedgerPayload::FundingPayment(FundingPaymentLedgerRecord {
                amount,
                currency: "USDC".to_owned(),
                funding_time_ms: occurred_at_ms,
                quality: shared_types::ExecutionLedgerQuality::Actual,
            }),
            internal_order_id,
            occurred_at_ms,
        )
    }

    fn orderbook_event() -> ExecutionLedgerEvent {
        event_with_payload(
            "orderbook-1",
            shared_types::ExecutionLedgerEventType::OrderbookEvidence,
            ExecutionLedgerPayload::OrderbookEvidence(Box::new(
                shared_types::OrderbookDepthLedgerRecord {
                    reference_price: Some(100.0),
                    bid: Some(99.9),
                    ask: Some(100.1),
                    mid: Some(100.0),
                    open_vwap_price: Some(100.1),
                    open_slippage_bps: Some(1.0),
                    close_vwap_price: Some(99.9),
                    close_slippage_bps: Some(1.0),
                    depth_usd_5bps: Some(500.0),
                    depth_usd_10bps: Some(1000.0),
                    depth_usd_20bps: Some(1500.0),
                    max_notional_usd: Some(1500.0),
                    market_timestamp_ms: Some(123),
                    health: Some(shared_types::MarketDataHealth {
                        quality: shared_types::MarketDataQuality::Fresh,
                        source: shared_types::MarketDataSourceKind::WsPush,
                        freshness_ms: Some(4),
                        retry_after_ms: None,
                        last_error: None,
                        observed_at_ms: 123,
                        coverage: None,
                        problem: None,
                    }),
                    reason: None,
                    quality: shared_types::ExecutionLedgerQuality::Actual,
                },
            )),
            "ord-1",
            123,
        )
    }

    fn event_with_payload(
        event_id: &str,
        event_type: shared_types::ExecutionLedgerEventType,
        payload: ExecutionLedgerPayload,
        internal_order_id: &str,
        occurred_at_ms: i64,
    ) -> ExecutionLedgerEvent {
        ExecutionLedgerEvent {
            event_id: event_id.to_owned(),
            event_type,
            source: shared_types::OrderUpdateSource::PrivateWs,
            order: shared_types::ExecutionLedgerOrderRef {
                run_id: Some("run-1".to_owned()),
                ticket_id: Some("ticket-1".to_owned()),
                leg_role: Some(shared_types::HedgeLegRole::Long),
                reduce_only: None,
                exchange: "mock".to_owned(),
                symbol: "BTC".to_owned(),
                side: shared_types::OrderSide::Buy,
                identity: shared_types::VenueOrderIdentity {
                    account_scope: None,
                    internal_order_id: internal_order_id.to_owned(),
                    public_client_order_id: format!("{internal_order_id}-client"),
                    product: shared_types::FeeProduct::Perp,
                    venue_client_order_id: None,
                    exchange_order_id: Some("ex-1".to_owned()),
                    client_order_id_policy: None,
                    transport_metadata: Default::default(),
                },
            },
            payload,
            occurred_at_ms,
            captured_at_ms: 10,
        }
    }

    fn order_record() -> OrderRecord {
        let intent = shared_types::OrderIntent {
            id: "ord-1".to_owned(),
            source: shared_types::OrderSource::Manual,
            strategy: None,
            mode: shared_types::ExecutionMode::DryRun,
            exchange: "mock".to_owned(),
            symbol: "BTC".to_owned(),
            side: shared_types::OrderSide::Buy,
            order_type: shared_types::OrderType::Limit,
            quantity: 1.0,
            price: Some(10.0),
            slippage_tolerance_bps: None,
            reduce_only: false,
            time_in_force: shared_types::TimeInForce::Ioc,
            post_only: false,
            margin_mode: shared_types::MarginMode::Cross,
            leverage: 1.0,
            client_order_id: "client-1".to_owned(),
            client_order_id_policy: None,
            created_at_ms: 1,
        };
        OrderRecord {
            identity: shared_types::VenueOrderIdentity::from_intent(&intent),
            intent,
            state: shared_types::LiveOrderState::Accepted,
            risk: None,
            last_update_source: shared_types::OrderUpdateSource::PrivateWs,
            exchange_order_id: Some("ex-1".to_owned()),
            message: None,
            filled_quantity: None,
            filled_price: None,
            filled_fee: None,
            updated_at_ms: 10,
        }
    }

    fn order_record_for(id: &str) -> OrderRecord {
        let mut record = order_record();
        record.intent.id = id.to_owned();
        record.intent.client_order_id = format!("{id}-client");
        record.identity = shared_types::VenueOrderIdentity::from_intent(&record.intent);
        record.updated_at_ms = 20;
        record
    }
}
