use shared_types::OrderRecord;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

#[derive(Debug, Default)]
pub(super) struct OrderSnapshotReplay {
    pub(super) rows: Vec<OrderRecord>,
    pub(super) failed_lines: usize,
}

pub(super) fn append_jsonl(path: &Path, record: &OrderRecord) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    serde_json::to_writer(&mut file, record)?;
    file.write_all(b"\n")?;
    Ok(())
}

pub(super) fn read_jsonl(path: &Path) -> std::io::Result<OrderSnapshotReplay> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(OrderSnapshotReplay::default());
        }
        Err(err) => return Err(err),
    };
    let mut replay = OrderSnapshotReplay::default();
    for line in BufReader::new(file).lines() {
        push_line(&mut replay, &line?);
    }
    Ok(replay)
}

fn push_line(replay: &mut OrderSnapshotReplay, line: &str) {
    let line = line.trim();
    if line.is_empty() {
        return;
    }
    match serde_json::from_str::<OrderRecord>(line) {
        Ok(record) => replay.rows.push(record),
        Err(_) => replay.failed_lines = replay.failed_lines.saturating_add(1),
    }
}
