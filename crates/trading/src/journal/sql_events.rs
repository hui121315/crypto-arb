use super::*;
use crate::{
    SqlLedgerPersistAck, SqlProjectionJob, SqlProjectionJobAck, SqlRunCostFact,
    SqlRunCostRebuildReport,
};

impl OrderJournal {
    pub async fn persist_ledger_event_durable(
        &self,
        event: &ExecutionLedgerEvent,
    ) -> Result<(), String> {
        if let Some(store) = &self.sql_ledger_store {
            return store
                .persist_event(event)
                .await
                .map(|_| ())
                .map_err(|error| error.to_string());
        }
        if self.sql_ledger_migration_health.configured {
            Err("trading SQL ledger is configured but the durable writer is unavailable".into())
        } else {
            Ok(())
        }
    }

    pub async fn persist_ledger_event_group_durable(
        &self,
        events: &[ExecutionLedgerEvent],
    ) -> Result<(), String> {
        if let Some(store) = &self.sql_ledger_store {
            return store
                .persist_event_group(events)
                .await
                .map(|_| ())
                .map_err(|error| error.to_string());
        }
        if self.sql_ledger_migration_health.configured {
            Err("trading SQL ledger is configured but the durable writer is unavailable".into())
        } else {
            Ok(())
        }
    }

    pub async fn claim_sql_projection_jobs(
        &self,
        projector: &str,
        now_ms: i64,
        limit: usize,
        lease_ms: i64,
    ) -> Result<Vec<SqlProjectionJob>, String> {
        if let Some(store) = &self.sql_ledger_store {
            return store
                .claim_projection_jobs(projector, now_ms, limit, lease_ms)
                .await
                .map_err(|error| error.to_string());
        }
        if self.sql_ledger_migration_health.configured {
            Err("trading SQL ledger is configured but projection jobs cannot be claimed".into())
        } else {
            Ok(Vec::new())
        }
    }

    pub async fn complete_sql_projection_job(
        &self,
        job: &SqlProjectionJob,
        completed_at_ms: i64,
    ) -> Result<SqlProjectionJobAck, String> {
        let store = self
            .sql_ledger_store
            .as_ref()
            .ok_or_else(|| "trading SQL projection writer is unavailable".to_owned())?;
        store
            .complete_projection_job(job, completed_at_ms)
            .await
            .map_err(|error| error.to_string())
    }

    pub async fn retry_sql_projection_job(
        &self,
        job: &SqlProjectionJob,
        available_at_ms: i64,
        last_error: &str,
    ) -> Result<SqlProjectionJobAck, String> {
        let store = self
            .sql_ledger_store
            .as_ref()
            .ok_or_else(|| "trading SQL projection writer is unavailable".to_owned())?;
        store
            .retry_projection_job(job, available_at_ms, last_error)
            .await
            .map_err(|error| error.to_string())
    }

    pub async fn drain_sql_ledger_and_shutdown(&self) -> Result<(), String> {
        if let Some(store) = &self.sql_ledger_store {
            return store
                .shutdown()
                .await
                .map_err(|error| error.to_string());
        }
        if self.sql_ledger_migration_health.configured {
            Err("trading SQL ledger is configured but the writer cannot be drained".into())
        } else {
            Ok(())
        }
    }

    pub fn append_balance_ledger_event(&self, event: SqlBalanceLedgerEvent) -> bool {
        if let Some(store) = &self.sql_ledger_store {
            store.append_balance_event(event);
            true
        } else {
            false
        }
    }

    pub fn append_run_finality_event(&self, event: SqlRunFinalityLedgerEvent) -> bool {
        if let Some(store) = &self.sql_ledger_store {
            store.append_run_finality_event(event);
            true
        } else {
            false
        }
    }

    pub async fn persist_run_finality_event(
        &self,
        event: &SqlRunFinalityLedgerEvent,
    ) -> Result<SqlLedgerPersistAck, String> {
        if let Some(store) = &self.sql_ledger_store {
            return store
                .persist_run_finality_event(event)
                .await
                .map_err(|error| error.to_string());
        }
        if self.sql_ledger_migration_health.configured {
            Err("trading SQL ledger is configured but run finality cannot be committed".into())
        } else {
            Ok(SqlLedgerPersistAck::AlreadyPersisted)
        }
    }

    pub async fn project_run_cost_event(
        &self,
        event: &ExecutionLedgerEvent,
    ) -> Result<usize, String> {
        let store = self
            .sql_ledger_store
            .as_ref()
            .ok_or_else(|| "trading SQL run-cost writer is unavailable".to_owned())?;
        store
            .project_run_cost_event(event)
            .await
            .map_err(|error| error.to_string())
    }

    pub async fn rebuild_run_cost_facts(
        &self,
        page_size: usize,
    ) -> Result<SqlRunCostRebuildReport, String> {
        let store = self
            .sql_ledger_store
            .as_ref()
            .ok_or_else(|| "trading SQL run-cost writer is unavailable".to_owned())?;
        store
            .rebuild_run_cost_facts(page_size)
            .await
            .map_err(|error| error.to_string())
    }

    pub async fn query_run_cost_facts(
        &self,
        run_kind: &str,
        run_id: &str,
    ) -> Result<Vec<SqlRunCostFact>, String> {
        let store = self
            .sql_ledger_store
            .as_ref()
            .ok_or_else(|| "trading SQL run-cost writer is unavailable".to_owned())?;
        store
            .query_run_cost_facts(run_kind, run_id)
            .await
            .map_err(|error| error.to_string())
    }
}
