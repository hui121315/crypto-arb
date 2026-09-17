#[tokio::test]
async fn snapshot_exposes_watchlist_storage_and_bounded_prewarm_evidence() -> anyhow::Result<()> {
    let path = std::env::temp_dir().join(format!(
        "crossline-operation-watchlist-alerts-{}-{}.sqlite",
        std::process::id(),
        common::time::now_ms()
    ));
    let mut config = common::config::AppConfig::default();
    config.api_surface.watchlist_alerts = true;
    config.storage.watchlist_alerts_path = Some(path.display().to_string());
    let state = AppState::new(config).await?;

    let health = snapshot(&state);
    let storage = health
        .rows
        .iter()
        .find(|row| row.operation == shared_types::OP_STORAGE_WATCHLIST_ALERTS)
        .expect("watchlist alert storage health row");
    assert_eq!(storage.status, VenueOperationStatus::Ok);
    assert_eq!(storage.configured, Some(true));
    assert!(storage.evidence.as_ref().is_some_and(|evidence| {
        evidence.schema_hash != UNRECORDED_EVIDENCE_MARKER
            && evidence.fixture_id == "watchlist-alert-sqlite-roundtrip"
    }));

    let prewarm = health
        .rows
        .iter()
        .find(|row| row.operation == shared_types::OP_WATCHLIST_PREWARM)
        .expect("watchlist prewarm health row");
    assert_eq!(prewarm.status, VenueOperationStatus::Ok);
    assert_eq!(prewarm.configured, Some(true));
    assert!(prewarm.evidence.as_ref().is_some_and(|evidence| {
        evidence
            .request_context
            .iter()
            .any(|context| context == "private_ws_symbols=0")
    }));

    drop(state);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
    let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    Ok(())
}
