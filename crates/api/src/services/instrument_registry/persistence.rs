use super::{
    instrument_metadata_evidence, metadata_evidence_matches, normalized_product_key,
    InstrumentRegistry, VenueInstrument, VenueProbeState, SUPPORTED_VENUES,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const CHECKPOINT_VERSION: u32 = 1;
const MAX_CHECKPOINT_BYTES: u64 = 64 * 1024 * 1024;
const MAX_CHECKPOINT_ROWS: usize = 100_000;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Checkpoint {
    version: u32,
    saved_at_ms: i64,
    instruments: Vec<VenueInstrument>,
}

impl InstrumentRegistry {
    pub(crate) async fn load(checkpoint_path: Option<PathBuf>) -> Self {
        let rows = load_checkpoint(checkpoint_path.clone()).await;
        let registry = Self {
            checkpoint_path: checkpoint_path.clone(),
            ..Self::default()
        };
        registry.apply_checkpoint_replay(checkpoint_path.as_deref(), rows);
        registry
    }

    fn apply_checkpoint_replay(
        &self,
        checkpoint_path: Option<&Path>,
        rows: Result<Vec<VenueInstrument>, String>,
    ) {
        let rows = match rows {
            Ok(rows) => rows,
            Err(problem) => {
                tracing::warn!(%problem, "instrument registry checkpoint replay failed; starting empty");
                return;
            }
        };
        self.report_restored_rows(checkpoint_path, self.restore_checkpoint_rows(rows));
    }

    fn report_restored_rows(&self, checkpoint_path: Option<&Path>, restored: usize) {
        if restored == 0 {
            return;
        }
        tracing::info!(
            restored,
            path = %checkpoint_path.map_or_else(
                || "disabled".to_owned(),
                |path| path.display().to_string(),
            ),
            "instrument registry checkpoint restored; fresh probes remain required"
        );
    }

    pub(crate) async fn persist_checkpoint(&self) -> Result<(), String> {
        let Some(path) = self.checkpoint_path.clone() else {
            return Ok(());
        };
        let _guard = self.checkpoint_write_lock.lock().await;
        let mut instruments = self
            .by_key
            .iter()
            .map(|entry| entry.value().clone())
            .collect::<Vec<_>>();
        instruments.sort_by(|left, right| {
            left.venue
                .cmp(&right.venue)
                .then_with(|| left.product_type.cmp(&right.product_type))
                .then_with(|| left.native_symbol.cmp(&right.native_symbol))
        });
        let checkpoint = Checkpoint {
            version: CHECKPOINT_VERSION,
            saved_at_ms: common::time::now_ms().max(1),
            instruments,
        };
        tokio::task::spawn_blocking(move || write_checkpoint(&path, &checkpoint))
            .await
            .map_err(|error| format!("instrument registry checkpoint writer failed: {error}"))?
    }

    fn restore_checkpoint_rows(&self, rows: Vec<VenueInstrument>) -> usize {
        let mut probe_times = BTreeMap::<String, i64>::new();
        let mut spot_probe_times = BTreeMap::<String, i64>::new();
        let mut restored = 0;
        for instrument in rows.into_iter().take(MAX_CHECKPOINT_ROWS) {
            let probe_scope = restored_probe_scope(&instrument.venue);
            let evidence = instrument_metadata_evidence(&probe_scope);
            let valid = self.supports_venue(&instrument.venue)
                && instrument.is_structurally_valid()
                && (evidence.is_some_and(|row| metadata_evidence_matches(&instrument, row))
                    || exchange::official_instrument_evidence_matches(&instrument));
            if !valid {
                continue;
            }
            let checked_at_ms = instrument.checked_at_ms.max(1);
            probe_times
                .entry(probe_scope.clone())
                .and_modify(|current| *current = (*current).max(checked_at_ms))
                .or_insert(checked_at_ms);
            if normalized_product_key(instrument.product_type.as_deref()) == "spot" {
                spot_probe_times
                    .entry(probe_scope)
                    .and_modify(|current| *current = (*current).max(checked_at_ms))
                    .or_insert(checked_at_ms);
            }
            self.by_key.insert(Self::key(&instrument), instrument);
            restored += 1;
        }
        if restored == 0 {
            return 0;
        }
        self.rebuild_instrument_lookup();
        for (venue, checked_at_ms) in probe_times {
            self.probe_by_venue
                .insert(venue, VenueProbeState::Restored { checked_at_ms });
        }
        for (venue, checked_at_ms) in spot_probe_times {
            self.spot_probe_by_venue
                .insert(venue, VenueProbeState::Restored { checked_at_ms });
        }
        restored
    }
}

fn restored_probe_scope(venue: &str) -> String {
    let exact = venue.trim().to_ascii_lowercase();
    if SUPPORTED_VENUES
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(&exact))
    {
        exact
    } else {
        shared_types::venue_family(&exact).to_ascii_lowercase()
    }
}

async fn load_checkpoint(path: Option<PathBuf>) -> Result<Vec<VenueInstrument>, String> {
    let Some(path) = path else {
        return Ok(Vec::new());
    };
    tokio::task::spawn_blocking(move || read_checkpoint(&path))
        .await
        .map_err(|error| format!("instrument registry checkpoint reader failed: {error}"))?
}

fn read_checkpoint(path: &Path) -> Result<Vec<VenueInstrument>, String> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.to_string()),
    };
    if metadata.len() > MAX_CHECKPOINT_BYTES {
        return Err(format!(
            "instrument registry checkpoint exceeds {MAX_CHECKPOINT_BYTES} bytes"
        ));
    }
    let checkpoint: Checkpoint =
        serde_json::from_slice(&fs::read(path).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
    if checkpoint.version != CHECKPOINT_VERSION {
        return Err(format!(
            "unsupported instrument registry checkpoint version {}",
            checkpoint.version
        ));
    }
    if checkpoint.instruments.len() > MAX_CHECKPOINT_ROWS {
        return Err(format!(
            "instrument registry checkpoint contains too many rows: {}",
            checkpoint.instruments.len()
        ));
    }
    Ok(checkpoint.instruments)
}

fn write_checkpoint(path: &Path, checkpoint: &Checkpoint) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let bytes = serde_json::to_vec(checkpoint).map_err(|error| error.to_string())?;
    if bytes.len() > MAX_CHECKPOINT_BYTES as usize {
        return Err(format!(
            "instrument registry checkpoint exceeds {MAX_CHECKPOINT_BYTES} bytes"
        ));
    }
    let temp = path.with_extension(format!(
        "tmp-{}-{}",
        std::process::id(),
        common::time::now_ms().max(1)
    ));
    fs::write(&temp, bytes).map_err(|error| error.to_string())?;
    if let Err(error) = fs::rename(&temp, path) {
        let _ = fs::remove_file(&temp);
        return Err(error.to_string());
    }
    Ok(())
}
