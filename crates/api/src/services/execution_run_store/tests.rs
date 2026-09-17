use super::*;
use shared_types::{
    ExecutionFillConfidence, ExecutionRunEventKind, ExecutionRunLeg, ExecutionRunState,
    ExecutionRunTimelineEvent, HedgeLegRole, LiveOrderState, OrderUpdateSource, RecoveryAction,
};

const PROJECTOR: &str = "execution_run_v1";

#[test]
fn replays_old_bare_snapshots() -> Result<(), Box<dyn std::error::Error>> {
    let path = temp_path("old-format");
    let mut first = run("run-1", ExecutionRunState::SubmittingSecondLeg);
    let mut second = run("run-1", ExecutionRunState::Hedged);
    first.status_reason = "submitting".to_owned();
    second.status_reason = "hedged".to_owned();

    append_bare_jsonl(&path, &first)?;
    append_bare_jsonl(&path, &second)?;
    let rows = read_jsonl(&path)?;
    let runs = latest_runs(rows.runs);

    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].state, ExecutionRunState::Hedged);
    assert_eq!(runs[0].status_reason, "hedged");
    assert!(rows.receipts.is_empty());
    remove(path);
    Ok(())
}

#[test]
fn replays_projected_snapshot_and_receipt_after_bare_row() -> Result<(), Box<dyn std::error::Error>>
{
    let path = temp_path("receipt");
    let first = run("run-1", ExecutionRunState::SubmittingSecondLeg);
    let projected = run("run-1", ExecutionRunState::Hedged);
    append_bare_jsonl(&path, &first)?;

    let store = ExecutionRunStore::load(&config(&path)).store;
    assert_eq!(
        store.append_projected(&[projected], PROJECTOR, "event-1")?,
        ProjectionAppend::Appended
    );
    let replay = ExecutionRunStore::load(&config(&path));

    assert_eq!(replay.runs.len(), 1);
    assert_eq!(replay.runs[0].state, ExecutionRunState::Hedged);
    assert!(replay.store.has_projection_receipt(PROJECTOR, "event-1"));
    assert_eq!(
        replay.store.append_projected(&[], PROJECTOR, "event-1")?,
        ProjectionAppend::AlreadyReceived
    );
    remove(path);
    Ok(())
}

#[test]
fn durable_run_snapshot_replays_embedded_timeline() {
    let path = temp_path("timeline");
    let mut persisted = run("run-timeline", ExecutionRunState::SecondLegSubmitted);
    persisted.evidence.request_id = Some("req-timeline".to_owned());
    persisted.evidence.events.push(ExecutionRunTimelineEvent {
        event_id: "event-preview".to_owned(),
        kind: ExecutionRunEventKind::Preview,
        state: ExecutionRunState::Previewed,
        source: OrderUpdateSource::Internal,
        message: "previewed".to_owned(),
        occurred_at_ms: 1,
        request_id: Some("req-timeline".to_owned()),
        leg_role: None,
        order_identity: None,
        ledger_event_type: None,
        finality_confidence: ExecutionFillConfidence::Unknown,
        problem: None,
    });

    ExecutionRunStore::load(&config(&path))
        .store
        .append(&persisted);
    let replay = ExecutionRunStore::load(&config(&path));

    assert_eq!(replay.runs.len(), 1);
    assert_eq!(replay.runs[0].evidence.events.len(), 1);
    assert_eq!(
        replay.runs[0].evidence.request_id.as_deref(),
        Some("req-timeline")
    );
    assert_eq!(replay.runs[0].evidence.events[0].event_id, "event-preview");
    remove(path);
}

#[test]
fn rejects_unknown_projection_envelope_schema() -> Result<(), Box<dyn std::error::Error>> {
    let path = temp_path("unknown-schema");
    append_jsonl(
        &path,
        &ProjectedExecutionRuns {
            schema_version: PROJECTED_ENVELOPE_VERSION + 1,
            runs: vec![run("run-1", ExecutionRunState::Hedged)],
            receipt: ProjectionReceipt {
                projector: PROJECTOR.to_owned(),
                event_id: "event-1".to_owned(),
            },
        },
        false,
    )?;

    assert!(read_jsonl(&path).is_err());
    remove(path);
    Ok(())
}

#[test]
fn load_retains_replay_failure_health() -> Result<(), Box<dyn std::error::Error>> {
    let path = temp_path("replay-health");
    std::fs::write(&path, b"{bad json}\n")?;

    let replay = ExecutionRunStore::load(&config(&path));

    assert!(replay.store.persistence_configured());
    assert!(replay.store.persistence_degraded());
    assert!(replay.runs.is_empty());
    remove(path);
    Ok(())
}

#[test]
fn production_replay_filter_drops_only_impossible_runtime_rows() {
    let mut invalid = run("test-seed", ExecutionRunState::Hedged);
    invalid.created_at_ms = 1;
    invalid.updated_at_ms = 9_000_000;
    let mut valid = run("live-history", ExecutionRunState::Closed);
    valid.created_at_ms = MIN_PERSISTED_RUN_TIMESTAMP_MS;
    valid.updated_at_ms = MIN_PERSISTED_RUN_TIMESTAMP_MS + 1;
    let mut rows = vec![invalid, valid];

    let filtered = filter_invalid_replay_runs(&mut rows, false);

    assert_eq!(filtered, 1);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].run_id, "live-history");
}

#[test]
fn test_replay_keeps_synthetic_timestamps_inside_isolated_test_state() {
    let mut rows = vec![run("test-seed", ExecutionRunState::Hedged)];

    let filtered = filter_invalid_replay_runs(&mut rows, true);

    assert_eq!(filtered, 0);
    assert_eq!(rows.len(), 1);
}

fn config(path: &Path) -> AppConfig {
    let mut config = AppConfig::default();
    config.storage.execution_run_ledger_path = Some(path.display().to_string());
    config
}

fn run(id: &str, state: ExecutionRunState) -> ExecutionRun {
    ExecutionRun {
        run_id: id.to_owned(),
        ticket_id: "ticket-1".to_owned(),
        opportunity_id: "opp-1".to_owned(),
        state,
        long_leg: leg(HedgeLegRole::Long),
        short_leg: leg(HedgeLegRole::Short),
        net_exposure_usd: 0.0,
        cost_reconciliation: None,
        valuation_problem: None,
        unwind_problem: None,
        finality_problem: None,
        finality_checked_at_ms: None,
        evidence: Default::default(),
        recovery_action: Some(RecoveryAction::ManualReview),
        status_reason: "seed".to_owned(),
        created_at_ms: 1,
        updated_at_ms: 1,
    }
}

fn leg(role: HedgeLegRole) -> ExecutionRunLeg {
    ExecutionRunLeg {
        role,
        exchange: "paper".to_owned(),
        symbol: "BTC-USDT".to_owned(),
        order_ids: Vec::new(),
        identity: None,
        finality_source: None,
        confirmed_filled_at_ms: None,
        state: LiveOrderState::Created,
        target_quantity: 0.0,
        filled_quantity: None,
        target_notional_usd: 0.0,
        filled_notional_usd: None,
        filled_fee: None,
    }
}

fn temp_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "crossline-execution-run-{label}-{}-{}.jsonl",
        std::process::id(),
        common::time::now_ms()
    ))
}

fn remove(path: PathBuf) {
    let _ = std::fs::remove_file(path);
}
