use super::*;

#[tokio::test]
async fn disabled_store_is_noop() -> Result<(), Box<dyn std::error::Error>> {
    let store = HistoryStore::disabled();
    store.append_funding_rates(&[funding_rate()]).await?;
    let rows = store
        .query_funding(FundingQuery {
            limit: 10,
            ..FundingQuery::default()
        })
        .await?;
    assert!(rows.is_empty());
    assert_eq!(store.backend_name(), "disabled");
    Ok(())
}

#[tokio::test]
async fn health_snapshot_tracks_memory_io() -> Result<(), Box<dyn std::error::Error>> {
    let store = HistoryStore::new(10);
    store.append_funding_rates(&[funding_rate()]).await?;
    let _rows = store
        .query_funding(FundingQuery {
            limit: 10,
            ..FundingQuery::default()
        })
        .await?;

    let health = store.health_snapshot(1_000);

    assert_eq!(health.backend, "memory");
    assert_eq!(
        health.storage_contract.backend_kind,
        shared_types::StorageBackendKind::Memory
    );
    assert!(health
        .storage_contract
        .degraded_reasons
        .contains(&shared_types::StorageDegradedReason::Ephemeral));
    assert!(health.enabled);
    assert!(!health.durable);
    assert!(health.ephemeral);
    assert_eq!(health.schema_version, Some(HISTORY_SCHEMA_VERSION));
    let checksum = history_migration_checksum_hex();
    assert_eq!(
        health.migration_checksum.as_deref(),
        Some(checksum.as_str())
    );
    assert_eq!(health.timescale_status, None);
    assert_eq!(health.timescale_problem, None);
    assert_eq!(health.append_success_total, 1);
    assert_eq!(health.query_success_total, 1);
    assert_eq!(health.error_total(), 0);
    assert!(health.last_success_at_ms.is_some());
    assert!(health.last_append_at_ms.is_some());
    assert!(health.last_query_at_ms.is_some());
    Ok(())
}

#[test]
fn memory_fallback_health_preserves_startup_problem() {
    let store = HistoryStore::memory_fallback("postgres history store unavailable: db down");

    let health = store.health_snapshot(1_000);

    assert_eq!(health.backend, "memory");
    assert!(health.enabled);
    assert!(health.ephemeral);
    assert_eq!(health.schema_version, Some(HISTORY_SCHEMA_VERSION));
    let checksum = history_migration_checksum_hex();
    assert_eq!(
        health.migration_checksum.as_deref(),
        Some(checksum.as_str())
    );
    assert_eq!(health.timescale_status, None);
    assert_eq!(health.timescale_problem, None);
    assert!(health.fallback);
    assert!(health
        .storage_contract
        .degraded_reasons
        .contains(&shared_types::StorageDegradedReason::Fallback));
    assert_eq!(
        health.startup_problem.as_deref(),
        Some("postgres history store unavailable: db down")
    );
}

#[test]
fn schema_drift_records_typed_storage_reason() {
    let store = HistoryStore::new(10);
    let result = store.record_query::<()>(Err(HistoryError::SchemaDrift(
        "funding_rates missing".to_owned(),
    )));

    assert!(result.is_err());
    let health = store.health_snapshot(1_000);
    assert!(health.has_degraded_reason(shared_types::StorageDegradedReason::SchemaDrift));
    assert_eq!(
        health.backend_status().storage_contract.degraded_reasons,
        health.storage_contract.degraded_reasons
    );
}
