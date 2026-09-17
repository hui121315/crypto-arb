use super::super::{
    run_finality, sql_event_identity_matches, sql_event_row, write_event_group_transaction,
    write_event_transaction, SqlEventRow, SqlLedgerWriteStats, SqlRunFinalityLedgerEvent,
};
use shared_types::ExecutionLedgerEvent;

const MAX_EVENT_WRITE_ATTEMPTS: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SqlLedgerPersistAck {
    Committed,
    AlreadyPersisted,
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum SqlLedgerWriteError {
    #[error("SQL ledger writer queue remained full until the enqueue timeout")]
    QueueFull,
    #[error("SQL ledger writer queue is closed")]
    QueueClosed,
    #[error("SQL ledger writer acknowledgement timed out")]
    AckTimeout,
    #[error("SQL ledger writer stopped before acknowledging the request")]
    AckClosed,
    #[error("SQL ledger event integrity conflict for event_id {event_id}")]
    IntegrityConflict { event_id: String },
    #[error("SQL ledger event encoding failed: {0}")]
    Encoding(String),
    #[error("invalid SQL projection job request: {0}")]
    InvalidProjectionJobRequest(String),
    #[error("SQL ledger PostgreSQL write failed: {0}")]
    Postgres(String),
}

impl From<tokio_postgres::Error> for SqlLedgerWriteError {
    fn from(error: tokio_postgres::Error) -> Self {
        Self::Postgres(error.to_string())
    }
}

#[derive(Debug)]
pub(in crate::sql_ledger) enum EventWriteError {
    IntegrityConflict { event_id: String },
    Encoding(String),
    Postgres(tokio_postgres::Error),
}

impl From<tokio_postgres::Error> for EventWriteError {
    fn from(error: tokio_postgres::Error) -> Self {
        Self::Postgres(error)
    }
}

impl EventWriteError {
    pub(in crate::sql_ledger) fn into_public(self) -> SqlLedgerWriteError {
        match self {
            Self::IntegrityConflict { event_id } => {
                SqlLedgerWriteError::IntegrityConflict { event_id }
            }
            Self::Encoding(error) => SqlLedgerWriteError::Encoding(error),
            Self::Postgres(error) => SqlLedgerWriteError::Postgres(error.to_string()),
        }
    }

    pub(super) fn message(&self) -> String {
        match self {
            Self::IntegrityConflict { event_id } => {
                format!("order_events integrity conflict for event_id {event_id}")
            }
            Self::Encoding(error) => error.clone(),
            Self::Postgres(error) => format!("order event transaction failed: {error}"),
        }
    }
}

pub(super) async fn persist_event(
    client: &mut tokio_postgres::Client,
    event: &ExecutionLedgerEvent,
) -> Result<SqlLedgerPersistAck, EventWriteError> {
    let row = sql_event_row(event).map_err(EventWriteError::Encoding)?;
    let mut attempt = 1;
    loop {
        match write_event_transaction(client, event, &row).await {
            Ok(ack) => return Ok(ack),
            Err(EventWriteError::Postgres(error))
                if retry_decision(
                    attempt,
                    classify_postgres_failure(
                        error.is_closed(),
                        error.code().map(|code| code.code()),
                    ),
                ) == RetryDecision::Retry =>
            {
                tokio::time::sleep(retry_delay(attempt)).await;
                attempt += 1;
            }
            Err(error) => return Err(error),
        }
    }
}

pub(super) async fn persist_event_group(
    client: &mut tokio_postgres::Client,
    events: &[ExecutionLedgerEvent],
) -> Result<Vec<SqlLedgerPersistAck>, EventWriteError> {
    let mut attempt = 1;
    loop {
        match write_event_group_transaction(client, events).await {
            Ok(acks) => return Ok(acks),
            Err(EventWriteError::Postgres(error))
                if retry_decision(
                    attempt,
                    classify_postgres_failure(
                        error.is_closed(),
                        error.code().map(|code| code.code()),
                    ),
                ) == RetryDecision::Retry =>
            {
                tokio::time::sleep(retry_delay(attempt)).await;
                attempt += 1;
            }
            Err(error) => return Err(error),
        }
    }
}

pub(super) async fn persist_run_finality(
    client: &mut tokio_postgres::Client,
    event: &SqlRunFinalityLedgerEvent,
) -> Result<SqlLedgerPersistAck, EventWriteError> {
    let mut attempt = 1;
    loop {
        match run_finality::write_transaction(client, event).await {
            Ok(ack) => return Ok(ack),
            Err(EventWriteError::Postgres(error))
                if retry_decision(
                    attempt,
                    classify_postgres_failure(
                        error.is_closed(),
                        error.code().map(|code| code.code()),
                    ),
                ) == RetryDecision::Retry =>
            {
                tokio::time::sleep(retry_delay(attempt)).await;
                attempt += 1;
            }
            Err(error) => return Err(error),
        }
    }
}

pub(super) fn record_event_result(
    stats: &SqlLedgerWriteStats,
    result: &Result<SqlLedgerPersistAck, EventWriteError>,
) {
    match result {
        Ok(_) => stats.record_event_success(),
        Err(error) => stats.record_event_error(&error.message()),
    }
}

pub(super) fn record_event_group_result(
    stats: &SqlLedgerWriteStats,
    event_count: usize,
    result: &Result<Vec<SqlLedgerPersistAck>, EventWriteError>,
) {
    match result {
        Ok(_) => {
            for _ in 0..event_count {
                stats.record_event_success();
            }
        }
        Err(error) => stats.record_event_errors(event_count, &error.message()),
    }
}

pub(super) fn record_run_finality_result(
    stats: &SqlLedgerWriteStats,
    result: &Result<SqlLedgerPersistAck, EventWriteError>,
) {
    match result {
        Ok(_) => stats.record_run_finality_success(),
        Err(error) => stats.record_run_finality_error(&error.message()),
    }
}

pub(in crate::sql_ledger) fn classify_event_write(
    inserted: bool,
    existing: &SqlEventRow,
    requested: &SqlEventRow,
) -> Result<SqlLedgerPersistAck, EventWriteError> {
    if inserted {
        return Ok(SqlLedgerPersistAck::Committed);
    }
    if sql_event_identity_matches(existing, requested)
        && existing.payload == requested.payload
        && existing.payload_hash == requested.payload_hash
        && existing.schema_version == requested.schema_version
    {
        Ok(SqlLedgerPersistAck::AlreadyPersisted)
    } else {
        Err(EventWriteError::IntegrityConflict {
            event_id: requested.event_id.clone(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RetryDecision {
    Retry,
    Stop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PostgresFailure {
    Retryable,
    Terminal,
}

pub(super) fn retry_decision(attempt: usize, failure: PostgresFailure) -> RetryDecision {
    if attempt >= MAX_EVENT_WRITE_ATTEMPTS || failure == PostgresFailure::Terminal {
        RetryDecision::Stop
    } else {
        RetryDecision::Retry
    }
}

pub(super) fn classify_postgres_failure(
    connection_closed: bool,
    sqlstate: Option<&str>,
) -> PostgresFailure {
    if connection_closed || sqlstate.is_some_and(|code| code.starts_with("08")) {
        return PostgresFailure::Terminal;
    }
    if matches!(sqlstate, Some("40001" | "40P01" | "55P03")) {
        PostgresFailure::Retryable
    } else {
        PostgresFailure::Terminal
    }
}

fn retry_delay(attempt: usize) -> std::time::Duration {
    const DELAYS_MS: [u64; MAX_EVENT_WRITE_ATTEMPTS - 1] = [25, 100];
    std::time::Duration::from_millis(DELAYS_MS[attempt.saturating_sub(1).min(DELAYS_MS.len() - 1)])
}
