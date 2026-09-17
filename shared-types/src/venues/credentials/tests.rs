use super::*;

#[test]
fn credential_probe_request_id_is_backward_compatible() {
    let probe: VenueCredentialProbe = serde_json::from_str(
        r#"{
            "kind":"order_permission",
            "status":"unknown",
            "scope":"place_cancel_order_stream",
            "source":"not_probed",
            "message":"not proven",
            "checkedAtMs":10
        }"#,
    )
    .expect("legacy probe decodes");

    assert_eq!(probe.request_id, None);
}

#[test]
fn credential_probe_serializes_request_id_when_present() {
    let probe = VenueCredentialProbe {
        kind: "order_permission".to_owned(),
        status: VenueCredentialProbeStatus::Unknown,
        scope: "place_cancel_order_stream".to_owned(),
        source: "credential_validation".to_owned(),
        message: "not proven".to_owned(),
        checked_at_ms: 10,
        request_id: Some("req-credential".to_owned()),
    };

    let json = serde_json::to_value(&probe).expect("probe serializes");

    assert_eq!(json["requestId"], "req-credential");
}

#[test]
fn legacy_validation_evidence_defaults_permission_matrix_to_empty() {
    let evidence: VenueCredentialValidationEvidence = serde_json::from_str(
        r#"{
            "status":"read_only_ok",
            "checkedAtMs":10,
            "probes":[]
        }"#,
    )
    .expect("legacy validation evidence decodes");

    assert!(evidence.permission_evidence.is_empty());
}

#[test]
fn scoped_permission_projection_preserves_denial_and_request_evidence() {
    let evidence = VenueCredentialValidationEvidence {
        status: VenueCredentialValidationStatus::ReadOnlyOk,
        checked_at_ms: 10,
        probes: vec![
            VenueCredentialProbe {
                kind: "open_orders_read".to_owned(),
                status: VenueCredentialProbeStatus::Ok,
                scope: "private_read.open_orders".to_owned(),
                source: "exchange_adapter.get_open_orders".to_owned(),
                message: "read-only open orders probe succeeded".to_owned(),
                checked_at_ms: 10,
                request_id: Some("req-read".to_owned()),
            },
            VenueCredentialProbe {
                kind: "order_permission".to_owned(),
                status: VenueCredentialProbeStatus::Failed,
                scope: "cancel_no_match".to_owned(),
                source: "venue.cancel".to_owned(),
                message: "cancel permission denied".to_owned(),
                checked_at_ms: 11,
                request_id: Some("req-cancel".to_owned()),
            },
        ],
        permission_evidence: Vec::new(),
    }
    .with_order_permission_scopes(&[VenueCredentialPermission::CancelOrder]);

    let open_orders = evidence
        .permission(VenueCredentialPermission::OpenOrdersRead)
        .expect("open-orders evidence");
    assert_eq!(
        open_orders.status,
        VenueCredentialPermissionStatus::Validated
    );
    assert!(open_orders.status.is_validated());
    assert_eq!(open_orders.permission_scope, "open_orders_read");
    assert_eq!(open_orders.request_id.as_deref(), Some("req-read"));

    let place = evidence
        .permission(VenueCredentialPermission::PlaceOrder)
        .expect("place-order evidence");
    assert_eq!(place.status, VenueCredentialPermissionStatus::Missing);

    let cancel = evidence
        .permission(VenueCredentialPermission::CancelOrder)
        .expect("cancel-order evidence");
    assert_eq!(cancel.status, VenueCredentialPermissionStatus::Denied);
    assert_eq!(cancel.permission_scope, "cancel_order");
    assert_eq!(cancel.request_id.as_deref(), Some("req-cancel"));
    assert_eq!(cancel.error.as_deref(), Some("cancel permission denied"));
}

#[test]
fn credential_field_source_defaults_to_missing_for_legacy_payloads() {
    let field: VenueCredentialField = serde_json::from_str(
        r#"{
            "key":"api_key",
            "label":"API Key",
            "envKey":"OKX_API_KEY",
            "configured":false,
            "secret":true
        }"#,
    )
    .expect("legacy field decodes");

    assert_eq!(field.source, VenueCredentialFieldSource::Missing);
    assert!(field.required);
}

#[test]
fn legacy_secret_storage_status_defaults_to_unknown_health() {
    let status: SecretStorageStatus = serde_json::from_str(
        r#"{
            "mode":"runtime_only",
            "persistent":false,
            "encrypted":false,
            "atomicWrite":false,
            "label":"legacy",
            "message":"legacy runtime storage"
        }"#,
    )
    .expect("legacy storage status decodes");

    assert_eq!(status.health, SecretStorageHealth::Unknown);
    assert_eq!(status.last_error, None);
}

#[test]
fn secret_storage_constructors_report_typed_health() {
    assert_eq!(
        SecretStorageStatus::keychain("crossline").health,
        SecretStorageHealth::Ready
    );
    assert_eq!(
        SecretStorageStatus::env_file_atomic(None).health,
        SecretStorageHealth::Degraded
    );
    assert_eq!(
        SecretStorageStatus::runtime_only().health,
        SecretStorageHealth::Degraded
    );
    assert_eq!(
        SecretStorageStatus::keychain_unavailable("crossline").health,
        SecretStorageHealth::Unavailable
    );
}

#[test]
fn backend_error_is_typed_and_serialized_without_legacy_placeholders() {
    let status = SecretStorageStatus::keychain("crossline").with_backend_error("keychain locked");
    let json = serde_json::to_value(&status).expect("storage status serializes");

    assert_eq!(status.health, SecretStorageHealth::Unavailable);
    assert_eq!(status.last_error.as_deref(), Some("keychain locked"));
    assert_eq!(json["health"], "unavailable");
    assert_eq!(json["lastError"], "keychain locked");
}
