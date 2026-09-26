use parking_lot::Mutex as ParkingMutex;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap as OrderedMap;
use std::fs;
use std::path::{Path, PathBuf};

const LIVE_ORDER_PROOF_CHECKPOINT_VERSION: u32 = 1;

pub(crate) type CredentialFingerprintResolver =
    fn(&str, shared_types::FeeProduct) -> Option<String>;

#[derive(Default)]
struct LiveOrderProofCheckpointStore {
    path: Option<PathBuf>,
    credential_fingerprint: Option<CredentialFingerprintResolver>,
    proofs: ParkingMutex<OrderedMap<String, CredentialBoundLiveOrderProof>>,
}

pub(crate) struct LiveOrderProofCheckpointReplay {
    pub(crate) store: LiveOrderProofHealthStore,
    pub(crate) restored: usize,
    pub(crate) problem: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LiveOrderProofCheckpoint {
    version: u32,
    proofs: Vec<CredentialBoundLiveOrderProof>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CredentialBoundLiveOrderProof {
    venue: String,
    credential_fingerprint: String,
    place_proof: LiveOrderProofSample,
    cancel_request: Option<LiveOrderProofSample>,
    cancel_finality: LiveOrderProofSample,
}

impl LiveOrderProofHealthStore {
    pub(crate) fn load_checkpoint(
        path: Option<PathBuf>,
        credential_fingerprint: CredentialFingerprintResolver,
    ) -> LiveOrderProofCheckpointReplay {
        let checkpoint = LiveOrderProofCheckpointStore {
            path,
            credential_fingerprint: Some(credential_fingerprint),
            proofs: ParkingMutex::new(OrderedMap::new()),
        };
        let store = Self {
            rows: DashMap::new(),
            checkpoint,
        };
        let loaded = store.read_checkpoint();
        let (restored, problem) = match loaded {
            Ok(proofs) => (store.restore_credential_bound_proofs(proofs), None),
            Err(error) => (0, Some(error)),
        };
        LiveOrderProofCheckpointReplay {
            store,
            restored,
            problem,
        }
    }

    fn restore_credential_bound_proofs(&self, proofs: Vec<CredentialBoundLiveOrderProof>) -> usize {
        let mut restored = 0;
        let mut retained = self.checkpoint.proofs.lock();
        for proof in proofs {
            if !checkpoint_matches_current_credentials(&self.checkpoint, &proof)
                || !samples_match_order_identity(&proof.place_proof, &proof.cancel_finality)
            {
                continue;
            }
            let key = normalized_venue_name(&proof.venue);
            restore_runtime_row(&self.rows, &key, &proof);
            retained.insert(key, proof);
            restored += 1;
        }
        restored
    }

    fn read_checkpoint(&self) -> Result<Vec<CredentialBoundLiveOrderProof>, String> {
        let Some(path) = self.checkpoint.path.as_deref() else {
            return Ok(Vec::new());
        };
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(format!("live order proof checkpoint read failed: {error}")),
        };
        let checkpoint = serde_json::from_slice::<LiveOrderProofCheckpoint>(&bytes)
            .map_err(|error| format!("live order proof checkpoint decode failed: {error}"))?;
        if checkpoint.version != LIVE_ORDER_PROOF_CHECKPOINT_VERSION {
            return Err(format!(
                "unsupported live order proof checkpoint version {}",
                checkpoint.version
            ));
        }
        Ok(checkpoint.proofs)
    }

    fn persist_complete_checkpoint(&self, key: &str) {
        let Some(proof) = self.checkpoint_for_runtime_row(key) else {
            return;
        };
        let snapshot = {
            let mut proofs = self.checkpoint.proofs.lock();
            proofs.insert(key.to_owned(), proof);
            proofs.values().cloned().collect::<Vec<_>>()
        };
        self.persist_checkpoint_snapshot(snapshot);
    }

    fn checkpoint_for_runtime_row(&self, key: &str) -> Option<CredentialBoundLiveOrderProof> {
        let row = self.rows.get(key)?;
        if problem_is_newer_than_latest_proof(row.value()) {
            return None;
        }
        let place_proof = row.place_proof.clone()?;
        let cancel_finality = row.cancel_finality.clone()?;
        // Bind to the original order, never label an old receipt with a new global fingerprint.
        let fingerprint = place_proof.account_scope.clone()?;
        if self
            .checkpoint
            .credential_fingerprint
            .and_then(|resolve| resolve(&row.venue, place_proof.product))
            .as_ref()
            != Some(&fingerprint)
        {
            return None;
        }
        samples_match_order_identity(&place_proof, &cancel_finality).then(|| {
            CredentialBoundLiveOrderProof {
                venue: row.venue.clone(),
                credential_fingerprint: fingerprint,
                place_proof,
                cancel_request: row.cancel_request.clone(),
                cancel_finality,
            }
        })
    }

    fn remove_checkpoint_exact(&self, venue: &str) {
        let key = normalized_venue_name(venue);
        let snapshot = {
            let mut proofs = self.checkpoint.proofs.lock();
            if proofs.remove(&key).is_none() {
                return;
            }
            proofs.values().cloned().collect::<Vec<_>>()
        };
        self.persist_checkpoint_snapshot(snapshot);
    }

    fn remove_checkpoint_family(&self, venue: &str) {
        let exact_key = normalized_venue_name(venue);
        let family_key = normalized_venue_name(venue_family(venue));
        let snapshot = {
            let mut proofs = self.checkpoint.proofs.lock();
            let before = proofs.len();
            proofs.retain(|key, _| {
                key != &exact_key && normalized_venue_name(venue_family(key)) != family_key
            });
            if proofs.len() == before {
                return;
            }
            proofs.values().cloned().collect::<Vec<_>>()
        };
        self.persist_checkpoint_snapshot(snapshot);
    }

    fn persist_checkpoint_snapshot(&self, proofs: Vec<CredentialBoundLiveOrderProof>) {
        let Some(path) = self.checkpoint.path.as_deref() else {
            return;
        };
        let checkpoint = LiveOrderProofCheckpoint {
            version: LIVE_ORDER_PROOF_CHECKPOINT_VERSION,
            proofs,
        };
        let result = serde_json::to_vec_pretty(&checkpoint)
            .map_err(|error| error.to_string())
            .and_then(|bytes| {
                atomic_write_checkpoint(path, &bytes).map_err(|error| error.to_string())
            });
        if let Err(error) = result {
            tracing::warn!(%error, path = %path.display(), "live order proof checkpoint write failed");
        }
    }
}

fn checkpoint_matches_current_credentials(
    store: &LiveOrderProofCheckpointStore,
    proof: &CredentialBoundLiveOrderProof,
) -> bool {
    !proof.credential_fingerprint.trim().is_empty()
        && proof.place_proof.account_scope.as_ref() == Some(&proof.credential_fingerprint)
        && store
            .credential_fingerprint
            .and_then(|resolve| resolve(&proof.venue, proof.place_proof.product))
            .as_deref()
            == Some(proof.credential_fingerprint.as_str())
}

fn restore_runtime_row(
    rows: &DashMap<String, LiveOrderProofRuntimeHealth>,
    key: &str,
    proof: &CredentialBoundLiveOrderProof,
) {
    let observed_at_ms = proof
        .place_proof
        .checked_at_ms
        .max(proof.cancel_finality.checked_at_ms);
    let mut row = empty_runtime_health(&proof.venue, observed_at_ms);
    row.place_ack_count = 1;
    row.cancel_requested_count = u64::from(proof.cancel_request.is_some());
    row.cancel_finality_count = 1;
    row.place_proof = Some(proof.place_proof.clone());
    row.cancel_request = proof.cancel_request.clone();
    row.cancel_finality = Some(proof.cancel_finality.clone());
    refresh_runtime_row(&mut row);
    rows.insert(key.to_owned(), row);
}

fn atomic_write_checkpoint(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp = path.with_extension(format!("tmp-{}", std::process::id()));
    fs::write(&temp, bytes)?;
    if let Err(error) = fs::rename(&temp, path) {
        let _ = fs::remove_file(&temp);
        return Err(error);
    }
    Ok(())
}
