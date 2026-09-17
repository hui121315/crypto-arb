#![allow(clippy::expect_used, clippy::panic)]

use shared_types::{
    CloseRun, CloseRunCostComponent, CloseRunCostLedgerEvent, CloseRunManualTerminalEvidence,
    CloseRunScope, CloseRunStatus, CloseRunUnwindPlan, CloseRunUnwindPlanStatus,
    ExecutionFillConfidence, ExecutionLedgerEvent, ExecutionLedgerEventType,
    ExecutionLedgerOrderRef, ExecutionLedgerPayload, ExecutionLedgerQuality, FeeLedgerSnapshot,
    FillLedgerSnapshot, FundingPaymentLedgerRecord, HedgeLegRole, OrderSide, OrderUpdateSource,
    SlippageLedgerRecord, VenueOrderIdentity,
};
use trading::sql_ledger::{
    SqlLedgerPersistAck, SqlLedgerStore, SqlLedgerWriteError, SqlProjectionJob,
    SqlProjectionJobAck, SqlRunCostFact, SqlRunCostRebuildReport, SqlRunFinalityLedgerEvent,
    EXECUTION_RUN_PROJECTOR, RUN_COST_PROJECTOR,
};

const DATABASE_URL_ENV: &str = "CROSSLINE_TEST_POSTGRES_URL";

#[tokio::test]
#[ignore = "requires CROSSLINE_TEST_POSTGRES_URL"]
async fn sql_ledger_commit_ack_rejects_conflicting_duplicate() {
    let database_url = required_database_url();
    let init = trading::init_sql_ledger_store(Some(&database_url)).await;
    assert!(
        init.migration_health.applied,
        "migration failed: {:?}",
        init.migration_health.last_error
    );
    let store = init.store.expect("configured PostgreSQL writer");
    let event = fill_event(unique_id("commit-ack"));

    assert_eq!(
        store.persist_event(&event).await,
        Ok(SqlLedgerPersistAck::Committed)
    );
    assert_eq!(
        store.persist_event(&event).await,
        Ok(SqlLedgerPersistAck::AlreadyPersisted)
    );

    let mut conflicting = event.clone();
    let ExecutionLedgerPayload::FillSnapshot(fill) = &mut conflicting.payload else {
        panic!("fill fixture payload drifted");
    };
    fill.quantity += 1.0;
    assert_eq!(
        store.persist_event(&conflicting).await,
        Err(SqlLedgerWriteError::IntegrityConflict {
            event_id: event.event_id.clone(),
        })
    );
    store.shutdown().await.expect("writer shutdown");

    let (client, connection) = tokio_postgres::connect(&database_url, tokio_postgres::NoTls)
        .await
        .expect("verification connection");
    tokio::spawn(async move {
        connection.await.expect("verification connection task");
    });
    let event_count: i64 = client
        .query_one(
            "SELECT COUNT(*) FROM order_events WHERE event_id = $1",
            &[&event.event_id],
        )
        .await
        .expect("order event count")
        .get(0);
    let jobs = client
        .query(
            "SELECT projector, status FROM ledger_projection_jobs \
             WHERE event_id = $1 ORDER BY projector",
            &[&event.event_id],
        )
        .await
        .expect("projection jobs");

    assert_eq!(event_count, 1);
    assert_eq!(jobs.len(), 3);
    assert!(jobs.iter().all(|row| row.get::<_, String>(1) == "pending"));
    assert_eq!(
        jobs.iter()
            .map(|row| row.get::<_, String>(0))
            .collect::<Vec<_>>(),
        vec![
            "close_run_v1".to_owned(),
            "execution_run_v1".to_owned(),
            "run_cost_facts_v1".to_owned(),
        ]
    );
}

#[tokio::test]
#[ignore = "requires CROSSLINE_TEST_POSTGRES_URL"]
async fn projection_jobs_are_exclusive_and_cas_guarded_across_writers() {
    let database_url = required_database_url();
    let first = initialized_store(&database_url).await;
    let second = initialized_store(&database_url).await;
    let projector = unique_id("projector-two-claimers");
    let first_event = persist_seeded_job(&database_url, &first, &projector, "claim-a").await;
    let second_event = persist_seeded_job(&database_url, &first, &projector, "claim-b").await;
    let now_ms = common::time::now_ms();

    let (first_claim, second_claim) = tokio::join!(
        first.claim_projection_jobs(&projector, now_ms, 1, 10_000),
        second.claim_projection_jobs(&projector, now_ms, 1, 10_000),
    );
    let first_job = only_job(first_claim.expect("first claim"));
    let second_job = only_job(second_claim.expect("second claim"));
    assert_ne!(first_job.event_id, second_job.event_id);
    assert_ne!(first_job.claim_token, second_job.claim_token);
    assert_eq!(first_job.attempt_count, 1);
    assert_eq!(second_job.attempt_count, 1);
    assert_eq!(
        [first_job.event_id.as_str(), second_job.event_id.as_str()]
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>(),
        [first_event.as_str(), second_event.as_str()]
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>()
    );

    let mut stale = first_job.clone();
    stale.claim_token.push_str(":stale");
    assert_eq!(
        first.complete_projection_job(&stale, now_ms + 1).await,
        Ok(SqlProjectionJobAck::StaleClaim)
    );
    assert!(first
        .claim_projection_jobs(&projector, now_ms + 9_999, 1, 10_000)
        .await
        .expect("active leases are not claimable")
        .is_empty());
    let lease_reclaimed = only_job(
        first
            .claim_projection_jobs(&projector, now_ms + 10_000, 1, 10_000)
            .await
            .expect("expired lease reclaim"),
    );
    let (expired_job, retry_job) = if lease_reclaimed.event_id == first_job.event_id {
        (&first_job, &second_job)
    } else {
        (&second_job, &first_job)
    };
    assert_ne!(lease_reclaimed.claim_token, expired_job.claim_token);
    assert_eq!(lease_reclaimed.attempt_count, 2);
    assert_eq!(
        first
            .complete_projection_job(expired_job, now_ms + 10_001)
            .await,
        Ok(SqlProjectionJobAck::StaleClaim)
    );
    assert_eq!(
        first
            .complete_projection_job(&lease_reclaimed, now_ms + 10_002)
            .await,
        Ok(SqlProjectionJobAck::Applied)
    );
    assert_eq!(
        second
            .retry_projection_job(retry_job, now_ms + 10_100, "projection failed")
            .await,
        Ok(SqlProjectionJobAck::Applied)
    );
    assert!(second
        .claim_projection_jobs(&projector, now_ms + 10_099, 1, 10_000)
        .await
        .expect("not-yet-available claim")
        .is_empty());
    let reclaimed = only_job(
        second
            .claim_projection_jobs(&projector, now_ms + 10_100, 1, 10_000)
            .await
            .expect("retry reclaim"),
    );
    assert_eq!(reclaimed.event_id, retry_job.event_id);
    assert_eq!(reclaimed.attempt_count, 2);
    assert_ne!(reclaimed.claim_token, retry_job.claim_token);
    assert_eq!(
        first
            .complete_projection_job(retry_job, now_ms + 10_101)
            .await,
        Ok(SqlProjectionJobAck::StaleClaim)
    );
    assert_eq!(
        second
            .complete_projection_job(&reclaimed, now_ms + 10_102)
            .await,
        Ok(SqlProjectionJobAck::Applied)
    );

    first.shutdown().await.expect("first writer shutdown");
    second.shutdown().await.expect("second writer shutdown");
}

#[tokio::test]
#[ignore = "requires CROSSLINE_TEST_POSTGRES_URL"]
async fn projection_jobs_survive_restart_and_duplicate_persist_does_not_reset_them() {
    let database_url = required_database_url();
    let store = initialized_store(&database_url).await;
    let projector = unique_id("projector-restart");
    let event = fill_event(unique_id("restart-visible"));
    assert_eq!(
        store.persist_event(&event).await,
        Ok(SqlLedgerPersistAck::Committed)
    );
    seed_projection_job(&database_url, &event.event_id, &projector).await;
    let generated_claim_token = unique_id("generated-completed-token");
    mark_projection_job_completed(
        &database_url,
        &event.event_id,
        EXECUTION_RUN_PROJECTOR,
        &generated_claim_token,
    )
    .await;
    store.shutdown().await.expect("initial writer shutdown");

    let restarted = initialized_store(&database_url).await;
    let now_ms = common::time::now_ms();
    let job = only_job(
        restarted
            .claim_projection_jobs(&projector, now_ms, 1, 10_000)
            .await
            .expect("restart claim"),
    );
    assert_eq!(job.event, event);
    assert_eq!(
        restarted.complete_projection_job(&job, now_ms + 1).await,
        Ok(SqlProjectionJobAck::Applied)
    );
    assert_eq!(
        restarted.persist_event(&event).await,
        Ok(SqlLedgerPersistAck::AlreadyPersisted)
    );

    let (status, attempt_count, claim_token) =
        projection_job_state(&database_url, &event.event_id, &projector).await;
    assert_eq!(status, "completed");
    assert_eq!(attempt_count, 1);
    assert_eq!(claim_token.as_deref(), Some(job.claim_token.as_str()));
    assert_eq!(
        projection_job_state(&database_url, &event.event_id, EXECUTION_RUN_PROJECTOR).await,
        ("completed".to_owned(), 1, Some(generated_claim_token))
    );
    let run_cost_state =
        projection_job_state(&database_url, &event.event_id, RUN_COST_PROJECTOR).await;
    assert_eq!(run_cost_state, ("pending".to_owned(), 0, None));
    restarted
        .shutdown()
        .await
        .expect("restarted writer shutdown");
}

#[tokio::test]
#[ignore = "requires CROSSLINE_TEST_POSTGRES_URL"]
async fn sql_ledger_postgres_roundtrip_restarts_from_committed_facts() {
    let database_url = required_database_url();
    let execution_run_id = unique_id("roundtrip-execution-run");
    let close_run_id = unique_id("roundtrip-close-run");
    let events = run_cost_source_events(&execution_run_id);
    let event_ids = events
        .iter()
        .map(|event| event.event_id.clone())
        .collect::<Vec<_>>();
    let store = initialized_store(&database_url).await;

    let acks = store
        .persist_event_group(&events)
        .await
        .expect("committed run-cost order sources");
    assert!(acks
        .iter()
        .all(|ack| *ack == SqlLedgerPersistAck::Committed));
    for event in &events {
        store
            .project_run_cost_event(event)
            .await
            .expect("project committed order source");
    }
    let finality = manual_finality_event(&close_run_id);
    assert_eq!(
        store.persist_run_finality_event(&finality).await,
        Ok(SqlLedgerPersistAck::Committed)
    );

    let execution_facts: Vec<SqlRunCostFact> = store
        .query_run_cost_facts("execution_run", &execution_run_id)
        .await
        .expect("query execution run-cost facts");
    let close_facts: Vec<SqlRunCostFact> = store
        .query_run_cost_facts("close_run", &close_run_id)
        .await
        .expect("query close run-cost facts");
    assert_eq!(execution_facts.len(), 3);
    assert_eq!(close_facts.len(), 1);
    let facts_before = stored_run_cost_facts(&database_url, &execution_run_id, &close_run_id).await;
    let sources_before =
        immutable_source_rows(&database_url, &event_ids, finality.event_id()).await;
    let normalized_before = stored_normalized_cost_sources(&database_url, &event_ids).await;
    assert_normalized_cost_sources(&normalized_before, &event_ids);
    store.shutdown().await.expect("initial writer shutdown");

    clear_rebuildable_run_cost_state(&database_url, &execution_run_id, &close_run_id).await;
    let restarted = initialized_store(&database_url).await;
    let report: SqlRunCostRebuildReport = restarted
        .rebuild_run_cost_facts(2)
        .await
        .expect("rebuild committed run-cost facts");
    let facts_after = stored_run_cost_facts(&database_url, &execution_run_id, &close_run_id).await;
    let sources_after = immutable_source_rows(&database_url, &event_ids, finality.event_id()).await;
    let normalized_after = stored_normalized_cost_sources(&database_url, &event_ids).await;

    assert_eq!(facts_after, facts_before);
    assert_eq!(sources_after, sources_before);
    assert_eq!(normalized_after, normalized_before);
    assert!(report.complete);
    assert_run_cost_rebuild_receipt_completed(&database_url).await;
    assert_finality_source_link(&database_url, finality.event_id()).await;
    restarted
        .shutdown()
        .await
        .expect("restarted writer shutdown");
}

#[derive(Debug, PartialEq)]
struct StoredRunCostFact {
    run_kind: String,
    run_id: String,
    scope: String,
    component: String,
    event_id: String,
    source_order_event_id: Option<String>,
    source_run_finality_event_id: Option<String>,
    amount: f64,
    amount_usd: Option<f64>,
    quality: String,
    payload_hash: String,
}

#[derive(Debug, PartialEq)]
struct StoredNormalizedCostSources {
    fill: (String, f64, f64, f64, String, f64),
    fee: (String, String, f64, Option<String>, String),
    slippage: (String, f64, f64, f64, f64, String),
    funding: (String, f64, String, i64, String),
}

async fn stored_normalized_cost_sources(
    database_url: &str,
    event_ids: &[String],
) -> StoredNormalizedCostSources {
    assert_eq!(event_ids.len(), 3);
    let client = postgres_client(database_url).await;
    let fill = client
        .query_one(
            "SELECT event_id, quantity, average_price, quote_value, fill_confidence, \
                    fill_confidence_score FROM fills WHERE event_id = $1",
            &[&event_ids[0]],
        )
        .await
        .expect("normalized fill source");
    let fee = client
        .query_one(
            "SELECT event_id, fee_origin, amount, currency, quality \
             FROM fees WHERE event_id = $1",
            &[&event_ids[0]],
        )
        .await
        .expect("normalized fee source");
    let slippage = client
        .query_one(
            "SELECT event_id, amount_usd, reference_price, fill_price, quantity, quality \
             FROM slippage_events WHERE event_id = $1",
            &[&event_ids[1]],
        )
        .await
        .expect("normalized slippage source");
    let funding = client
        .query_one(
            "SELECT event_id, amount, currency, funding_time_ms, quality \
             FROM funding_payments WHERE event_id = $1",
            &[&event_ids[2]],
        )
        .await
        .expect("normalized funding source");

    StoredNormalizedCostSources {
        fill: (
            fill.get(0),
            fill.get(1),
            fill.get(2),
            fill.get(3),
            fill.get(4),
            fill.get(5),
        ),
        fee: (fee.get(0), fee.get(1), fee.get(2), fee.get(3), fee.get(4)),
        slippage: (
            slippage.get(0),
            slippage.get(1),
            slippage.get(2),
            slippage.get(3),
            slippage.get(4),
            slippage.get(5),
        ),
        funding: (
            funding.get(0),
            funding.get(1),
            funding.get(2),
            funding.get(3),
            funding.get(4),
        ),
    }
}

fn assert_normalized_cost_sources(rows: &StoredNormalizedCostSources, event_ids: &[String]) {
    assert_eq!(rows.fill.0, event_ids[0]);
    assert_eq!((rows.fill.1, rows.fill.2, rows.fill.3), (2.0, 101.5, 203.0));
    assert_eq!((rows.fill.4.as_str(), rows.fill.5), ("venue_fill", 1.0));
    assert_eq!(
        rows.fee,
        (
            event_ids[0].clone(),
            "embedded_fill".to_owned(),
            -0.12,
            Some("USDC".to_owned()),
            "actual".to_owned(),
        )
    );
    assert_eq!(
        rows.slippage,
        (
            event_ids[1].clone(),
            1.25,
            100.0,
            101.25,
            1.0,
            "actual".to_owned(),
        )
    );
    assert_eq!(
        rows.funding,
        (
            event_ids[2].clone(),
            -0.45,
            "USDC".to_owned(),
            3_000,
            "actual".to_owned(),
        )
    );
}

async fn stored_run_cost_facts(
    database_url: &str,
    execution_run_id: &str,
    close_run_id: &str,
) -> Vec<StoredRunCostFact> {
    postgres_client(database_url)
        .await
        .query(
            "SELECT run_kind, run_id, scope, component, event_id, source_order_event_id, \
                    source_run_finality_event_id, amount, amount_usd, quality, payload_hash \
             FROM run_cost_facts \
             WHERE (run_kind = 'execution_run' AND run_id = $1) \
                OR (run_kind = 'close_run' AND run_id = $2) \
             ORDER BY run_kind, run_id, scope, component, event_id",
            &[&execution_run_id, &close_run_id],
        )
        .await
        .expect("stored run-cost facts")
        .iter()
        .map(|row| StoredRunCostFact {
            run_kind: row.get("run_kind"),
            run_id: row.get("run_id"),
            scope: row.get("scope"),
            component: row.get("component"),
            event_id: row.get("event_id"),
            source_order_event_id: row.get("source_order_event_id"),
            source_run_finality_event_id: row.get("source_run_finality_event_id"),
            amount: row.get("amount"),
            amount_usd: row.get("amount_usd"),
            quality: row.get("quality"),
            payload_hash: row.get("payload_hash"),
        })
        .collect()
}

async fn immutable_source_rows(
    database_url: &str,
    event_ids: &[String],
    finality_event_id: &str,
) -> (
    Vec<(String, String)>,
    (String, String, Option<String>, Option<String>),
) {
    let client = postgres_client(database_url).await;
    let event_ids = event_ids.to_vec();
    let order_rows = client
        .query(
            "SELECT event_id, payload_hash FROM order_events \
             WHERE event_id = ANY($1) ORDER BY event_id",
            &[&event_ids],
        )
        .await
        .expect("immutable order source rows")
        .iter()
        .map(|row| (row.get("event_id"), row.get("payload_hash")))
        .collect();
    let finality = client
        .query_one(
            "SELECT event_id, payload_hash, source_event_id, source_order_event_id \
             FROM run_finality_events WHERE event_id = $1",
            &[&finality_event_id],
        )
        .await
        .expect("immutable finality source row");
    (
        order_rows,
        (
            finality.get("event_id"),
            finality.get("payload_hash"),
            finality.get("source_event_id"),
            finality.get("source_order_event_id"),
        ),
    )
}

async fn clear_rebuildable_run_cost_state(
    database_url: &str,
    execution_run_id: &str,
    close_run_id: &str,
) {
    let client = postgres_client(database_url).await;
    let deleted = client
        .execute(
            "DELETE FROM run_cost_facts \
             WHERE (run_kind = 'execution_run' AND run_id = $1) \
                OR (run_kind = 'close_run' AND run_id = $2)",
            &[&execution_run_id, &close_run_id],
        )
        .await
        .expect("delete rebuildable run-cost facts");
    assert_eq!(deleted, 4);
    client
        .execute(
            "DELETE FROM run_cost_rebuild_receipts WHERE projector = $1",
            &[&RUN_COST_PROJECTOR],
        )
        .await
        .expect("reset run-cost rebuild receipt");
}

async fn assert_run_cost_rebuild_receipt_completed(database_url: &str) {
    let row = postgres_client(database_url)
        .await
        .query_one(
            "SELECT order_cursor, order_high_water, finality_cursor, finality_high_water \
             FROM run_cost_rebuild_receipts WHERE projector = $1",
            &[&RUN_COST_PROJECTOR],
        )
        .await
        .expect("completed run-cost rebuild receipt");
    assert_eq!(row.get::<_, i64>(0), row.get::<_, i64>(1));
    assert_eq!(row.get::<_, i64>(2), row.get::<_, i64>(3));
}

async fn assert_finality_source_link(database_url: &str, finality_event_id: &str) {
    let row = postgres_client(database_url)
        .await
        .query_one(
            "SELECT status, source_event_id, source_order_event_id, candidate_count \
             FROM run_finality_source_links WHERE run_finality_event_id = $1",
            &[&finality_event_id],
        )
        .await
        .expect("legacy finality source link");
    assert_eq!(row.get::<_, String>(0), "unlinked");
    assert_eq!(row.get::<_, Option<String>>(1), None);
    assert_eq!(row.get::<_, Option<String>>(2), None);
    assert_eq!(row.get::<_, i32>(3), 0);
}

fn run_cost_source_events(run_id: &str) -> Vec<ExecutionLedgerEvent> {
    let mut fee = fill_event(unique_id("roundtrip-fee"));
    fee.order.run_id = Some(run_id.to_owned());
    fee.order.reduce_only = Some(false);

    let mut slippage = fill_event(unique_id("roundtrip-slippage"));
    slippage.event_type = ExecutionLedgerEventType::Slippage;
    slippage.order.run_id = Some(run_id.to_owned());
    slippage.order.reduce_only = Some(false);
    slippage.payload = ExecutionLedgerPayload::Slippage(SlippageLedgerRecord {
        amount_usd: 1.25,
        reference_price: 100.0,
        fill_price: 101.25,
        quantity: 1.0,
        quality: ExecutionLedgerQuality::Actual,
    });
    slippage.occurred_at_ms = 2_000;
    slippage.captured_at_ms = 2_001;

    let mut funding = fill_event(unique_id("roundtrip-funding"));
    funding.event_type = ExecutionLedgerEventType::FundingPayment;
    funding.order.run_id = Some(run_id.to_owned());
    funding.order.reduce_only = Some(false);
    funding.payload = ExecutionLedgerPayload::FundingPayment(FundingPaymentLedgerRecord {
        amount: -0.45,
        currency: "USDC".to_owned(),
        funding_time_ms: 3_000,
        quality: ExecutionLedgerQuality::Actual,
    });
    funding.occurred_at_ms = 3_000;
    funding.captured_at_ms = 3_001;
    vec![fee, slippage, funding]
}

fn manual_finality_event(close_run_id: &str) -> SqlRunFinalityLedgerEvent {
    let occurred_at_ms = 4_000;
    let snapshot_version = unique_id("roundtrip-snapshot");
    let manual_cost_event_id = unique_id("roundtrip-manual-cost");
    let run = CloseRun {
        id: close_run_id.to_owned(),
        scope: CloseRunScope::Single,
        status: CloseRunStatus::ManuallyResolved,
        action_run_id: None,
        request_id: None,
        idempotency_key: None,
        snapshot_version: snapshot_version.clone(),
        expected_leg_count: 0,
        reason: Some("PostgreSQL roundtrip acceptance".to_owned()),
        legs: Vec::new(),
        submitted_order_count: 0,
        failed_leg_count: 0,
        naked_exposure_usd: 0.0,
        message: "manually resolved".to_owned(),
        problem: None,
        finality_problem: None,
        finality_checked_at_ms: Some(occurred_at_ms),
        unwind_plan: Some(CloseRunUnwindPlan {
            status: CloseRunUnwindPlanStatus::ManualTerminalRecorded,
            filled_legs: Vec::new(),
            failed_legs: Vec::new(),
            compensation_candidates: Vec::new(),
            remaining_positions: Vec::new(),
            compensation_attempts: Vec::new(),
            manual_terminal_evidence: Some(CloseRunManualTerminalEvidence {
                action_run_id: None,
                actor: "postgres-test".to_owned(),
                reason: "PostgreSQL roundtrip acceptance".to_owned(),
                snapshot_version,
                recorded_at_ms: occurred_at_ms,
                remaining_positions: Vec::new(),
                required_evidence: Vec::new(),
                evidence: vec!["postgres-roundtrip".to_owned()],
                manual_handling_cost_usd: Some(2.75),
                manual_handling_event_id: Some(manual_cost_event_id.clone()),
            }),
            next_actions: Vec::new(),
            required_evidence: Vec::new(),
        }),
        cost_events: vec![CloseRunCostLedgerEvent {
            event_id: manual_cost_event_id,
            component: CloseRunCostComponent::ManualHandling,
            amount_usd: 2.75,
            source: OrderUpdateSource::Manual,
            quality: ExecutionLedgerQuality::Actual,
            occurred_at_ms,
            captured_at_ms: occurred_at_ms + 1,
        }],
        cost_reconciliation: None,
        started_at_ms: occurred_at_ms - 1,
        updated_at_ms: occurred_at_ms,
    };
    SqlRunFinalityLedgerEvent::from_close_run(
        &run,
        OrderUpdateSource::Manual,
        None,
        None,
        occurred_at_ms,
    )
    .expect("manual finality event")
}

async fn initialized_store(database_url: &str) -> SqlLedgerStore {
    let init = trading::init_sql_ledger_store(Some(database_url)).await;
    assert!(
        init.migration_health.applied,
        "migration failed: {:?}",
        init.migration_health.last_error
    );
    init.store.expect("configured PostgreSQL writer")
}

async fn persist_seeded_job(
    database_url: &str,
    store: &SqlLedgerStore,
    projector: &str,
    event_prefix: &str,
) -> String {
    let event = fill_event(unique_id(event_prefix));
    assert_eq!(
        store.persist_event(&event).await,
        Ok(SqlLedgerPersistAck::Committed)
    );
    seed_projection_job(database_url, &event.event_id, projector).await;
    event.event_id
}

async fn seed_projection_job(database_url: &str, event_id: &str, projector: &str) {
    let client = postgres_client(database_url).await;
    let now_ms = common::time::now_ms();
    client
        .execute(
            "INSERT INTO ledger_projection_jobs \
             (event_id, projector, payload_hash, available_at_ms, created_at_ms, updated_at_ms) \
             SELECT event_id, $2, payload_hash, $3, $3, $3 FROM order_events WHERE event_id = $1",
            &[&event_id, &projector, &now_ms],
        )
        .await
        .expect("seed projection job");
}

async fn mark_projection_job_completed(
    database_url: &str,
    event_id: &str,
    projector: &str,
    claim_token: &str,
) {
    let now_ms = common::time::now_ms();
    let updated = postgres_client(database_url)
        .await
        .execute(
            "UPDATE ledger_projection_jobs \
             SET status = 'completed', attempt_count = 1, claim_token = $3, \
                 claimed_at_ms = $4, completed_at_ms = $4, updated_at_ms = $4 \
             WHERE event_id = $1 AND projector = $2",
            &[&event_id, &projector, &claim_token, &now_ms],
        )
        .await
        .expect("mark generated projection job completed");
    assert_eq!(updated, 1);
}

async fn projection_job_state(
    database_url: &str,
    event_id: &str,
    projector: &str,
) -> (String, i32, Option<String>) {
    let row = postgres_client(database_url)
        .await
        .query_one(
            "SELECT status, attempt_count, claim_token FROM ledger_projection_jobs \
             WHERE event_id = $1 AND projector = $2",
            &[&event_id, &projector],
        )
        .await
        .expect("projection job state");
    (row.get(0), row.get(1), row.get(2))
}

async fn postgres_client(database_url: &str) -> tokio_postgres::Client {
    let (client, connection) = tokio_postgres::connect(database_url, tokio_postgres::NoTls)
        .await
        .expect("PostgreSQL connection");
    tokio::spawn(async move {
        connection.await.expect("PostgreSQL connection task");
    });
    client
}

fn only_job(mut jobs: Vec<SqlProjectionJob>) -> SqlProjectionJob {
    assert_eq!(jobs.len(), 1);
    jobs.pop().expect("one projection job")
}

fn required_database_url() -> String {
    std::env::var(DATABASE_URL_ENV).unwrap_or_else(|_| {
        panic!("{DATABASE_URL_ENV} must point to a disposable PostgreSQL database")
    })
}

fn unique_id(prefix: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock after epoch")
        .as_nanos();
    format!("{prefix}-{}-{nanos}", std::process::id())
}

fn fill_event(event_id: String) -> ExecutionLedgerEvent {
    let internal_order_id = format!("order-{event_id}");
    ExecutionLedgerEvent {
        event_id,
        event_type: ExecutionLedgerEventType::FillEvent,
        source: OrderUpdateSource::PrivateWs,
        order: ExecutionLedgerOrderRef {
            run_id: Some(format!("run-{internal_order_id}")),
            ticket_id: Some(format!("ticket-{internal_order_id}")),
            leg_role: Some(HedgeLegRole::Long),
            reduce_only: None,
            exchange: "postgres-test".to_owned(),
            symbol: "BTC-USDC".to_owned(),
            side: OrderSide::Buy,
            identity: VenueOrderIdentity {
                internal_order_id: internal_order_id.clone(),
                public_client_order_id: format!("client-{internal_order_id}"),
                product: shared_types::FeeProduct::Perp,
                venue_client_order_id: None,
                exchange_order_id: Some(format!("exchange-{internal_order_id}")),
                client_order_id_policy: None,
                transport_metadata: Default::default(),
            },
        },
        payload: ExecutionLedgerPayload::FillSnapshot(FillLedgerSnapshot {
            quantity: 2.0,
            average_price: 101.5,
            quote_value: 203.0,
            quality: ExecutionLedgerQuality::Actual,
            confidence: ExecutionFillConfidence::VenueFill,
            fee: Some(FeeLedgerSnapshot {
                amount: -0.12,
                currency: Some("USDC".to_owned()),
                quality: ExecutionLedgerQuality::Actual,
            }),
        }),
        occurred_at_ms: 1_000,
        captured_at_ms: 1_001,
    }
}
