use super::*;
use shared_types::{CloseRunScope, CloseRunStatus};
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::Arc;

const PROJECTOR: &str = "close_run_v1";

#[test]
fn replays_old_bare_snapshots() -> Result<(), Box<dyn std::error::Error>> {
    let path = temp_path("old-format");
    let mut first = close_run("close-1", CloseRunStatus::Submitted);
    let mut second = close_run("close-1", CloseRunStatus::Succeeded);
    first.message = "submitted".to_owned();
    second.message = "filled".to_owned();

    append_bare_jsonl(&path, &first)?;
    append_bare_jsonl(&path, &second)?;
    let rows = read_jsonl(&path)?;
    let runs = latest_runs(rows.runs);

    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, CloseRunStatus::Succeeded);
    assert_eq!(runs[0].message, "filled");
    assert!(rows.receipts.is_empty());
    remove(&path);
    Ok(())
}

#[test]
fn replays_projected_snapshot_and_receipt_after_bare_row() -> Result<(), Box<dyn std::error::Error>>
{
    let path = temp_path("receipt");
    let first = close_run("close-1", CloseRunStatus::Submitted);
    let projected = close_run("close-1", CloseRunStatus::Succeeded);
    append_bare_jsonl(&path, &first)?;

    let store = CloseRunStore::load(&config(&path)).store;
    assert_eq!(
        store.append_projected(&[projected], PROJECTOR, "event-1")?,
        ProjectionAppend::Appended
    );
    let replay = CloseRunStore::load(&config(&path));

    assert_eq!(replay.runs.len(), 1);
    assert_eq!(replay.runs[0].status, CloseRunStatus::Succeeded);
    assert!(replay.store.has_projection_receipt(PROJECTOR, "event-1"));
    remove(&path);
    Ok(())
}

#[test]
fn malformed_rows_remain_skippable() -> Result<(), Box<dyn std::error::Error>> {
    let path = temp_path("malformed");
    append_bare_jsonl(&path, &close_run("close-1", CloseRunStatus::Submitted))?;
    let mut file = OpenOptions::new().append(true).open(&path)?;
    file.write_all(b"{bad json}\n")?;
    drop(file);
    append_bare_jsonl(&path, &close_run("close-2", CloseRunStatus::Succeeded))?;

    let rows = read_jsonl(&path)?;

    assert_eq!(rows.failures, 1);
    assert_eq!(rows.runs.len(), 2);
    remove(&path);
    Ok(())
}

#[test]
fn unknown_projection_envelope_schema_is_skipped() -> Result<(), Box<dyn std::error::Error>> {
    let path = temp_path("unknown-schema");
    append_jsonl(
        &path,
        &ProjectedCloseRuns {
            schema_version: PROJECTED_ENVELOPE_VERSION + 1,
            runs: vec![close_run("close-1", CloseRunStatus::Succeeded)],
            receipt: ProjectionReceipt {
                projector: PROJECTOR.to_owned(),
                event_id: "event-1".to_owned(),
            },
        },
        false,
    )?;

    let rows = read_jsonl(&path)?;
    assert_eq!(rows.failures, 1);
    assert!(rows.runs.is_empty());
    assert!(rows.receipts.is_empty());
    remove(&path);
    Ok(())
}

#[test]
fn load_repairs_malformed_rows_and_preserves_backup() -> Result<(), Box<dyn std::error::Error>> {
    let path = temp_path("replay-health");
    std::fs::write(&path, b"{bad json}\n")?;

    let replay = CloseRunStore::load(&config(&path));

    assert!(replay.store.persistence_configured());
    assert!(!replay.store.persistence_degraded());
    assert!(replay.runs.is_empty());
    assert_eq!(std::fs::read(&path)?, b"");
    assert_eq!(corrupt_backups(&path).len(), 1);
    remove(&path);
    Ok(())
}

#[test]
fn concurrent_bare_appends_replay_without_interleaving() -> Result<(), Box<dyn std::error::Error>> {
    let path = temp_path("concurrent");
    let store = Arc::new(CloseRunStore::load(&config(&path)).store);
    let threads = (0..32)
        .map(|index| {
            let store = Arc::clone(&store);
            std::thread::spawn(move || {
                store.append(&close_run(
                    &format!("close-{index}"),
                    CloseRunStatus::Submitted,
                ));
            })
        })
        .collect::<Vec<_>>();
    for thread in threads {
        thread.join().map_err(|_| "append thread panicked")?;
    }

    let rows = read_jsonl(&path)?;
    assert_eq!(rows.failures, 0);
    assert_eq!(rows.runs.len(), 32);
    remove(&path);
    Ok(())
}

fn config(path: &Path) -> AppConfig {
    let mut config = AppConfig::default();
    config.storage.close_run_ledger_path = Some(path.display().to_string());
    config
}

fn close_run(id: &str, status: CloseRunStatus) -> CloseRun {
    CloseRun {
        id: id.to_owned(),
        scope: CloseRunScope::Single,
        status,
        action_run_id: Some("act-1".to_owned()),
        request_id: Some("req-1".to_owned()),
        idempotency_key: None,
        snapshot_version: "pos-1".to_owned(),
        expected_leg_count: 0,
        reason: None,
        legs: Vec::new(),
        submitted_order_count: 0,
        failed_leg_count: 0,
        naked_exposure_usd: 0.0,
        message: "seed".to_owned(),
        problem: None,
        finality_problem: None,
        finality_checked_at_ms: None,
        unwind_plan: None,
        cost_events: Vec::new(),
        cost_reconciliation: None,
        started_at_ms: 1,
        updated_at_ms: 1,
    }
}

fn temp_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "crossline-close-run-{label}-{}-{}.jsonl",
        std::process::id(),
        common::time::now_ms()
    ))
}

fn remove(path: &Path) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(suffixed_path(path, ".lock"));
    for backup in corrupt_backups(path) {
        let _ = std::fs::remove_file(backup);
    }
}

fn corrupt_backups(path: &Path) -> Vec<PathBuf> {
    let Some(parent) = path.parent() else {
        return Vec::new();
    };
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return Vec::new();
    };
    std::fs::read_dir(parent)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|candidate| {
            candidate
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(&format!("{file_name}.corrupt-")))
        })
        .collect()
}
