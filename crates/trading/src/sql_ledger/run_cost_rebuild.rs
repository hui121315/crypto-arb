use super::{
    json_hash, run_cost, writer, SqlLedgerWriteError, SqlRunFinalityLedgerEvent, RUN_COST_PROJECTOR,
};
use shared_types::{CloseRun, ExecutionLedgerEvent};

const REBUILD_ADVISORY_LOCK_KEY: i64 = 0x4352_4f53_5446_4143;
const MAX_PAGE_SIZE: usize = 2_048;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SqlRunCostRebuildReport {
    pub order_high_water_mark: i64,
    pub finality_high_water_mark: i64,
    pub order_rows_scanned: usize,
    pub finality_rows_scanned: usize,
    pub facts_written: usize,
    pub legacy_links_written: usize,
    pub complete: bool,
}

#[derive(Debug, Clone, Copy)]
struct RebuildCursor {
    order_cursor: i64,
    finality_cursor: i64,
    order_high_water: i64,
    finality_high_water: i64,
}

pub(super) async fn rebuild(
    url: &str,
    page_size: usize,
) -> Result<SqlRunCostRebuildReport, SqlLedgerWriteError> {
    let page_size = validate_page_size(page_size)?;
    let (mut client, connection) = tokio_postgres::connect(url, tokio_postgres::NoTls)
        .await
        .map_err(SqlLedgerWriteError::from)?;
    tokio::spawn(async move {
        if let Err(error) = connection.await {
            tracing::warn!(%error, "run-cost rebuild postgres connection failed");
        }
    });
    client
        .query_one("SELECT pg_advisory_lock($1)", &[&REBUILD_ADVISORY_LOCK_KEY])
        .await
        .map_err(SqlLedgerWriteError::from)?;
    let result = rebuild_locked(&mut client, page_size).await;
    if let Err(error) = client
        .query_one(
            "SELECT pg_advisory_unlock($1)",
            &[&REBUILD_ADVISORY_LOCK_KEY],
        )
        .await
    {
        tracing::warn!(%error, "run-cost rebuild advisory unlock failed");
    }
    result
}

async fn rebuild_locked(
    client: &mut tokio_postgres::Client,
    page_size: i64,
) -> Result<SqlRunCostRebuildReport, SqlLedgerWriteError> {
    let cursor = initialize_cursor(client).await?;
    let mut report = SqlRunCostRebuildReport {
        order_high_water_mark: cursor.order_high_water,
        finality_high_water_mark: cursor.finality_high_water,
        ..SqlRunCostRebuildReport::default()
    };
    let mut order_cursor = cursor.order_cursor;
    while order_cursor < cursor.order_high_water {
        let page =
            rebuild_order_page(client, order_cursor, cursor.order_high_water, page_size).await?;
        order_cursor = page.cursor;
        report.order_rows_scanned = report.order_rows_scanned.saturating_add(page.rows);
        report.facts_written = report.facts_written.saturating_add(page.facts_written);
    }
    let mut finality_cursor = cursor.finality_cursor;
    while finality_cursor < cursor.finality_high_water {
        let page = rebuild_finality_page(
            client,
            finality_cursor,
            cursor.finality_high_water,
            page_size,
        )
        .await?;
        finality_cursor = page.cursor;
        report.finality_rows_scanned = report.finality_rows_scanned.saturating_add(page.rows);
        report.facts_written = report.facts_written.saturating_add(page.facts_written);
        report.legacy_links_written = report
            .legacy_links_written
            .saturating_add(page.legacy_links_written);
    }
    report.complete =
        order_cursor >= cursor.order_high_water && finality_cursor >= cursor.finality_high_water;
    Ok(report)
}

async fn initialize_cursor(
    client: &mut tokio_postgres::Client,
) -> Result<RebuildCursor, SqlLedgerWriteError> {
    let transaction = client
        .transaction()
        .await
        .map_err(SqlLedgerWriteError::from)?;
    let high_water = transaction
        .query_one(
            "SELECT COALESCE((SELECT MAX(id) FROM order_events), 0) AS order_high_water, \
                    COALESCE((SELECT MAX(id) FROM run_finality_events), 0) AS finality_high_water, \
                    (SELECT COUNT(*) FROM run_cost_facts) AS fact_count, \
                    (SELECT COUNT(*) FROM run_finality_source_links) AS link_count",
            &[],
        )
        .await
        .map_err(SqlLedgerWriteError::from)?;
    let order_high_water: i64 = high_water.get("order_high_water");
    let finality_high_water: i64 = high_water.get("finality_high_water");
    let fact_count: i64 = high_water.get("fact_count");
    let link_count: i64 = high_water.get("link_count");
    let row = transaction
        .query_one(
            "INSERT INTO run_cost_rebuild_receipts \
             (projector, order_cursor, finality_cursor, order_high_water, finality_high_water, \
              facts_written, legacy_links_written, updated_at_ms) \
             VALUES ($1, 0, 0, $2, $3, 0, 0, $4) \
             ON CONFLICT (projector) DO UPDATE SET \
                 order_cursor = CASE WHEN $5::BIGINT = 0 THEN 0 ELSE run_cost_rebuild_receipts.order_cursor END, \
                 finality_cursor = CASE WHEN $6::BIGINT = 0 THEN 0 ELSE run_cost_rebuild_receipts.finality_cursor END, \
                 order_high_water = GREATEST(run_cost_rebuild_receipts.order_high_water, $2), \
                 finality_high_water = GREATEST(run_cost_rebuild_receipts.finality_high_water, $3), \
                 updated_at_ms = $4 \
             RETURNING order_cursor, finality_cursor, order_high_water, finality_high_water",
            &[
                &RUN_COST_PROJECTOR,
                &order_high_water,
                &finality_high_water,
                &common::time::now_ms(),
                &fact_count,
                &link_count,
            ],
        )
        .await
        .map_err(SqlLedgerWriteError::from)?;
    let cursor = RebuildCursor {
        order_cursor: row.get("order_cursor"),
        finality_cursor: row.get("finality_cursor"),
        order_high_water: row.get("order_high_water"),
        finality_high_water: row.get("finality_high_water"),
    };
    transaction
        .commit()
        .await
        .map_err(SqlLedgerWriteError::from)?;
    Ok(cursor)
}

#[derive(Debug, Clone, Copy, Default)]
struct PageResult {
    cursor: i64,
    rows: usize,
    facts_written: usize,
    legacy_links_written: usize,
}

async fn rebuild_order_page(
    client: &mut tokio_postgres::Client,
    cursor: i64,
    high_water: i64,
    page_size: i64,
) -> Result<PageResult, SqlLedgerWriteError> {
    let transaction = client
        .transaction()
        .await
        .map_err(SqlLedgerWriteError::from)?;
    let rows = transaction
        .query(
            "SELECT id, event_id, payload, payload::text AS payload_text, payload_hash FROM order_events \
             WHERE id > $1 AND id <= $2 ORDER BY id LIMIT $3",
            &[&cursor, &high_water, &page_size],
        )
        .await
        .map_err(SqlLedgerWriteError::from)?;
    let mut result = PageResult {
        cursor: high_water,
        rows: rows.len(),
        ..PageResult::default()
    };
    for row in &rows {
        result.cursor = row.get("id");
        let event = decode_order_event(row)?;
        result.facts_written = result.facts_written.saturating_add(
            run_cost::upsert_order_event_facts(&transaction, &event)
                .await
                .map_err(writer::EventWriteError::into_public)?,
        );
    }
    update_receipt(&transaction, result.cursor, None, result.facts_written, 0).await?;
    transaction
        .commit()
        .await
        .map_err(SqlLedgerWriteError::from)?;
    Ok(result)
}

async fn rebuild_finality_page(
    client: &mut tokio_postgres::Client,
    cursor: i64,
    high_water: i64,
    page_size: i64,
) -> Result<PageResult, SqlLedgerWriteError> {
    let transaction = client
        .transaction()
        .await
        .map_err(SqlLedgerWriteError::from)?;
    let rows = transaction
        .query(
            "SELECT id, event_id, run_kind, run_id, source_event_id, source_order_event_id, \
                    source, state, payload, payload::text AS payload_text, payload_hash, \
                    occurred_at_ms, captured_at_ms \
             FROM run_finality_events WHERE id > $1 AND id <= $2 ORDER BY id LIMIT $3",
            &[&cursor, &high_water, &page_size],
        )
        .await
        .map_err(SqlLedgerWriteError::from)?;
    let mut result = PageResult {
        cursor: high_water,
        rows: rows.len(),
        ..PageResult::default()
    };
    for row in &rows {
        result.cursor = row.get("id");
        let event = decode_finality_event(row)?;
        result.facts_written = result.facts_written.saturating_add(
            run_cost::upsert_finality_facts(&transaction, &event)
                .await
                .map_err(writer::EventWriteError::into_public)?,
        );
        result.legacy_links_written = result
            .legacy_links_written
            .saturating_add(upsert_source_link(&transaction, &event).await?);
    }
    update_receipt(
        &transaction,
        cursor,
        Some(result.cursor),
        result.facts_written,
        result.legacy_links_written,
    )
    .await?;
    transaction
        .commit()
        .await
        .map_err(SqlLedgerWriteError::from)?;
    Ok(result)
}

async fn update_receipt<C>(
    client: &C,
    order_cursor: i64,
    finality_cursor: Option<i64>,
    facts_written: usize,
    legacy_links_written: usize,
) -> Result<(), SqlLedgerWriteError>
where
    C: tokio_postgres::GenericClient + Sync,
{
    let facts_written = i64::try_from(facts_written)
        .map_err(|_| SqlLedgerWriteError::Encoding("fact count exceeds BIGINT".to_owned()))?;
    let legacy_links_written = i64::try_from(legacy_links_written)
        .map_err(|_| SqlLedgerWriteError::Encoding("link count exceeds BIGINT".to_owned()))?;
    client
        .execute(
            "UPDATE run_cost_rebuild_receipts SET \
                 order_cursor = CASE WHEN $2::BIGINT IS NULL THEN $1 ELSE order_cursor END, \
                 finality_cursor = COALESCE($2::BIGINT, finality_cursor), \
                 facts_written = facts_written + $3, \
                 legacy_links_written = legacy_links_written + $4, \
                 updated_at_ms = $5 \
             WHERE projector = $6",
            &[
                &order_cursor,
                &finality_cursor,
                &facts_written,
                &legacy_links_written,
                &common::time::now_ms(),
                &RUN_COST_PROJECTOR,
            ],
        )
        .await
        .map_err(SqlLedgerWriteError::from)?;
    Ok(())
}

fn decode_order_event(
    row: &tokio_postgres::Row,
) -> Result<ExecutionLedgerEvent, SqlLedgerWriteError> {
    let event_id: String = row.try_get("event_id").map_err(SqlLedgerWriteError::from)?;
    let payload: serde_json::Value = row.try_get("payload").map_err(SqlLedgerWriteError::from)?;
    let payload_text: String = row
        .try_get("payload_text")
        .map_err(SqlLedgerWriteError::from)?;
    let payload_hash: String = row
        .try_get("payload_hash")
        .map_err(SqlLedgerWriteError::from)?;
    let payload = normalize_rebuild_payload(&event_id, &payload, &payload_text, &payload_hash)?;
    let event = serde_json::from_value::<ExecutionLedgerEvent>(payload)
        .map_err(|error| SqlLedgerWriteError::Encoding(error.to_string()))?;
    if event.event_id != event_id {
        return Err(SqlLedgerWriteError::IntegrityConflict { event_id });
    }
    Ok(event)
}

fn decode_finality_event(
    row: &tokio_postgres::Row,
) -> Result<SqlRunFinalityLedgerEvent, SqlLedgerWriteError> {
    let event_id: String = row.try_get("event_id").map_err(SqlLedgerWriteError::from)?;
    let payload: serde_json::Value = row.try_get("payload").map_err(SqlLedgerWriteError::from)?;
    let payload_text: String = row
        .try_get("payload_text")
        .map_err(SqlLedgerWriteError::from)?;
    let payload_hash: String = row
        .try_get("payload_hash")
        .map_err(SqlLedgerWriteError::from)?;
    let payload = normalize_rebuild_payload(&event_id, &payload, &payload_text, &payload_hash)?;
    Ok(SqlRunFinalityLedgerEvent {
        event_id,
        run_kind: row.try_get("run_kind").map_err(SqlLedgerWriteError::from)?,
        run_id: row.try_get("run_id").map_err(SqlLedgerWriteError::from)?,
        source_event_id: row
            .try_get("source_event_id")
            .map_err(SqlLedgerWriteError::from)?,
        source_order_event_id: row
            .try_get("source_order_event_id")
            .map_err(SqlLedgerWriteError::from)?,
        source: row.try_get("source").map_err(SqlLedgerWriteError::from)?,
        state: row.try_get("state").map_err(SqlLedgerWriteError::from)?,
        payload,
        payload_hash,
        occurred_at_ms: row
            .try_get("occurred_at_ms")
            .map_err(SqlLedgerWriteError::from)?,
        captured_at_ms: row
            .try_get("captured_at_ms")
            .map_err(SqlLedgerWriteError::from)?,
    })
}

fn normalize_rebuild_payload(
    event_id: &str,
    payload: &serde_json::Value,
    payload_text: &str,
    payload_hash: &str,
) -> Result<serde_json::Value, SqlLedgerWriteError> {
    if json_hash(payload).map_err(SqlLedgerWriteError::Encoding)? == payload_hash {
        return Ok(payload.clone());
    }
    let normalized =
        super::parse_json_with_exact_floats(payload_text).map_err(SqlLedgerWriteError::Encoding)?;
    if json_hash(&normalized).map_err(SqlLedgerWriteError::Encoding)? == payload_hash {
        Ok(normalized)
    } else {
        Err(SqlLedgerWriteError::IntegrityConflict {
            event_id: event_id.to_owned(),
        })
    }
}

struct SourceLink {
    status: &'static str,
    source_event_id: Option<String>,
    source_order_event_id: Option<String>,
    candidate_count: i32,
}

async fn upsert_source_link<C>(
    client: &C,
    event: &SqlRunFinalityLedgerEvent,
) -> Result<usize, SqlLedgerWriteError>
where
    C: tokio_postgres::GenericClient + Sync,
{
    let link = resolve_source_link(client, event).await?;
    let updated = client
        .execute(
            "INSERT INTO run_finality_source_links \
             (run_finality_event_id, status, source_event_id, source_order_event_id, \
              candidate_count, linked_at_ms) \
             VALUES ($1, $2, $3, $4, $5, $6) \
             ON CONFLICT (run_finality_event_id) DO UPDATE SET \
                 status = EXCLUDED.status, source_event_id = EXCLUDED.source_event_id, \
                 source_order_event_id = EXCLUDED.source_order_event_id, \
                 candidate_count = EXCLUDED.candidate_count, linked_at_ms = EXCLUDED.linked_at_ms \
             WHERE run_finality_source_links.status IS DISTINCT FROM EXCLUDED.status \
                OR run_finality_source_links.source_event_id IS DISTINCT FROM EXCLUDED.source_event_id \
                OR run_finality_source_links.source_order_event_id IS DISTINCT FROM EXCLUDED.source_order_event_id \
                OR run_finality_source_links.candidate_count IS DISTINCT FROM EXCLUDED.candidate_count",
            &[
                &event.event_id,
                &link.status,
                &link.source_event_id,
                &link.source_order_event_id,
                &link.candidate_count,
                &common::time::now_ms(),
            ],
        )
        .await
        .map_err(SqlLedgerWriteError::from)?;
    Ok(updated as usize)
}

async fn resolve_source_link<C>(
    client: &C,
    event: &SqlRunFinalityLedgerEvent,
) -> Result<SourceLink, SqlLedgerWriteError>
where
    C: tokio_postgres::GenericClient + Sync,
{
    if event.source_event_id.is_some() || event.source_order_event_id.is_some() {
        return Ok(SourceLink {
            status: "intrinsic",
            source_event_id: event.source_event_id.clone(),
            source_order_event_id: event.source_order_event_id.clone(),
            candidate_count: 1,
        });
    }
    let candidates = legacy_candidates(client, event).await?;
    match candidates.as_slice() {
        [(event_id, order_event_id)] => Ok(SourceLink {
            status: "unique",
            source_event_id: Some(event_id.clone()),
            source_order_event_id: Some(order_event_id.clone()),
            candidate_count: 1,
        }),
        [] if has_legacy_identity(event) => Ok(SourceLink {
            status: "missing",
            source_event_id: None,
            source_order_event_id: None,
            candidate_count: 0,
        }),
        [] => Ok(SourceLink {
            status: "unlinked",
            source_event_id: None,
            source_order_event_id: None,
            candidate_count: 0,
        }),
        many => Ok(SourceLink {
            status: "ambiguous",
            source_event_id: None,
            source_order_event_id: None,
            candidate_count: i32::try_from(many.len()).unwrap_or(i32::MAX),
        }),
    }
}

async fn legacy_candidates<C>(
    client: &C,
    event: &SqlRunFinalityLedgerEvent,
) -> Result<Vec<(String, String)>, SqlLedgerWriteError>
where
    C: tokio_postgres::GenericClient + Sync,
{
    let rows = if event.run_kind == "execution_run" {
        client
            .query(
                "SELECT event_id, internal_order_id FROM order_events \
                 WHERE run_id = $1 AND occurred_at_ms = $2 ORDER BY event_id LIMIT 3",
                &[&event.run_id, &event.occurred_at_ms],
            )
            .await
            .map_err(SqlLedgerWriteError::from)?
    } else {
        let order_ids = close_run_order_ids(event);
        if order_ids.is_empty() {
            return Ok(Vec::new());
        }
        client
            .query(
                "SELECT event_id, internal_order_id FROM order_events \
                 WHERE internal_order_id = ANY($1) AND occurred_at_ms = $2 \
                 ORDER BY event_id LIMIT 3",
                &[&order_ids, &event.occurred_at_ms],
            )
            .await
            .map_err(SqlLedgerWriteError::from)?
    };
    rows.iter()
        .map(|row| Ok((row.try_get("event_id")?, row.try_get("internal_order_id")?)))
        .collect::<Result<Vec<_>, tokio_postgres::Error>>()
        .map_err(SqlLedgerWriteError::from)
}

fn close_run_order_ids(event: &SqlRunFinalityLedgerEvent) -> Vec<String> {
    serde_json::from_value::<CloseRun>(event.payload.clone())
        .map(|run| {
            let mut ids = run
                .legs
                .into_iter()
                .filter_map(|leg| leg.order.map(|order| order.intent.id))
                .collect::<Vec<_>>();
            if let Some(plan) = run.unwind_plan {
                ids.extend(
                    plan.compensation_attempts
                        .into_iter()
                        .filter_map(|attempt| attempt.order.map(|order| order.intent.id)),
                );
            }
            ids.sort();
            ids.dedup();
            ids
        })
        .unwrap_or_default()
}

fn has_legacy_identity(event: &SqlRunFinalityLedgerEvent) -> bool {
    event.run_kind == "execution_run" || !close_run_order_ids(event).is_empty()
}

fn validate_page_size(page_size: usize) -> Result<i64, SqlLedgerWriteError> {
    if !(1..=MAX_PAGE_SIZE).contains(&page_size) {
        return Err(SqlLedgerWriteError::InvalidProjectionJobRequest(format!(
            "run-cost rebuild page_size must be between 1 and {MAX_PAGE_SIZE}"
        )));
    }
    i64::try_from(page_size)
        .map_err(|_| SqlLedgerWriteError::Encoding("page_size exceeds BIGINT".to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rebuild_hash_accepts_postgres_expanded_numeric_text() {
        let canonical = serde_json::json!({"value": -1.136_868_377_216_160_3e-13});
        let payload_hash = json_hash(&canonical).expect("canonical hash");
        let payload_text = r#"{"value":-0.00000000000011368683772161603}"#;
        let postgres_decoded: serde_json::Value =
            serde_json::from_str(payload_text).expect("postgres payload");

        assert_ne!(
            json_hash(&postgres_decoded).expect("postgres-decoded hash"),
            payload_hash
        );
        assert_eq!(
            normalize_rebuild_payload(
                "orderbook-evidence-1",
                &postgres_decoded,
                payload_text,
                &payload_hash,
            ),
            Ok(canonical)
        );
    }

    #[test]
    fn rebuild_page_size_is_bounded() {
        assert_eq!(validate_page_size(1), Ok(1));
        assert_eq!(validate_page_size(MAX_PAGE_SIZE), Ok(MAX_PAGE_SIZE as i64));
        assert!(validate_page_size(0).is_err());
        assert!(validate_page_size(MAX_PAGE_SIZE + 1).is_err());
    }

    #[test]
    fn legacy_close_without_order_identity_is_never_fabricated() {
        let event = SqlRunFinalityLedgerEvent {
            event_id: "legacy-finality".to_owned(),
            run_kind: "close_run".to_owned(),
            run_id: "close-1".to_owned(),
            source_event_id: None,
            source_order_event_id: None,
            source: "manual".to_owned(),
            state: "manually_resolved".to_owned(),
            payload: serde_json::json!({}),
            payload_hash: "fnv1a64:0".to_owned(),
            occurred_at_ms: 1,
            captured_at_ms: 1,
        };

        assert!(!has_legacy_identity(&event));
        assert!(close_run_order_ids(&event).is_empty());
    }
}
