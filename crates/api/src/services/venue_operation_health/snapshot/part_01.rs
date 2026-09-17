use crate::lifecycle::nav_persist::{self, NavStorageHealth};
use crate::middleware::audit::{self, AuditSinkHealthSnapshot};
use crate::services::live_order_proof_health::{
    LiveOrderProofRuntimeHealth, LiveOrderProofSample, SOURCE_LIVE_ORDER_PROOF_RUNTIME,
};
use crate::services::market_data::{MarketQuality, MarketRuntimeHealth};
use crate::services::private_ws_health::{PrivateWsRuntimeHealth, SOURCE_PRIVATE_WS_RUNTIME};
use crate::services::reconciliation_health::{
    ReconciliationRuntimeHealth, GLOBAL_RECONCILIATION_VENUE, SOURCE_RECONCILIATION_RUNTIME,
};
use crate::services::run_finality_health::{
    RunFinalityRuntimeHealth, GLOBAL_RUN_FINALITY_VENUE, SOURCE_RUN_FINALITY_RUNTIME,
};
use crate::services::venue_credentials;
use crate::state::AppState;
use crate::task_registry::{TaskIssue, TaskIssueKind, TaskSnapshot};
use crate::trading_service::{
    AccountCacheQuality, AccountCacheSnapshot, BALANCE_CACHE_TTL_MS, POSITION_CACHE_MAX_STALE_MS,
};
use exchange::{
    EndpointDataKind, EndpointEvidenceSnapshot, EndpointSpec, EndpointUseCase, HostGateSnapshot,
    HttpOutcomeMetricSnapshot, RateLimiterSnapshot,
};
use realtime::HistoryStoreHealth;
use shared_types::{
    credential_probe_operation, normalized_venue_name, problem::codes, venue_family, ApiProblem,
    ExchangeWsOperation, ExchangeWsVenue, ExecutionMode, HistoryTimescaleStatus, LiveOrderState,
    OrderRecord, StorageDegradedReason, VenueCredentialProbe, VenueCredentialProbeStatus,
    VenueCredentialStatus, VenueCredentialValidationEvidence, VenueCredentialValidationStatus,
    VenueId, VenueOperationEvidence, VenueOperationHealth, VenueOperationHealthSnapshot,
    VenueOperationKind, VenueOperationStatus, OP_BACKGROUND_TASKS, OP_BACKGROUND_TASK_PREFIX,
    OP_BALANCE, OP_HOST_GATE_PREFIX, OP_ORDER_FINALITY, OP_ORDER_RECONCILIATION, OP_ORDER_WRITE,
    OP_POSITIONS, OP_PRIVATE_READ, OP_PRIVATE_WS_ACCOUNT_STREAM, OP_PRIVATE_WS_ORDER_STREAM,
    OP_PRIVATE_WS_SESSION, OP_PRIVATE_WS_SUBSCRIBE, OP_RATE_LIMITER_PREFIX,
    OP_STORAGE_AUDIT_LOG as OP_AUDIT_LOG_STORAGE,
    OP_STORAGE_EXECUTION_LEDGER as OP_EXECUTION_LEDGER_STORAGE,
    OP_STORAGE_HISTORY as OP_HISTORY_STORAGE,
    OP_STORAGE_ORDER_SNAPSHOT as OP_ORDER_SNAPSHOT_STORAGE,
    OP_STORAGE_PORTFOLIO_NAV as OP_NAV_STORAGE,
    OP_STORAGE_TRADING_SQL_LEDGER as OP_TRADING_SQL_LEDGER_STORAGE,
    OP_STORAGE_TRADING_SQL_MIGRATIONS as OP_TRADING_SQL_MIGRATIONS_STORAGE,
    UNRECORDED_EVIDENCE_MARKER,
};
use std::collections::BTreeMap;
use trading::{
    ExecutionLedgerStorageSnapshot, OrderSnapshotStorageSnapshot, SqlLedgerMigrationHealth,
    SqlLedgerStorageSnapshot,
};

const SOURCE_ACCOUNT_CACHE: &str = "account_cache";
const SOURCE_CREDENTIAL_CONFIG: &str = "credential_config";
const SOURCE_CREDENTIAL_VALIDATION: &str = "credential_validation";
const SOURCE_AUDIT_LOG_STORAGE: &str = "audit_log_jsonl";
const SOURCE_HISTORY_STORAGE: &str = "history_store";
const SOURCE_EXECUTION_LEDGER_STORAGE: &str = "execution_ledger_jsonl";
const SOURCE_ORDER_SNAPSHOT_STORAGE: &str = "order_snapshot_jsonl";
const SOURCE_TRADING_SQL_MIGRATION: &str = "trading_sql_migration_runner";
const SOURCE_TRADING_SQL_LEDGER_WRITER: &str = "trading_sql_ledger_writer";
const SOURCE_HOST_GATE: &str = "host_gate";
const SOURCE_HTTP_OUTCOME_METRICS: &str = "exchange_http_metrics";
const SOURCE_MARKET_DATA_CACHE: &str = "market_data_cache";
const SOURCE_INSTRUMENT_REGISTRY: &str = "instrument_registry";
const SOURCE_OPPORTUNITY_INDEX: &str = "opportunity_index";
const SOURCE_NAV_STORAGE: &str = "portfolio_nav_store";
const SOURCE_RATE_LIMITER: &str = "rate_limiter";
const SOURCE_TASK_REGISTRY: &str = "task_registry";
const SYSTEM_VENUE: &str = "system";
const HTTP_ERROR_RECENT_MS: i64 = 60_000;
/// 成功 endpoint 的 p95 延迟超过该阈值时降为 Warn，让“慢交易所”能定位到具体慢 endpoint。
const HTTP_SLOW_P95_WARN_MS: u64 = 2_500;
const HISTORY_STORAGE_ERROR_RECENT_MS: i64 = 60_000;
const NAV_STORAGE_ERROR_RECENT_MS: i64 = 60_000;
const ORDER_STREAM_PROBE_RETRY_AFTER_MS: u64 = 60_000;
const RATE_LIMITER_RECENT_MS: i64 = 60_000;
const RATE_LIMITER_PRESSURE_WAIT_MS: u64 = 10_000;
const PRIVATE_WS_OPS: [&str; 4] = [
    OP_PRIVATE_WS_SESSION,
    OP_PRIVATE_WS_SUBSCRIBE,
    OP_PRIVATE_WS_ORDER_STREAM,
    OP_PRIVATE_WS_ACCOUNT_STREAM,
];
const POSTGRES_CREATE_TABLE_DOC_URL: &str =
    "https://www.postgresql.org/docs/current/sql-createtable.html";
const POSTGRES_INSERT_DOC_URL: &str = "https://www.postgresql.org/docs/current/sql-insert.html";
const POSTGRES_SELECT_DOC_URL: &str = "https://www.postgresql.org/docs/current/sql-select.html";
const POSTGRES_JSON_DOC_URL: &str = "https://www.postgresql.org/docs/current/functions-json.html";

pub(crate) fn snapshot(state: &AppState) -> VenueOperationHealthSnapshot {
    let now_ms = common::time::now_ms();
    let credentials = venue_credentials::status();
    let mut rows = credential_rows(&credentials.venues, now_ms);
    let live_order_proofs = state.live_order_proof_health().snapshot(now_ms);
    overlay_order_write_runtime_rows(
        &mut rows,
        order_write_runtime_rows(&credentials.venues, live_order_proofs.clone(), now_ms),
    );
    rows.extend(credential_validation_rows(
        &credentials.venues,
        venue_credentials::validation_evidence_snapshot(),
        now_ms,
    ));
    let (balance_cache, position_cache) =
        append_account_runtime_rows(&mut rows, &credentials.venues, state, now_ms);
    let unresolved_orders = if state.trading_service().open_order_count() == 0 {
        Vec::new()
    } else {
        state.trading_service().list_orders()
    };
    let mut private_ws_health = state.private_ws_health().snapshot(now_ms);
    resolve_completed_account_refetches(
        &mut private_ws_health,
        &balance_cache,
        &position_cache,
        now_ms,
    );
    let mut private_ws_rows = private_ws_runtime_rows(
        &credentials.venues,
        private_ws_health,
        &unresolved_orders,
        now_ms,
    );
    overlay_order_stream_readiness_from_live_proof(&mut private_ws_rows, &live_order_proofs);
    overlay_idle_private_ws_readiness(&mut private_ws_rows, &balance_cache, &position_cache);
    rows.extend(private_ws_rows);
    rows.extend(hyperliquid_signer_session_rows(now_ms));
    rows.extend(reconciliation_runtime_rows(
        &credentials.venues,
        state.reconciliation_health().snapshot(now_ms),
        now_ms,
    ));
    rows.extend(run_finality_runtime_rows(
        &credentials.venues,
        state.run_finality_health().snapshot(now_ms),
        now_ms,
    ));
    rows.extend(http_outcome_rows(
        exchange::http_outcome_metrics_snapshot(),
        now_ms,
    ));
    rows.extend(host_gate_rows(
        exchange::host_gate_snapshots(now_ms),
        now_ms,
    ));
    let rate_limiter_snapshots = exchange::rate_limiter_snapshots(now_ms);
    rows.extend(rate_limiter_rows(&rate_limiter_snapshots, now_ms));
    rows.extend(task_registry_rows(state.task_registry(), now_ms));
    rows.extend(app_ws_broadcast_rows(state, now_ms));
    rows.push(audit_log_storage_health_row(now_ms));
    let history_health = state.history_store().health_snapshot(now_ms);
    rows.push(history_storage_row(&history_health, now_ms));
    rows.push(execution_ledger_storage_health_row(state, now_ms));
    rows.push(order_snapshot_storage_health_row(state, now_ms));
    rows.push(trading_sql_migration_storage_health_row(state, now_ms));
    rows.push(trading_sql_ledger_storage_health_row(state, now_ms));
    rows.push(portfolio_nav_storage_health_row(state, now_ms));
    rows.push(watchlist_alert_storage_health_row(state, now_ms));
    rows.push(watchlist_prewarm_health_row(state, now_ms));
    rows.extend(instrument_registry_rows(state, now_ms));
    rows.push(opportunity_snapshot_row(state, now_ms));
    rows.extend(
        state
            .market_data()
            .runtime_health_snapshot()
            .into_iter()
            .map(|row| market_row(row, now_ms)),
    );
    rows.sort_by(|left, right| {
        left.venue
            .cmp(&right.venue)
            .then_with(|| left.operation.cmp(&right.operation))
            .then_with(|| left.source.cmp(&right.source))
    });
    VenueOperationHealthSnapshot::new(rows, now_ms)
}

pub(crate) fn portfolio_nav_storage_health_row(
    state: &AppState,
    now_ms: i64,
) -> VenueOperationHealth {
    let health = state.portfolio_nav_storage_health().snapshot(now_ms);
    nav_storage_row(&health, now_ms)
}

pub(crate) fn execution_ledger_storage_health_row(
    state: &AppState,
    now_ms: i64,
) -> VenueOperationHealth {
    let snapshot = state.trading_service().execution_ledger_storage_snapshot();
    execution_ledger_storage_row(&snapshot, now_ms)
}

pub(crate) fn order_snapshot_storage_health_row(
    state: &AppState,
    now_ms: i64,
) -> VenueOperationHealth {
    let snapshot = state.trading_service().order_snapshot_storage_snapshot();
    order_snapshot_storage_row(&snapshot, now_ms)
}

pub(crate) fn trading_sql_migration_storage_health_row(
    state: &AppState,
    now_ms: i64,
) -> VenueOperationHealth {
    trading_sql_migration_storage_row(state.trading_sql_ledger_health(), now_ms)
}

pub(crate) fn trading_sql_ledger_storage_health_row(
    state: &AppState,
    now_ms: i64,
) -> VenueOperationHealth {
    let snapshot = state.trading_service().sql_ledger_storage_snapshot();
    trading_sql_ledger_storage_row(&snapshot, now_ms)
}

pub(crate) fn audit_log_storage_health_row(now_ms: i64) -> VenueOperationHealth {
    let snapshot = audit::health_snapshot(now_ms);
    audit_log_storage_row(&snapshot, now_ms)
}

fn credential_rows(
    credentials: &[VenueCredentialStatus],
    observed_at_ms: i64,
) -> Vec<VenueOperationHealth> {
    credentials
        .iter()
        .flat_map(|venue| {
            [
                credential_row(venue, OP_PRIVATE_READ, venue.private_read, observed_at_ms),
                credential_row(venue, OP_ORDER_WRITE, venue.live_write, observed_at_ms),
            ]
        })
        .collect()
}

fn account_cache_rows(
    credentials: &[VenueCredentialStatus],
    snapshots: Vec<AccountCacheSnapshot>,
    operation: &str,
    observed_at_ms: i64,
) -> Vec<VenueOperationHealth> {
    let mut by_venue = snapshots
        .into_iter()
        .map(|snapshot| (normalized_venue_name(&snapshot.venue), snapshot))
        .collect::<BTreeMap<_, _>>();
    let mut rows = credentials
        .iter()
        .map(|venue| {
            let key = normalized_venue_name(&venue.venue);
            match by_venue.remove(&key) {
                Some(snapshot) => account_cache_row(operation, snapshot, Some(venue)),
                None => account_cache_missing_row(venue, operation, observed_at_ms),
            }
        })
        .collect::<Vec<_>>();
    rows.extend(
        by_venue
            .into_values()
            .map(|snapshot| account_cache_row(operation, snapshot, None)),
    );
    rows
}
