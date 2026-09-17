use common::config::AppConfig;
use dashmap::DashSet;
use parking_lot::{Mutex, MutexGuard};
use serde::{Deserialize, Serialize};
use shared_types::ExecutionRun;
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

mod replay_filter;

use replay_filter::{filter_invalid_replay_runs, report_replay_health};

const PROJECTED_ENVELOPE_VERSION: u8 = 1;
const MIN_PERSISTED_RUN_TIMESTAMP_MS: i64 = 946_684_800_000;

#[derive(Debug)]
pub(crate) struct ExecutionRunStore {
    path: Option<PathBuf>,
    projection_receipts: DashSet<(String, String)>,
    projection_lock: Mutex<()>,
    projected_append_lock: Mutex<()>,
    append_successes: AtomicUsize,
    append_failures: AtomicUsize,
    replay_failures: usize,
    last_append_at_ms: AtomicI64,
}

pub(crate) struct ExecutionRunStoreReplay {
    pub(crate) store: ExecutionRunStore,
    pub(crate) runs: Vec<ExecutionRun>,
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
struct ProjectedExecutionRuns {
    schema_version: u8,
    runs: Vec<ExecutionRun>,
    receipt: ProjectionReceipt,
}

#[derive(Debug)]
enum ExecutionRunLogEntry {
    Projected(ProjectedExecutionRuns),
    Bare(Box<ExecutionRun>),
}

struct ReplayRows {
    runs: Vec<ExecutionRun>,
    receipts: Vec<ProjectionReceipt>,
}

impl ExecutionRunStore {
    pub(crate) fn load(config: &AppConfig) -> ExecutionRunStoreReplay {
        let path = execution_run_ledger_path(config);
        let (mut rows, replay_failures) = replay_execution_runs(path.as_deref());
        let filtered_runs = filter_invalid_replay_runs(&mut rows.runs, cfg!(test));
        report_replay_health(path.as_deref(), replay_failures, filtered_runs);
        ExecutionRunStoreReplay {
            runs: latest_runs(rows.runs),
            store: Self {
                path,
                projection_receipts: rows
                    .receipts
                    .into_iter()
                    .map(|receipt| (receipt.projector, receipt.event_id))
                    .collect(),
                projection_lock: Mutex::new(()),
                projected_append_lock: Mutex::new(()),
                append_successes: AtomicUsize::new(0),
                append_failures: AtomicUsize::new(0),
                replay_failures,
                last_append_at_ms: AtomicI64::new(0),
            },
        }
    }

    pub(crate) fn append(&self, run: &ExecutionRun) {
        let Some(path) = self.path.as_deref() else {
            return;
        };
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
        runs: &[ExecutionRun],
        projector: &str,
        event_id: &str,
    ) -> std::io::Result<ProjectionAppend> {
        self.append_projection_envelope(runs, projector, event_id)
    }

    fn append_projection_envelope(
        &self,
        runs: &[ExecutionRun],
        projector: &str,
        event_id: &str,
    ) -> std::io::Result<ProjectionAppend> {
        let _append_guard = self.projected_append_lock.lock();
        if self.has_projection_receipt(projector, event_id) {
            return Ok(ProjectionAppend::AlreadyReceived);
        }
        let receipt = ProjectionReceipt {
            projector: projector.to_owned(),
            event_id: event_id.to_owned(),
        };
        let envelope = ProjectedExecutionRuns {
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
                    "failed to append execution run ledger snapshot"
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

fn execution_run_ledger_path(config: &AppConfig) -> Option<PathBuf> {
    config
        .storage
        .execution_run_ledger_path
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(|path| config.storage.resolve_runtime_path(path))
}

fn replay_execution_runs(path: Option<&Path>) -> (ReplayRows, usize) {
    let Some(path) = path else {
        return (empty_replay(), 0);
    };
    match read_jsonl(path) {
        Ok(rows) => (rows, 0),
        Err(error) => {
            tracing::warn!(path = %path.display(), %error, "failed to replay execution run ledger");
            (empty_replay(), 1)
        }
    }
}

fn empty_replay() -> ReplayRows {
    ReplayRows {
        runs: Vec::new(),
        receipts: Vec::new(),
    }
}

fn latest_runs(runs: Vec<ExecutionRun>) -> Vec<ExecutionRun> {
    let mut latest = BTreeMap::new();
    for run in runs {
        latest.insert(run.run_id.clone(), run);
    }
    latest.into_values().collect()
}

fn append_bare_jsonl(path: &Path, run: &ExecutionRun) -> std::io::Result<()> {
    append_jsonl(path, run, true)
}

fn append_durable_jsonl<T: Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    append_jsonl(path, value, true)
}

fn append_jsonl<T: Serialize>(path: &Path, value: &T, durable: bool) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut row = serde_json::to_vec(value).map_err(invalid_data)?;
    row.push(b'\n');
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    file.write_all(&row)?;
    if durable {
        file.sync_data()?;
    }
    Ok(())
}

fn read_jsonl(path: &Path) -> std::io::Result<ReplayRows> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(empty_replay()),
        Err(error) => return Err(error),
    };
    let mut rows = empty_replay();
    for line in BufReader::new(file).lines() {
        let line = line?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match parse_log_entry(line)? {
            ExecutionRunLogEntry::Bare(run) => rows.runs.push(*run),
            ExecutionRunLogEntry::Projected(envelope) => {
                validate_projected_envelope(&envelope)?;
                rows.runs.extend(envelope.runs);
                rows.receipts.push(envelope.receipt);
            }
        }
    }
    Ok(rows)
}

fn parse_log_entry(line: &str) -> std::io::Result<ExecutionRunLogEntry> {
    let bare_error = match serde_json::from_str::<ExecutionRun>(line) {
        Ok(run) => return Ok(ExecutionRunLogEntry::Bare(Box::new(run))),
        Err(error) => error,
    };
    match serde_json::from_str::<ProjectedExecutionRuns>(line) {
        Ok(envelope) => Ok(ExecutionRunLogEntry::Projected(envelope)),
        Err(projected_error) => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "execution run row failed bare parse ({bare_error}) and projection parse ({projected_error})"
            ),
        )),
    }
}

fn validate_projected_envelope(envelope: &ProjectedExecutionRuns) -> std::io::Result<()> {
    if envelope.schema_version != PROJECTED_ENVELOPE_VERSION
        || envelope.runs.is_empty()
        || envelope.receipt.projector.trim().is_empty()
        || envelope.receipt.event_id.trim().is_empty()
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "invalid execution projection envelope",
        ));
    }
    Ok(())
}

fn invalid_data(error: serde_json::Error) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, error)
}

#[cfg(test)]
mod tests;
