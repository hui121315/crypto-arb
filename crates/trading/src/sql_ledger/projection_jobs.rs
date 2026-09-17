use super::{json_hash, SqlLedgerWriteError};
use shared_types::ExecutionLedgerEvent;
use tokio_postgres::Row;

pub const EXECUTION_RUN_PROJECTOR: &str = "execution_run_v1";
pub const CLOSE_RUN_PROJECTOR: &str = "close_run_v1";
pub const RUN_COST_PROJECTOR: &str = "run_cost_facts_v1";
const SQL_PROJECTION_JOB_MAX_CLAIM_LIMIT: usize = 256;
const SQL_PROJECTION_JOB_MAX_LEASE_MS: i64 = 15 * 60 * 1_000;

const CLAIM_PROJECTION_JOBS_SQL: &str = "\
    WITH candidates AS MATERIALIZED ( \
        SELECT jobs.event_id, jobs.projector \
        FROM ledger_projection_jobs AS jobs \
        WHERE jobs.projector = $1 \
          AND ( \
              (jobs.status = 'pending' AND jobs.available_at_ms <= $2) \
              OR (jobs.status = 'processing' AND jobs.claimed_at_ms <= $2 - $4) \
          ) \
        ORDER BY jobs.available_at_ms, jobs.event_id \
        FOR UPDATE SKIP LOCKED \
        LIMIT $3 \
    ), claim AS MATERIALIZED ( \
        SELECT concat( \
            pg_backend_pid(), ':', txid_current(), ':', clock_timestamp(), ':', random() \
        ) AS claim_token \
    ) \
    UPDATE ledger_projection_jobs AS jobs \
    SET status = 'processing', \
        claim_token = claim.claim_token, \
        claimed_at_ms = $2, \
        completed_at_ms = NULL, \
        last_error = NULL, \
        attempt_count = jobs.attempt_count + 1, \
        updated_at_ms = $2 \
    FROM candidates, claim, order_events AS events \
    WHERE jobs.event_id = candidates.event_id \
      AND jobs.projector = candidates.projector \
      AND events.event_id = jobs.event_id \
    RETURNING jobs.event_id, jobs.projector, jobs.payload_hash, claim.claim_token, \
              jobs.attempt_count, events.payload";

const COMPLETE_PROJECTION_JOB_SQL: &str = "\
    UPDATE ledger_projection_jobs \
    SET status = 'completed', completed_at_ms = $4, updated_at_ms = $4, last_error = NULL \
    WHERE event_id = $1 \
      AND projector = $2 \
      AND status = 'processing' \
      AND claim_token = $3";

const RETRY_PROJECTION_JOB_SQL: &str = "\
    UPDATE ledger_projection_jobs \
    SET status = 'pending', \
        available_at_ms = $4, \
        claimed_at_ms = NULL, \
        completed_at_ms = NULL, \
        claim_token = NULL, \
        last_error = $5, \
        updated_at_ms = $6 \
    WHERE event_id = $1 \
      AND projector = $2 \
      AND status = 'processing' \
      AND claim_token = $3";

#[derive(Debug, Clone, PartialEq)]
pub struct SqlProjectionJob {
    pub event: ExecutionLedgerEvent,
    pub event_id: String,
    pub projector: String,
    pub payload_hash: String,
    pub claim_token: String,
    pub attempt_count: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SqlProjectionJobAck {
    Applied,
    StaleClaim,
}

pub(super) fn validate_claim_request(
    projector: &str,
    now_ms: i64,
    limit: usize,
    lease_ms: i64,
) -> Result<i64, SqlLedgerWriteError> {
    if projector.trim().is_empty() {
        return Err(invalid_request("projector must not be empty"));
    }
    if now_ms < 0 {
        return Err(invalid_request("now_ms must be non-negative"));
    }
    if !(1..=SQL_PROJECTION_JOB_MAX_CLAIM_LIMIT).contains(&limit) {
        return Err(invalid_request(format!(
            "limit must be between 1 and {SQL_PROJECTION_JOB_MAX_CLAIM_LIMIT}"
        )));
    }
    if !(1..=SQL_PROJECTION_JOB_MAX_LEASE_MS).contains(&lease_ms) {
        return Err(invalid_request(format!(
            "lease_ms must be between 1 and {SQL_PROJECTION_JOB_MAX_LEASE_MS}"
        )));
    }
    i64::try_from(limit).map_err(|_| invalid_request("limit exceeds PostgreSQL BIGINT"))
}

pub(super) async fn claim_projection_jobs(
    client: &mut tokio_postgres::Client,
    projector: &str,
    now_ms: i64,
    limit: i64,
    lease_ms: i64,
) -> Result<Vec<SqlProjectionJob>, SqlLedgerWriteError> {
    let transaction = client
        .transaction()
        .await
        .map_err(|error| postgres_error(&error))?;
    let rows = transaction
        .query(
            CLAIM_PROJECTION_JOBS_SQL,
            &[&projector, &now_ms, &limit, &lease_ms],
        )
        .await
        .map_err(|error| postgres_error(&error))?;
    let jobs = rows
        .iter()
        .map(decode_projection_job)
        .collect::<Result<Vec<_>, _>>()?;
    transaction
        .commit()
        .await
        .map_err(|error| postgres_error(&error))?;
    Ok(jobs)
}

pub(super) async fn complete_projection_job(
    client: &tokio_postgres::Client,
    event_id: &str,
    projector: &str,
    claim_token: &str,
    completed_at_ms: i64,
) -> Result<SqlProjectionJobAck, SqlLedgerWriteError> {
    let updated = client
        .execute(
            COMPLETE_PROJECTION_JOB_SQL,
            &[&event_id, &projector, &claim_token, &completed_at_ms],
        )
        .await
        .map_err(|error| postgres_error(&error))?;
    Ok(mutation_ack(updated))
}

pub(super) struct RetryProjectionJob<'a> {
    pub(super) event_id: &'a str,
    pub(super) projector: &'a str,
    pub(super) claim_token: &'a str,
    pub(super) available_at_ms: i64,
    pub(super) last_error: &'a str,
    pub(super) updated_at_ms: i64,
}

pub(super) async fn retry_projection_job(
    client: &tokio_postgres::Client,
    request: &RetryProjectionJob<'_>,
) -> Result<SqlProjectionJobAck, SqlLedgerWriteError> {
    let updated = client
        .execute(
            RETRY_PROJECTION_JOB_SQL,
            &[
                &request.event_id,
                &request.projector,
                &request.claim_token,
                &request.available_at_ms,
                &request.last_error,
                &request.updated_at_ms,
            ],
        )
        .await
        .map_err(|error| postgres_error(&error))?;
    Ok(mutation_ack(updated))
}

fn decode_projection_job(row: &Row) -> Result<SqlProjectionJob, SqlLedgerWriteError> {
    let event_id = row_string(row, "event_id")?;
    let projector = row_string(row, "projector")?;
    let payload_hash = row_string(row, "payload_hash")?;
    let claim_token = row_string(row, "claim_token")?;
    let attempt_count = row
        .try_get("attempt_count")
        .map_err(|error| encoding_error("attempt_count", &error))?;
    let payload = row
        .try_get::<_, serde_json::Value>("payload")
        .map_err(|error| encoding_error("payload", &error))?;
    let event = serde_json::from_value::<ExecutionLedgerEvent>(payload.clone())
        .map_err(|error| SqlLedgerWriteError::Encoding(format!("projection payload: {error}")))?;
    let decoded_hash = json_hash(&payload).map_err(SqlLedgerWriteError::Encoding)?;
    if event.event_id != event_id || decoded_hash != payload_hash {
        return Err(SqlLedgerWriteError::Encoding(format!(
            "projection job integrity mismatch for event_id {event_id}"
        )));
    }
    Ok(SqlProjectionJob {
        event,
        event_id,
        projector,
        payload_hash,
        claim_token,
        attempt_count,
    })
}

fn row_string(row: &Row, column: &str) -> Result<String, SqlLedgerWriteError> {
    row.try_get(column)
        .map_err(|error| encoding_error(column, &error))
}

fn encoding_error(column: &str, error: &tokio_postgres::Error) -> SqlLedgerWriteError {
    SqlLedgerWriteError::Encoding(format!("projection job {column}: {error}"))
}

fn invalid_request(message: impl Into<String>) -> SqlLedgerWriteError {
    SqlLedgerWriteError::InvalidProjectionJobRequest(message.into())
}

fn postgres_error(error: &tokio_postgres::Error) -> SqlLedgerWriteError {
    SqlLedgerWriteError::Postgres(error.to_string())
}

const fn mutation_ack(updated: u64) -> SqlProjectionJobAck {
    if updated == 1 {
        SqlProjectionJobAck::Applied
    } else {
        SqlProjectionJobAck::StaleClaim
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claim_query_filters_projector_before_locking() {
        assert!(CLAIM_PROJECTION_JOBS_SQL.contains("WHERE jobs.projector = $1"));
        assert!(CLAIM_PROJECTION_JOBS_SQL.contains("FOR UPDATE SKIP LOCKED"));
        assert!(CLAIM_PROJECTION_JOBS_SQL.contains("jobs.status = 'pending'"));
        assert!(CLAIM_PROJECTION_JOBS_SQL.contains("jobs.status = 'processing'"));
    }

    #[test]
    fn claim_bounds_reject_zero_and_excessive_work() {
        assert!(validate_claim_request(EXECUTION_RUN_PROJECTOR, 1, 0, 1).is_err());
        assert!(validate_claim_request(
            EXECUTION_RUN_PROJECTOR,
            1,
            SQL_PROJECTION_JOB_MAX_CLAIM_LIMIT + 1,
            1
        )
        .is_err());
        assert!(validate_claim_request(
            EXECUTION_RUN_PROJECTOR,
            1,
            1,
            SQL_PROJECTION_JOB_MAX_LEASE_MS + 1
        )
        .is_err());
    }
}
