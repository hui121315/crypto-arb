use super::ReplayRows;
use fs2::FileExt;
use serde::Serialize;
use shared_types::CloseRun;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

pub(super) fn append_bare_jsonl(path: &Path, run: &CloseRun) -> std::io::Result<()> {
    append_jsonl(path, run, false)
}

pub(super) fn append_durable_jsonl<T: Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    append_jsonl(path, value, true)
}

pub(super) fn append_jsonl<T: Serialize>(
    path: &Path,
    value: &T,
    durable: bool,
) -> std::io::Result<()> {
    let mut row = serde_json::to_vec(value).map_err(invalid_data)?;
    row.push(b'\n');
    with_ledger_lock(path, || {
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        file.write_all(&row)?;
        if durable {
            file.sync_data()?;
        }
        Ok(())
    })
}

pub(super) fn repair_replay_failures(path: Option<&Path>, rows: &ReplayRows) -> usize {
    if rows.failures == 0 || !rows.repairable {
        return rows.failures;
    }
    let Some(path) = path else {
        return rows.failures;
    };
    report_repair_result(path, rows, repair_jsonl(path, &rows.valid_lines))
}

fn report_repair_result(path: &Path, rows: &ReplayRows, result: std::io::Result<PathBuf>) -> usize {
    match result {
        Ok(backup) => {
            log_repair_success(path, rows, &backup);
            0
        }
        Err(error) => {
            log_repair_failure(path, &error);
            rows.failures
        }
    }
}

fn log_repair_success(path: &Path, rows: &ReplayRows, backup: &Path) {
    tracing::warn!(
        path = %path.display(),
        backup = %backup.display(),
        dropped_lines = rows.failures,
        retained_lines = rows.valid_lines.len(),
        "repaired close run ledger after preserving corrupt backup"
    );
}

fn log_repair_failure(path: &Path, error: &std::io::Error) {
    tracing::warn!(
        path = %path.display(),
        %error,
        "failed to repair close run ledger"
    );
}

fn repair_jsonl(path: &Path, valid_lines: &[Vec<u8>]) -> std::io::Result<PathBuf> {
    with_ledger_lock(path, || {
        let stamp = format!("{}-{}", common::time::now_ms(), std::process::id());
        let backup = suffixed_path(path, &format!(".corrupt-{stamp}.bak"));
        let temp = suffixed_path(path, &format!(".repair-{stamp}.tmp"));
        std::fs::copy(path, &backup)?;
        let result =
            write_repaired_rows(&temp, valid_lines).and_then(|()| std::fs::rename(&temp, path));
        if result.is_err() {
            let _ = std::fs::remove_file(&temp);
        }
        result?;
        sync_parent(path);
        Ok(backup)
    })
}

fn write_repaired_rows(path: &Path, valid_lines: &[Vec<u8>]) -> std::io::Result<()> {
    let mut file = OpenOptions::new().create_new(true).write(true).open(path)?;
    for line in valid_lines {
        file.write_all(line)?;
        file.write_all(b"\n")?;
    }
    file.sync_all()
}

fn with_ledger_lock<T>(
    path: &Path,
    action: impl FnOnce() -> std::io::Result<T>,
) -> std::io::Result<T> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let lock_path = suffixed_path(path, ".lock");
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(lock_path)?;
    lock.lock_exclusive()?;
    let result = action();
    let unlock = FileExt::unlock(&lock);
    match result {
        Ok(value) => unlock.map(|()| value),
        Err(error) => {
            let _ = unlock;
            Err(error)
        }
    }
}

pub(super) fn suffixed_path(path: &Path, suffix: &str) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(suffix);
    PathBuf::from(value)
}

fn sync_parent(path: &Path) {
    if let Some(parent) = path.parent() {
        if let Ok(directory) = File::open(parent) {
            let _ = directory.sync_all();
        }
    }
}

fn invalid_data(error: serde_json::Error) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, error)
}
