use super::sqlite::{open, sample_columns};
use super::*;
use std::path::Path;

#[tokio::test]
async fn round_trips_nav_samples_and_prunes_old_rows() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("nav.sqlite");
    let config = config(&path);
    let health = NavStorageHealthStore::new(&config);

    append_sample(&config, 1_000, 100.0, &health).await;
    append_sample(&config, 2_000, 120.0, &health).await;

    let rows = load(&config, 1_500, &health).await;

    assert_eq!(rows, vec![(2_000, 120.0)]);
    let snapshot = health.snapshot(common::time::now_ms());
    assert_eq!(snapshot.append_success_total, 2);
    assert_eq!(snapshot.load_success_total, 1);
    assert_eq!(snapshot.schema_version, Some(NAV_SCHEMA_VERSION));
    assert_eq!(snapshot.migration_checksum, Some(nav_schema_hash()));
    assert_eq!(snapshot.sample_count, 1);
    assert_eq!(
        snapshot.latest_sample_status.as_deref(),
        Some(NAV_SAMPLE_STATUS_OK)
    );
    assert_eq!(
        snapshot.latest_sample_source.as_deref(),
        Some(NAV_SAMPLE_SOURCE_ACCOUNT_EQUITY)
    );
    assert_eq!(snapshot.error_total(), 0);
    assert!(snapshot.last_success_at_ms.is_some());
    Ok(())
}

#[tokio::test]
async fn records_nav_load_failure_without_panicking() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let blocked_parent = dir.path().join("not-a-dir");
    std::fs::write(&blocked_parent, b"file")?;
    let config = config(&blocked_parent.join("nav.sqlite"));
    let health = NavStorageHealthStore::new(&config);

    let rows = load(&config, 0, &health).await;
    let snapshot = health.snapshot(common::time::now_ms());

    assert!(rows.is_empty());
    assert_eq!(snapshot.load_error_total, 1);
    assert_eq!(snapshot.success_total(), 0);
    assert!(snapshot.last_error.is_some());
    Ok(())
}

#[tokio::test]
async fn records_nav_append_failure_without_panicking() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let blocked_parent = dir.path().join("not-a-dir");
    std::fs::write(&blocked_parent, b"file")?;
    let config = config(&blocked_parent.join("nav.sqlite"));
    let health = NavStorageHealthStore::new(&config);

    append_sample(&config, 1_000, 100.0, &health).await;
    let snapshot = health.snapshot(common::time::now_ms());

    assert_eq!(snapshot.append_error_total, 1);
    assert_eq!(snapshot.success_total(), 0);
    assert!(snapshot.last_error.is_some());
    Ok(())
}

#[test]
fn disabled_nav_path_is_visible_in_health() {
    let config = AppConfig {
        storage: common::config::StorageConfig {
            portfolio_nav_path: None,
            ..Default::default()
        },
        ..Default::default()
    };
    let health = NavStorageHealthStore::new(&config).snapshot(1_000);

    assert!(!health.enabled);
    assert!(health.path.is_none());
    assert!(health.migration_checksum.is_none());
}

#[test]
fn enabled_health_snapshot_includes_nav_migration_checksum() {
    let config = config(Path::new("/tmp/crossline/nav.sqlite"));
    let health = NavStorageHealthStore::new(&config).snapshot(1_000);

    assert!(health.enabled);
    assert_eq!(health.migration_checksum, Some(nav_schema_hash()));
}

#[test]
fn open_migrates_schema_metadata_and_sample_columns() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("nav.sqlite");
    let conn = open(&path)?;

    let columns = sample_columns(&conn)?;
    let schema_version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    let meta_version: String = conn.query_row(
        "SELECT value FROM portfolio_nav_meta WHERE key = 'schema_version'",
        [],
        |row| row.get(0),
    )?;
    let meta_hash: String = conn.query_row(
        "SELECT value FROM portfolio_nav_meta WHERE key = 'schema_hash'",
        [],
        |row| row.get(0),
    )?;
    let migration_id: String = conn.query_row(
        "SELECT value FROM portfolio_nav_meta WHERE key = 'migration_id'",
        [],
        |row| row.get(0),
    )?;

    assert!(columns.iter().any(|column| column == "status"));
    assert!(columns.iter().any(|column| column == "source"));
    assert!(columns.iter().any(|column| column == "problem"));
    assert_eq!(schema_version, i64::from(NAV_SCHEMA_VERSION));
    assert_eq!(meta_version, NAV_SCHEMA_VERSION.to_string());
    assert_eq!(meta_hash, nav_schema_hash());
    assert_eq!(migration_id, NAV_SCHEMA_MIGRATION_ID);
    Ok(())
}

#[test]
fn nav_schema_migration_is_single_authority_with_stable_hash() {
    for fragment in [
        "CREATE TABLE IF NOT EXISTS portfolio_nav_samples",
        "status         TEXT NOT NULL DEFAULT 'ok'",
        "source         TEXT NOT NULL DEFAULT 'position_margin'",
        "problem        TEXT",
        "CREATE TABLE IF NOT EXISTS portfolio_nav_meta",
        "INSERT OR REPLACE INTO portfolio_nav_meta",
    ] {
        assert!(
            NAV_SCHEMA_SQL.contains(fragment),
            "NAV migration missing {fragment}"
        );
    }
    assert!(NAV_SCHEMA_SQL.contains(&format!("PRAGMA user_version = {NAV_SCHEMA_VERSION}")));

    let hash = nav_schema_hash();
    assert!(hash.starts_with("fnv1a64:"));
    let hex = &hash["fnv1a64:".len()..];
    assert_eq!(hex.len(), 16);
    assert!(hex.chars().all(|item| item.is_ascii_hexdigit()));
    assert_ne!(NAV_SCHEMA_HASH_VALUE, fnv1a64(b""));
}

#[test]
fn skipped_sample_records_unknown_status_without_writing_db() {
    let config = config(Path::new("/tmp/crossline/nav.sqlite"));
    let health = NavStorageHealthStore::new(&config);

    health.record_sample_skipped(
        3_000,
        NAV_SAMPLE_SOURCE_ACCOUNT_EQUITY_MISSING,
        "account equity unavailable",
    );
    let snapshot = health.snapshot(4_000);

    assert_eq!(snapshot.latest_sample_at_ms, Some(3_000));
    assert_eq!(
        snapshot.latest_sample_status.as_deref(),
        Some(NAV_SAMPLE_STATUS_UNKNOWN)
    );
    assert_eq!(
        snapshot.latest_sample_source.as_deref(),
        Some(NAV_SAMPLE_SOURCE_ACCOUNT_EQUITY_MISSING)
    );
    assert_eq!(
        snapshot.latest_sample_problem.as_deref(),
        Some("account equity unavailable")
    );
    assert_eq!(snapshot.sample_count, 0);
}

fn config(path: &Path) -> AppConfig {
    AppConfig {
        storage: common::config::StorageConfig {
            portfolio_nav_path: Some(path.to_string_lossy().into_owned()),
            ..Default::default()
        },
        ..Default::default()
    }
}
