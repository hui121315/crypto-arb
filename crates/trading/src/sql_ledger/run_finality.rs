use super::{
    run_cost, writer, SqlLedgerPersistAck, SqlRunFinalityLedgerEvent, SQL_LEDGER_SCHEMA_VERSION,
};
use tokio_postgres::GenericClient;

pub(super) async fn write_transaction(
    client: &mut tokio_postgres::Client,
    event: &SqlRunFinalityLedgerEvent,
) -> Result<SqlLedgerPersistAck, writer::EventWriteError> {
    let transaction = client.transaction().await?;
    let inserted = insert(&transaction, event).await?;
    let ack = if inserted == 0 {
        let existing = read_existing(&transaction, &event.event_id).await?;
        classify_write(false, &existing, event)?
    } else {
        classify_write(true, event, event)?
    };
    run_cost::upsert_finality_facts(&transaction, event).await?;
    transaction.commit().await?;
    Ok(ack)
}

async fn insert<C>(
    client: &C,
    event: &SqlRunFinalityLedgerEvent,
) -> Result<u64, tokio_postgres::Error>
where
    C: GenericClient + Sync,
{
    let schema_version = SQL_LEDGER_SCHEMA_VERSION as i32;
    client
        .execute(
            "INSERT INTO run_finality_events \
             (event_id, run_kind, run_id, source_event_id, source_order_event_id, source, state, \
              payload, payload_hash, schema_version, occurred_at_ms, captured_at_ms) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12) \
             ON CONFLICT (event_id) DO NOTHING",
            &[
                &event.event_id,
                &event.run_kind,
                &event.run_id,
                &event.source_event_id,
                &event.source_order_event_id,
                &event.source,
                &event.state,
                &event.payload,
                &event.payload_hash,
                &schema_version,
                &event.occurred_at_ms,
                &event.captured_at_ms,
            ],
        )
        .await
}

async fn read_existing<C>(
    client: &C,
    event_id: &str,
) -> Result<SqlRunFinalityLedgerEvent, writer::EventWriteError>
where
    C: GenericClient + Sync,
{
    let row = client
        .query_one(
            "SELECT event_id, run_kind, run_id, source_event_id, source_order_event_id, source, \
                    state, payload, payload_hash, occurred_at_ms, captured_at_ms \
             FROM run_finality_events WHERE event_id = $1",
            &[&event_id],
        )
        .await?;
    Ok(SqlRunFinalityLedgerEvent {
        event_id: row.try_get("event_id")?,
        run_kind: row.try_get("run_kind")?,
        run_id: row.try_get("run_id")?,
        source_event_id: row.try_get("source_event_id")?,
        source_order_event_id: row.try_get("source_order_event_id")?,
        source: row.try_get("source")?,
        state: row.try_get("state")?,
        payload: row.try_get("payload")?,
        payload_hash: row.try_get("payload_hash")?,
        occurred_at_ms: row.try_get("occurred_at_ms")?,
        captured_at_ms: row.try_get("captured_at_ms")?,
    })
}

fn classify_write(
    inserted: bool,
    existing: &SqlRunFinalityLedgerEvent,
    requested: &SqlRunFinalityLedgerEvent,
) -> Result<SqlLedgerPersistAck, writer::EventWriteError> {
    if inserted {
        return Ok(SqlLedgerPersistAck::Committed);
    }
    if existing == requested {
        Ok(SqlLedgerPersistAck::AlreadyPersisted)
    } else {
        Err(writer::EventWriteError::IntegrityConflict {
            event_id: requested.event_id.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finality_duplicate_requires_exact_payload_and_source_identity() {
        let event = fixture();
        assert_eq!(
            classify_write(false, &event, &event).expect("matching duplicate"),
            SqlLedgerPersistAck::AlreadyPersisted
        );
        let mut changed = event.clone();
        changed.source_event_id = Some("other-event".to_owned());
        assert!(matches!(
            classify_write(false, &event, &changed),
            Err(writer::EventWriteError::IntegrityConflict { .. })
        ));
    }

    fn fixture() -> SqlRunFinalityLedgerEvent {
        SqlRunFinalityLedgerEvent {
            event_id: "finality-1".to_owned(),
            run_kind: "close_run".to_owned(),
            run_id: "close-1".to_owned(),
            source_event_id: None,
            source_order_event_id: None,
            source: "manual".to_owned(),
            state: "manually_resolved".to_owned(),
            payload: serde_json::json!({"id": "close-1"}),
            payload_hash: "fnv1a64:0000000000000001".to_owned(),
            occurred_at_ms: 1,
            captured_at_ms: 2,
        }
    }
}
