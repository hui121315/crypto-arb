use super::dotenv::{
    dotenv_value, persist_fields_to_dotenv_path, remove_dotenv_fields,
    remove_fields_from_dotenv_path, upsert_dotenv_text, write_dotenv_atomic,
};
use super::*;

#[path = "tests/status.rs"]
mod venue_status;

#[allow(clippy::panic)]
fn fail(message: &str) -> ! {
    panic!("{message}");
}

#[tokio::test]
async fn update_marks_runtime_credentials_configured() {
    let result = update(VenueCredentialUpdateRequest {
        venue: "BYBIT".into(),
        fields: vec![VenueCredentialValue {
            key: "api_key".into(),
            value: "test-key".into(),
        }],
    })
    .await;
    assert!(result.is_ok(), "{result:?}");
    let Ok(response) = result else { return };

    assert_eq!(response.venue, "bybit");
    assert_eq!(response.configured_count, 1);
    assert_eq!(
        response.secret_storage.mode,
        shared_types::SecretStorageMode::EnvFileAtomic
    );
    assert!(response.secret_storage.persistent);
    assert!(!response.secret_storage.encrypted);
    assert!(response.validation_evidence.is_some());
    assert!(response
        .validation_evidence
        .as_ref()
        .is_some_and(|evidence| evidence
            .probes
            .iter()
            .any(|probe| probe.kind == "order_permission"
                && probe.source == "credential_save.order_permission_unproven.bybit")));
    assert!(response
        .validation_evidence
        .as_ref()
        .is_some_and(|evidence| evidence.probes.iter().any(|probe| {
            probe.kind == "account_mode_read"
                && probe.source == "not_probed"
                && probe.status == shared_types::VenueCredentialProbeStatus::Unknown
        })));
    assert!(response
        .validation_evidence
        .as_ref()
        .is_some_and(|evidence| evidence
            .probes
            .iter()
            .any(|probe| probe.kind == "positions_read"
                && probe.status == shared_types::VenueCredentialProbeStatus::Unknown)));
    assert!(response
        .validation_evidence
        .as_ref()
        .is_some_and(|evidence| evidence
            .probes
            .iter()
            .any(|probe| probe.kind == "open_orders_read"
                && probe.status == shared_types::VenueCredentialProbeStatus::Unknown)));
    let validation_snapshots = validation_evidence_snapshot();
    assert!(validation_snapshots.iter().any(|snapshot| {
        snapshot.venue == "bybit"
            && snapshot.evidence.probes.iter().any(|probe| {
                probe.kind == "order_permission"
                    && probe.source == "credential_save.order_permission_unproven.bybit"
            })
    }));
    assert!(validation_snapshots.iter().any(|snapshot| {
        snapshot.venue == "bybit"
            && snapshot.evidence.probes.iter().any(|probe| {
                probe.kind == "account_mode_read"
                    && probe.source == "not_probed"
                    && probe.status == shared_types::VenueCredentialProbeStatus::Unknown
            })
    }));
    let status = status();
    assert_eq!(
        status.secret_storage.mode,
        shared_types::SecretStorageMode::EnvFileAtomic
    );
    assert!(status.venues.iter().any(|venue| {
        venue.venue == "bybit"
            && venue
                .fields
                .iter()
                .any(|field| field.key == "api_key" && field.configured)
            && venue.validation_evidence.as_ref().is_some_and(|evidence| {
                evidence.probes.iter().any(|probe| {
                    probe.kind == "order_permission"
                        && probe.source == "credential_save.order_permission_unproven.bybit"
                })
            })
            && venue.validation_evidence.as_ref().is_some_and(|evidence| {
                evidence.probes.iter().any(|probe| {
                    probe.kind == "account_mode_read"
                        && probe.source == "not_probed"
                        && probe.status == shared_types::VenueCredentialProbeStatus::Unknown
                })
            })
    }));
}

#[tokio::test]
async fn update_accepts_builder_venue_family_alias() {
    let result = update(VenueCredentialUpdateRequest {
        venue: " Hyperliquid:XYZ ".into(),
        fields: vec![VenueCredentialValue {
            key: "account_address".into(),
            value: "0x53fde8e60d9164647051193a43ac59b138c41305".into(),
        }],
    })
    .await;
    assert!(result.is_ok(), "{result:?}");
    let Ok(response) = result else { return };

    assert_eq!(response.venue, "hyperliquid");
    assert_eq!(response.configured_count, 1);
}

#[test]
fn keychain_backend_reports_encrypted_and_reads_saved_secret() {
    let updates = vec![("OKX_API_KEY".to_owned(), "keychain-test-key".to_owned())];

    storage::persist_updates_for_backend(storage::SecretBackend::Keychain, &updates)
        .unwrap_or_else(|error| fail(&format!("{error:?}")));

    let keychain_secret =
        keychain::secret("OKX_API_KEY").unwrap_or_else(|error| fail(&format!("{error:?}")));
    assert_eq!(keychain_secret.as_deref(), Some("keychain-test-key"));
    assert_eq!(
        storage::secret_for_backend(storage::SecretBackend::Keychain, "OKX_API_KEY").as_deref(),
        Some("keychain-test-key")
    );
}

#[tokio::test]
async fn env_template_never_exposes_secret_values() {
    let result = update(VenueCredentialUpdateRequest {
        venue: "gate".into(),
        fields: vec![VenueCredentialValue {
            key: "api_secret".into(),
            value: "super-secret".into(),
        }],
    })
    .await;
    assert!(result.is_ok(), "{result:?}");

    let template = env_template();

    assert!(template.text.contains("GATE_API_SECRET="));
    assert!(!template.text.contains("super-secret"));
}

#[test]
fn dotenv_upsert_preserves_comments_and_updates_keys() {
    let original = "# creds\nOKX_API_KEY=old\n\nBYBIT_API_SECRET=keep\n";
    let updated = upsert_dotenv_text(
        original,
        &[
            ("OKX_API_KEY".into(), "new-key".into()),
            ("OKX_API_SECRET".into(), "sec ret#1".into()),
        ],
    ).unwrap_or_else(|error| fail(&format!("{error:?}")));

    assert!(updated.contains("# creds\nOKX_API_KEY=new-key"));
    assert!(updated.contains("BYBIT_API_SECRET=keep"));
    assert!(updated.contains("OKX_API_SECRET=\"sec ret#1\""));
}

#[test]
fn dotenv_value_escapes_multiline_secret() {
    let value = dotenv_value("a\"b\\c\n");

    assert_eq!(value, "\"a\\\"b\\\\c\\n\"");
}

#[test]
fn atomic_dotenv_write_replaces_file_and_removes_temp_artifact() {
    let dir = tempfile::tempdir().unwrap_or_else(|error| fail(&format!("{error:?}")));
    let path = dir.path().join(".env");
    std::fs::write(&path, "OLD=value\n").unwrap_or_else(|error| fail(&format!("{error:?}")));

    write_dotenv_atomic(&path, "NEW=value\n").unwrap_or_else(|error| fail(&format!("{error:?}")));

    let text = std::fs::read_to_string(&path).unwrap_or_else(|error| fail(&format!("{error:?}")));
    assert_eq!(text, "NEW=value\n");
    let temp_left = std::fs::read_dir(dir.path())
        .unwrap_or_else(|error| fail(&format!("{error:?}")))
        .filter_map(Result::ok)
        .any(|entry| entry.file_name().to_string_lossy().contains(".tmp"));
    assert!(!temp_left);
}

#[test]
fn dotenv_path_persistence_merges_existing_and_new_fields() {
    let dir = tempfile::tempdir().unwrap_or_else(|error| fail(&format!("{error:?}")));
    let path = dir.path().join(".env");
    std::fs::write(&path, "OKX_API_KEY=old\nBYBIT_API_SECRET=keep\n")
        .unwrap_or_else(|error| fail(&format!("{error:?}")));

    persist_fields_to_dotenv_path(
        &path,
        &[
            ("OKX_API_KEY".into(), "new".into()),
            ("GATE_API_KEY".into(), "gate-key".into()),
        ],
    )
    .unwrap_or_else(|error| fail(&format!("{error:?}")));

    let text = std::fs::read_to_string(&path).unwrap_or_else(|error| fail(&format!("{error:?}")));
    assert!(text.contains("OKX_API_KEY=new"));
    assert!(text.contains("BYBIT_API_SECRET=keep"));
    assert!(text.contains("GATE_API_KEY=gate-key"));
}

#[test]
fn dotenv_field_removal_preserves_unrelated_lines() {
    let original = "# creds\nOKX_API_KEY=old\nBYBIT_API_SECRET=keep\n";
    let updated = remove_dotenv_fields(original, &["OKX_API_KEY".to_owned()])
        .unwrap_or_else(|error| fail(&format!("{error:?}")));

    assert!(updated.contains("# creds"));
    assert!(!updated.contains("OKX_API_KEY"));
    assert!(updated.contains("BYBIT_API_SECRET=keep"));
}

#[test]
fn dotenv_path_removal_is_atomic_and_leaves_other_credentials_intact() {
    let dir = tempfile::tempdir().unwrap_or_else(|error| fail(&format!("{error:?}")));
    let path = dir.path().join(".env");
    std::fs::write(&path, "OKX_API_KEY=old\nBYBIT_API_SECRET=keep\n")
        .unwrap_or_else(|error| fail(&format!("{error:?}")));

    remove_fields_from_dotenv_path(&path, &["OKX_API_KEY".to_owned()])
        .unwrap_or_else(|error| fail(&format!("{error:?}")));

    let text = std::fs::read_to_string(&path).unwrap_or_else(|error| fail(&format!("{error:?}")));
    assert!(!text.contains("OKX_API_KEY"));
    assert!(text.contains("BYBIT_API_SECRET=keep"));
}

#[cfg(unix)]
#[test]
fn atomic_dotenv_write_sets_secret_file_permissions_best_effort() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap_or_else(|error| fail(&format!("{error:?}")));
    let path = dir.path().join(".env");

    write_dotenv_atomic(&path, "SECRET=value\n")
        .unwrap_or_else(|error| fail(&format!("{error:?}")));

    let mode = std::fs::metadata(&path)
        .unwrap_or_else(|error| fail(&format!("{error:?}")))
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);
}
