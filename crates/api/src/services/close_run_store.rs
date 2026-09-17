use common::config::AppConfig;
use dashmap::DashSet;
use parking_lot::{Mutex, MutexGuard};
use serde::{Deserialize, Serialize};
use shared_types::CloseRun;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

mod ledger;
use ledger::{append_bare_jsonl, append_durable_jsonl, repair_replay_failures};
#[cfg(test)]
use ledger::{append_jsonl, suffixed_path};

const PROJECTED_ENVELOPE_VERSION: u8 = 1;
#[derive(Debug)]
pub(crate) struct CloseRunStore {
    path: Option<PathBuf>,
    projection_receipts: DashSet<(String, String)>,
    projection_lock: Mutex<()>,
    append_lock: Mutex<()>,
    append_successes: AtomicUsize,
    append_failures: AtomicUsize,
    replay_failures: usize,
    last_append_at_ms: AtomicI64,
}
pub(crate) struct CloseRunStoreReplay {
    pub(crate) store: CloseRunStore,
    pub(crate) runs: Vec<CloseRun>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProjectionAppend {
    Appended,
    AlreadyReceived,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProjectionReceipt {
    projector: String,
    event_id: String,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProjectedCloseRuns {
    schema_version: u8,
    runs: Vec<CloseRun>,
    receipt: ProjectionReceipt,
}
#[derive(Debug)]
enum CloseRunLogEntry {
    Projected(ProjectedCloseRuns),
    Bare(Box<CloseRun>),
}
struct ReplayRows {
    runs: Vec<CloseRun>,
    receipts: Vec<ProjectionReceipt>,
    failures: usize,
    valid_lines: Vec<Vec<u8>>,
    repairable: bool,
}

impl CloseRunStore {
    pub(crate) fn load(config: &AppConfig) -> CloseRunStoreReplay {
        let path = close_run_ledger_path(config);
        let rows = replay_close_runs(path.as_deref());
        let replay_failures = repair_replay_failures(path.as_deref(), &rows);
        if replay_failures > 0 {
            tracing::warn!(replay_failures, "close run ledger replay degraded");
        }
        CloseRunStoreReplay {
            runs: latest_runs(rows.runs),
            store: Self {
                path,
                projection_receipts: rows
                    .receipts
                    .into_iter()
                    .map(|receipt| (receipt.projector, receipt.event_id))
                    .collect(),
                projection_lock: Mutex::new(()),
                append_lock: Mutex::new(()),
                append_successes: AtomicUsize::new(0),
                append_failures: AtomicUsize::new(0),
                replay_failures,
                last_append_at_ms: AtomicI64::new(0),
            },
        }
    }

    pub(crate) fn append(&self, run: &CloseRun) {
        let Some(path) = self.path.as_deref() else {
            return;
        };
        let _append_guard = self.append_lock.lock();
        self.record_append_result(path, append_bare_jsonl(path, run));
    }

    pub(crate) fn persistence_configured(&self) -> bool {
        self.path.is_some()
    }

    pub(crate) fn persistence_degraded(&self) -> bool {
        self.replay_failures > 0 || self.append_failures.load(Ordering::Acquire) > 0
    }

    pub(crate) fn has_projection_receipt(&self, projector: &str, event_id: &str) -> bool {
        self.projection_receipts
            .contains(&(projector.to_owned(), event_id.to_owned()))
    }

    pub(crate) fn lock_projection(&self) -> MutexGuard<'_, ()> {
        self.projection_lock.lock()
    }

    pub(crate) fn append_projected(
        &self,
        runs: &[CloseRun],
        projector: &str,
        event_id: &str,
    ) -> std::io::Result<ProjectionAppend> {
        self.append_projection_envelope(runs, projector, event_id)
    }

    fn append_projection_envelope(
        &self,
        runs: &[CloseRun],
        projector: &str,
        event_id: &str,
    ) -> std::io::Result<ProjectionAppend> {
        let _append_guard = self.append_lock.lock();
        if self.has_projection_receipt(projector, event_id) {
            return Ok(ProjectionAppend::AlreadyReceived);
        }
        let receipt = ProjectionReceipt {
            projector: projector.to_owned(),
            event_id: event_id.to_owned(),
        };
        let envelope = ProjectedCloseRuns {
            schema_version: PROJECTED_ENVELOPE_VERSION,
            runs: runs.to_vec(),
            receipt: receipt.clone(),
        };
        if let Some(path) = self.path.as_deref() {
            if let Err(error) = append_durable_jsonl(path, &envelope) {
                self.append_failures.fetch_add(1, Ordering::AcqRel);
                return Err(error);
            }
        }
        self.projection_receipts
            .insert((receipt.projector, receipt.event_id));
        self.mark_append_success();
        Ok(ProjectionAppend::Appended)
    }

    fn record_append_result(&self, path: &Path, result: std::io::Result<()>) {
        match result {
            Ok(()) => self.mark_append_success(),
            Err(error) => {
                self.append_failures.fetch_add(1, Ordering::AcqRel);
                tracing::warn!(
                    path = %path.display(),
                    %error,
                    "failed to append close run ledger snapshot"
                );
            }
        }
    }

    fn mark_append_success(&self) {
        self.append_successes.fetch_add(1, Ordering::AcqRel);
        self.last_append_at_ms
            .store(common::time::now_ms(), Ordering::Release);
    }
}

fn close_run_ledger_path(config: &AppConfig) -> Option<PathBuf> {
    config
        .storage
        .close_run_ledger_path
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(|path| config.storage.resolve_runtime_path(path))
}

fn replay_close_runs(path: Option<&Path>) -> ReplayRows {
    let Some(path) = path else {
        return empty_replay();
    };
    match read_jsonl(path) {
        Ok(rows) => rows,
        Err(error) => {
            tracing::warn!(path = %path.display(), %error, "failed to replay close run ledger");
            ReplayRows {
                failures: 1,
                ..empty_replay()
            }
        }
    }
}

fn empty_replay() -> ReplayRows {
    ReplayRows {
        runs: Vec::new(),
        receipts: Vec::new(),
        failures: 0,
        valid_lines: Vec::new(),
        repairable: false,
    }
}

fn latest_runs(runs: Vec<CloseRun>) -> Vec<CloseRun> {
    let mut latest = BTreeMap::new();
    for run in runs {
        latest.insert(run.id.clone(), run);
    }
    latest
        .into_values()
        .map(|mut run| {
            crate::services::close_runs::normalize_replayed_paper_finality(&mut run);
            run
        })
        .collect()
}

fn read_jsonl(path: &Path) -> std::io::Result<ReplayRows> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(empty_replay()),
        Err(error) => return Err(error),
    };
    let mut rows = ReplayRows {
        repairable: true,
        ..empty_replay()
    };
    for line in BufReader::new(file).lines() {
        let line = line?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if append_replay_line(&mut rows, line) {
            rows.valid_lines.push(line.as_bytes().to_vec());
        }
    }
    Ok(rows)
}

fn append_replay_line(rows: &mut ReplayRows, line: &str) -> bool {
    match parse_log_entry(line) {
        Ok(CloseRunLogEntry::Bare(run)) => {
            rows.runs.push(*run);
            true
        }
        Ok(CloseRunLogEntry::Projected(envelope)) => append_projected_replay(rows, envelope),
        Err(error) => {
            rows.failures += 1;
            tracing::warn!(%error, "skipping malformed close run ledger line");
            false
        }
    }
}

fn parse_log_entry(line: &str) -> Result<CloseRunLogEntry, String> {
    let bare_error = match serde_json::from_str::<CloseRun>(line) {
        Ok(run) => return Ok(CloseRunLogEntry::Bare(Box::new(run))),
        Err(error) => error,
    };
    match serde_json::from_str::<ProjectedCloseRuns>(line) {
        Ok(envelope) => Ok(CloseRunLogEntry::Projected(envelope)),
        Err(projected_error) => Err(format!(
            "close run row failed bare parse ({bare_error}) and projection parse ({projected_error})"
        )),
    }
}

fn append_projected_replay(rows: &mut ReplayRows, envelope: ProjectedCloseRuns) -> bool {
    if validate_projected_envelope(&envelope) {
        rows.runs.extend(envelope.runs);
        rows.receipts.push(envelope.receipt);
        true
    } else {
        rows.failures += 1;
        tracing::warn!("skipping invalid close run projection envelope");
        false
    }
}

fn validate_projected_envelope(envelope: &ProjectedCloseRuns) -> bool {
    envelope.schema_version == PROJECTED_ENVELOPE_VERSION
        && !envelope.runs.is_empty()
        && !envelope.receipt.projector.trim().is_empty()
        && !envelope.receipt.event_id.trim().is_empty()
}

#[cfg(test)]
mod tests;
