#[test]
fn checkpoint_replays_complete_proof_only_for_matching_credential_epoch() {
    let path = temp_checkpoint_path("matching-epoch");
    let first = LiveOrderProofHealthStore::load_checkpoint(
        Some(path.clone()),
        credential_fingerprint_v1,
    );
    assert!(first.problem.is_none());
    first
        .store
        .record_place_ack(sample("bitget", "canary-1", 1_000, None));
    first
        .store
        .record_cancel_requested(sample("bitget", "canary-1", 1_050, None));
    first
        .store
        .record_cancel_finality(sample("bitget", "canary-1", 1_100, None));
    drop(first);

    let restored = LiveOrderProofHealthStore::load_checkpoint(
        Some(path.clone()),
        credential_fingerprint_v1,
    );
    assert!(restored.problem.is_none());
    assert_eq!(restored.restored, 1);
    assert_eq!(restored.store.snapshot(2_000)[0].status, VenueOperationStatus::Ok);
    drop(restored);

    let rotated = LiveOrderProofHealthStore::load_checkpoint(
        Some(path.clone()),
        credential_fingerprint_v2,
    );
    assert!(rotated.problem.is_none());
    assert_eq!(rotated.restored, 0);
    assert!(rotated.store.snapshot(2_000).is_empty());
    let _ = std::fs::remove_file(path);
}

#[test]
fn credential_invalidation_removes_persisted_family_proof() {
    let path = temp_checkpoint_path("credential-invalidation");
    let replay = LiveOrderProofHealthStore::load_checkpoint(
        Some(path.clone()),
        credential_fingerprint_v1,
    );
    replay
        .store
        .record_place_ack(sample("hyperliquid:xyz", "canary-2", 1_000, None));
    replay
        .store
        .record_cancel_finality(sample("hyperliquid:xyz", "canary-2", 1_100, None));
    replay
        .store
        .invalidate_credentials_update("hyperliquid");
    drop(replay);

    let restored = LiveOrderProofHealthStore::load_checkpoint(
        Some(path.clone()),
        credential_fingerprint_v1,
    );
    assert!(restored.problem.is_none());
    assert_eq!(restored.restored, 0);
    assert!(restored.store.snapshot(2_000).is_empty());
    let _ = std::fs::remove_file(path);
}

#[test]
fn newer_remote_write_problem_removes_persisted_proof() {
    let path = temp_checkpoint_path("newer-problem");
    let replay = LiveOrderProofHealthStore::load_checkpoint(
        Some(path.clone()),
        credential_fingerprint_v1,
    );
    replay
        .store
        .record_place_ack(sample("bitget", "canary-3", 1_000, None));
    replay
        .store
        .record_cancel_finality(sample("bitget", "canary-3", 1_100, None));
    replay.store.record_problem(LiveOrderProofProblemInput {
        venue: "bitget",
        source: "submit_order",
        message: "new remote rejection",
        request_id: None,
        retry_after_ms: None,
        status: Some(502),
    });
    drop(replay);

    let restored = LiveOrderProofHealthStore::load_checkpoint(
        Some(path.clone()),
        credential_fingerprint_v1,
    );
    assert!(restored.problem.is_none());
    assert_eq!(restored.restored, 0);
    assert!(restored.store.snapshot(common::time::now_ms()).is_empty());
    let _ = std::fs::remove_file(path);
}

fn credential_fingerprint_v1(venue: &str) -> Option<String> {
    if venue.trim().is_empty() {
        return None;
    }
    Some(format!(
        "credential-v1:{}",
        normalized_venue_name(venue_family(venue))
    ))
}

fn credential_fingerprint_v2(venue: &str) -> Option<String> {
    if venue.trim().is_empty() {
        return None;
    }
    Some(format!(
        "credential-v2:{}",
        normalized_venue_name(venue_family(venue))
    ))
}

fn temp_checkpoint_path(label: &str) -> std::path::PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    std::env::temp_dir().join(format!(
        "crossline-live-order-proof-{label}-{}-{}-{}.json",
        std::process::id(),
        common::time::now_ms(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ))
}
