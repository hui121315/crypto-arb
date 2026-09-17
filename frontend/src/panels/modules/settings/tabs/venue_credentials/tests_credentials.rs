use super::*;

#[path = "tests_credentials/load_state.rs"]
mod load_state;
#[path = "tests_credentials/secret_storage.rs"]
mod secret_storage;
#[path = "tests_credentials/summary.rs"]
mod summary;
#[path = "tests_credentials/ws.rs"]
mod ws;

#[test]
fn initial_venue_selection_preserves_recovered_action_state() {
    assert!(!credential_selection_changed(None, "okx"));
    assert!(!credential_selection_changed(Some(""), "okx"));
    assert!(!credential_selection_changed(Some("okx"), "okx"));
    assert!(credential_selection_changed(Some("okx"), "binance"));
}

fn credential_status(live_write: bool) -> VenueCredentialStatus {
    VenueCredentialStatus {
        venue: "okx".into(),
        label: "OKX".into(),
        fields: vec![VenueCredentialField {
            key: "api_key".into(),
            label: "API Key".into(),
            env_key: "OKX_API_KEY".into(),
            configured: true,
            secret: true,
            required: true,
            source: shared_types::VenueCredentialFieldSource::Runtime,
        }],
        public_market: true,
        private_read: true,
        testnet_write: false,
        live_write,
        note: "test".into(),
        missing_fields: Vec::new(),
        validation_evidence: None,
    }
}

fn okx_credential_status() -> VenueCredentialStatus {
    let fields = [
        ("api_key", "API Key", "OKX_API_KEY"),
        ("api_secret", "API Secret", "OKX_API_SECRET"),
        ("passphrase", "Passphrase", "OKX_PASSPHRASE"),
        ("live_key", "实盘 API Key", "OKX_LIVE_API_KEY"),
        ("live_secret", "实盘 API Secret", "OKX_LIVE_API_SECRET"),
        ("live_passphrase", "实盘 Passphrase", "OKX_LIVE_PASSPHRASE"),
    ]
    .into_iter()
    .map(|(key, label, env_key)| VenueCredentialField {
        key: key.to_owned(),
        label: label.to_owned(),
        env_key: env_key.to_owned(),
        configured: false,
        secret: true,
        required: true,
        source: shared_types::VenueCredentialFieldSource::Missing,
    })
    .collect();
    VenueCredentialStatus {
        venue: "okx".into(),
        label: "OKX".into(),
        fields,
        public_market: true,
        private_read: true,
        testnet_write: false,
        live_write: true,
        note: "test".into(),
        missing_fields: vec![
            "api_key".into(),
            "api_secret".into(),
            "passphrase".into(),
            "live_key".into(),
            "live_secret".into(),
            "live_passphrase".into(),
        ],
        validation_evidence: None,
    }
}

fn draft(key: &str, value: &str) -> CredentialDraftValue {
    CredentialDraftValue {
        key: key.to_owned(),
        value: value.to_owned(),
    }
}

fn validation_evidence(
    probes: Vec<shared_types::VenueCredentialProbe>,
) -> shared_types::VenueCredentialValidationEvidence {
    shared_types::VenueCredentialValidationEvidence {
        status: shared_types::VenueCredentialValidationStatus::ReadOnlyOk,
        checked_at_ms: 42,
        probes,
        permission_evidence: Vec::new(),
    }
}

fn validation_probe(
    kind: &str,
    status: shared_types::VenueCredentialProbeStatus,
) -> shared_types::VenueCredentialProbe {
    shared_types::VenueCredentialProbe {
        kind: kind.to_owned(),
        status,
        scope: "test".to_owned(),
        source: "test".to_owned(),
        message: "test".to_owned(),
        checked_at_ms: 42,
        request_id: None,
    }
}
