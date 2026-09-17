use super::{write_balance_event, write_order_snapshot, SqlLedgerWriteStats};
use shared_types::ExecutionLedgerEvent;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

mod lifecycle;
mod protocol;

pub(super) use lifecycle::{send_barrier, ShutdownStart, WriterControl};
pub(super) use protocol::{classify_event_write, EventWriteError};
use protocol::{persist_event, record_event_result};
pub use protocol::{SqlLedgerPersistAck, SqlLedgerWriteError};

pub(super) const ACK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

pub(super) enum SqlLedgerWrite {
    Event(Box<ExecutionLedgerEvent>),
    DurableEvent {
        event: Box<ExecutionLedgerEvent>,
        ack: oneshot::Sender<Result<SqlLedgerPersistAck, SqlLedgerWriteError>>,
    },
    DurableEventGroup {
        events: Vec<ExecutionLedgerEvent>,
        ack: oneshot::Sender<Result<Vec<SqlLedgerPersistAck>, SqlLedgerWriteError>>,
    },
    ClaimProjectionJobs {
        projector: String,
        now_ms: i64,
        limit: i64,
        lease_ms: i64,
        ack: oneshot::Sender<Result<Vec<super::SqlProjectionJob>, SqlLedgerWriteError>>,
    },
    CompleteProjectionJob {
        event_id: String,
        projector: String,
        claim_token: String,
        completed_at_ms: i64,
        ack: oneshot::Sender<Result<super::SqlProjectionJobAck, SqlLedgerWriteError>>,
    },
    RetryProjectionJob {
        event_id: String,
        projector: String,
        claim_token: String,
        available_at_ms: i64,
        last_error: String,
        updated_at_ms: i64,
        ack: oneshot::Sender<Result<super::SqlProjectionJobAck, SqlLedgerWriteError>>,
    },
    OrderSnapshot(Box<shared_types::OrderRecord>),
    BalanceEvent(Box<super::SqlBalanceLedgerEvent>),
    RunFinality(Box<super::SqlRunFinalityLedgerEvent>),
    DurableRunFinality {
        event: Box<super::SqlRunFinalityLedgerEvent>,
        ack: oneshot::Sender<Result<SqlLedgerPersistAck, SqlLedgerWriteError>>,
    },
    ProjectRunCost {
        event: Box<ExecutionLedgerEvent>,
        ack: oneshot::Sender<Result<usize, SqlLedgerWriteError>>,
    },
    Drain(oneshot::Sender<()>),
    Shutdown(oneshot::Sender<()>),
}

pub(super) async fn run(
    mut client: tokio_postgres::Client,
    mut receiver: mpsc::Receiver<SqlLedgerWrite>,
    stats: Arc<SqlLedgerWriteStats>,
    control: Arc<WriterControl>,
) {
    let shutdown_ack = loop {
        let Some(write) = receiver.recv().await else {
            break None;
        };
        if let Some(ack) = handle_write(&mut client, &stats, write).await {
            break Some(ack);
        }
    };
    receiver.close();
    drop(receiver);
    control.mark_stopped();
    if let Some(ack) = shutdown_ack {
        let _ = ack.send(());
    }
}

async fn handle_write(
    client: &mut tokio_postgres::Client,
    stats: &SqlLedgerWriteStats,
    write: SqlLedgerWrite,
) -> Option<oneshot::Sender<()>> {
    match write {
        SqlLedgerWrite::Event(event) => {
            let result = persist_event(client, &event).await;
            record_event_result(stats, &result);
        }
        SqlLedgerWrite::DurableEvent { event, ack } => {
            let result = persist_event(client, &event).await;
            record_event_result(stats, &result);
            let _ = ack.send(result.map_err(EventWriteError::into_public));
        }
        SqlLedgerWrite::DurableEventGroup { events, ack } => {
            let result = protocol::persist_event_group(client, &events).await;
            protocol::record_event_group_result(stats, events.len(), &result);
            let _ = ack.send(result.map_err(EventWriteError::into_public));
        }
        SqlLedgerWrite::ClaimProjectionJobs {
            projector,
            now_ms,
            limit,
            lease_ms,
            ack,
        } => {
            let result = super::projection_jobs::claim_projection_jobs(
                client, &projector, now_ms, limit, lease_ms,
            )
            .await;
            let _ = ack.send(result);
        }
        SqlLedgerWrite::CompleteProjectionJob {
            event_id,
            projector,
            claim_token,
            completed_at_ms,
            ack,
        } => {
            let result = super::projection_jobs::complete_projection_job(
                client,
                &event_id,
                &projector,
                &claim_token,
                completed_at_ms,
            )
            .await;
            let _ = ack.send(result);
        }
        SqlLedgerWrite::RetryProjectionJob {
            event_id,
            projector,
            claim_token,
            available_at_ms,
            last_error,
            updated_at_ms,
            ack,
        } => {
            let request = super::projection_jobs::RetryProjectionJob {
                event_id: &event_id,
                projector: &projector,
                claim_token: &claim_token,
                available_at_ms,
                last_error: &last_error,
                updated_at_ms,
            };
            let result = super::projection_jobs::retry_projection_job(client, &request).await;
            let _ = ack.send(result);
        }
        SqlLedgerWrite::OrderSnapshot(record) => {
            write_order_snapshot(client, &record, stats).await;
        }
        SqlLedgerWrite::BalanceEvent(event) => {
            write_balance_event(client, &event, stats).await;
        }
        SqlLedgerWrite::RunFinality(event) => {
            let result = protocol::persist_run_finality(client, &event).await;
            protocol::record_run_finality_result(stats, &result);
        }
        SqlLedgerWrite::DurableRunFinality { event, ack } => {
            let result = protocol::persist_run_finality(client, &event).await;
            protocol::record_run_finality_result(stats, &result);
            let _ = ack.send(result.map_err(EventWriteError::into_public));
        }
        SqlLedgerWrite::ProjectRunCost { event, ack } => {
            let result = super::run_cost::project_order_event_transaction(client, &event)
                .await
                .map_err(EventWriteError::into_public);
            let _ = ack.send(result);
        }
        SqlLedgerWrite::Drain(ack) => {
            let _ = ack.send(());
        }
        SqlLedgerWrite::Shutdown(ack) => return Some(ack),
    }
    None
}

#[cfg(test)]
mod tests;
