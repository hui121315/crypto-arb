#![allow(clippy::panic)]

use super::*;

#[test]
fn secret_backend_env_values_are_tolerant() {
    assert_eq!(
        SecretBackend::from_env_value("keychain"),
        SecretBackend::Keychain
    );
    assert_eq!(
        SecretBackend::from_env_value("macos-keychain"),
        SecretBackend::Keychain
    );
    assert_eq!(
        SecretBackend::from_env_value("runtime_only"),
        SecretBackend::RuntimeOnly
    );
    assert_eq!(
        SecretBackend::from_env_value("unknown"),
        SecretBackend::EnvFileAtomic
    );
}

#[test]
fn keychain_status_claims_encrypted_persistence_in_tests() {
    let status = status_for_backend(SecretBackend::Keychain);

    assert_eq!(status.mode, shared_types::SecretStorageMode::Keychain);
    assert_eq!(status.health, shared_types::SecretStorageHealth::Ready);
    assert!(status.persistent);
    assert!(status.encrypted);
}

#[test]
fn secret_backend_read_failure_is_visible_in_storage_status() {
    record_backend_error(
        SecretBackend::Keychain,
        &CredentialUpdateError::SecretBackend("keychain locked".to_owned()),
    );

    let status = apply_backend_warning(
        SecretBackend::Keychain,
        status_for_backend(SecretBackend::Keychain),
    );

    assert!(status
        .warning
        .as_deref()
        .is_some_and(|warning| warning.contains("keychain locked")));
    assert_eq!(
        status.health,
        shared_types::SecretStorageHealth::Unavailable
    );
    assert!(status
        .last_error
        .as_deref()
        .is_some_and(|error| error.contains("keychain locked")));
    assert!(status.message.contains("读写异常"));
    clear_backend_error(SecretBackend::Keychain);
}

#[tokio::test]
async fn clear_masks_runtime_value() {
    let key = "CROSSLINE_TEST_CLEAR_MASK";
    persist_updates_for_backend(
        SecretBackend::RuntimeOnly,
        &[(key.to_owned(), "runtime-value".to_owned())],
    )
    .unwrap_or_else(|error| panic!("runtime save failed: {error}"));
    assert_eq!(source(key), VenueCredentialFieldSource::Runtime);

    clear_fields(&[key.to_owned()])
        .await
        .unwrap_or_else(|error| panic!("clear failed: {error}"));

    assert_eq!(secret(key), None);
    assert_eq!(source(key), VenueCredentialFieldSource::Missing);
}

#[tokio::test]
async fn migrate_persists_present_fields_and_reports_missing_keys() {
    let present = "CROSSLINE_TEST_MIGRATE_PRESENT";
    let missing = "CROSSLINE_TEST_MIGRATE_MISSING";
    persist_updates_for_backend(
        SecretBackend::RuntimeOnly,
        &[(present.to_owned(), "runtime-value".to_owned())],
    )
    .unwrap_or_else(|error| panic!("runtime save failed: {error}"));

    let result = migrate_fields(&[present.to_owned(), missing.to_owned()]).await;

    assert!(result.is_ok(), "migration failed: {result:?}");
    let result = result.unwrap_or_else(|error| panic!("migration failed: {error}"));
    assert_eq!(result.migrated_keys, vec![present]);
    assert_eq!(result.missing_keys, vec![missing]);
    assert_eq!(source(present), VenueCredentialFieldSource::EnvFile);
}

#[tokio::test]
async fn keychain_clear_removes_dotenv_fallback_before_restart() {
    let key = "CROSSLINE_TEST_KEYCHAIN_CLEAR_FALLBACK";
    let path = temp_dotenv_path("keychain-clear");
    std::fs::write(&path, format!("{key}=dotenv-value\nKEEP_ME=present\n"))
        .unwrap_or_else(|error| panic!("dotenv fixture failed: {error}"));
    persist_updates_for_backend(
        SecretBackend::Keychain,
        &[(key.to_owned(), "keychain-value".to_owned())],
    )
    .unwrap_or_else(|error| panic!("keychain save failed: {error}"));

    clear_fields_with_backend(SecretBackend::Keychain, &[key.to_owned()], Some(&path))
        .await
        .unwrap_or_else(|error| panic!("keychain clear failed: {error}"));

    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("dotenv read failed: {error}"));
    assert_eq!(text, "KEEP_ME=present\n");
    assert_eq!(secret_for_backend(SecretBackend::Keychain, key), None);
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn keychain_migration_removes_plaintext_dotenv_source() {
    let key = "CROSSLINE_TEST_KEYCHAIN_MIGRATE_FALLBACK";
    let path = temp_dotenv_path("keychain-migrate");
    std::fs::write(&path, format!("{key}=dotenv-value\nKEEP_ME=present\n"))
        .unwrap_or_else(|error| panic!("dotenv fixture failed: {error}"));
    remember_runtime(
        &[(key.to_owned(), "dotenv-value".to_owned())],
        VenueCredentialFieldSource::Environment,
    );

    let result = migrate_fields_to_backend(SecretBackend::Keychain, &[key.to_owned()], Some(&path))
        .await
        .unwrap_or_else(|error| panic!("keychain migration failed: {error}"));

    assert_eq!(result.migrated_keys, vec![key]);
    let source = source_for_current_backend(SecretBackend::Keychain, key)
        .unwrap_or_else(|error| panic!("keychain source failed: {error}"));
    assert_eq!(source, VenueCredentialFieldSource::Keychain);
    assert_eq!(
        secret_for_backend(SecretBackend::Keychain, key).as_deref(),
        Some("dotenv-value")
    );
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("dotenv read failed: {error}"));
    assert_eq!(text, "KEEP_ME=present\n");

    clear_fields_with_backend(SecretBackend::Keychain, &[key.to_owned()], Some(&path))
        .await
        .unwrap_or_else(|error| panic!("keychain cleanup failed: {error}"));
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn keychain_clear_restores_secret_when_dotenv_cleanup_fails() {
    let key = "CROSSLINE_TEST_KEYCHAIN_CLEAR_CLEANUP_FAILURE";
    let path = temp_dotenv_path("keychain-clear-failure");
    std::fs::create_dir(&path)
        .unwrap_or_else(|error| panic!("dotenv directory fixture failed: {error}"));
    persist_updates_for_backend(
        SecretBackend::Keychain,
        &[(key.to_owned(), "keychain-value".to_owned())],
    )
    .unwrap_or_else(|error| panic!("keychain save failed: {error}"));

    let result =
        clear_fields_with_backend(SecretBackend::Keychain, &[key.to_owned()], Some(&path)).await;

    assert!(result.is_err(), "dotenv cleanup failure must be visible");
    assert_eq!(secret_for_backend(SecretBackend::Keychain, key).as_deref(), Some("keychain-value"));
    assert_eq!(keychain::secret(key).ok().flatten().as_deref(), Some("keychain-value"));
    clear_backend_error(SecretBackend::Keychain);
    let _ = std::fs::remove_dir(path);
}

fn temp_dotenv_path(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "crossline-credential-{label}-{}-{}.env",
        std::process::id(),
        common::time::now_ms()
    ))
}
