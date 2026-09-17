use super::super::credential_maintenance::{
    credential_maintenance_replay_key, CredentialMaintenanceReplay,
};
use super::super::format::credential_maintenance_success_message;
use shared_types::{VenueCredentialMaintenanceOperation, VenueCredentialMaintenanceResponse};

#[test]
fn credential_maintenance_replay_key_reuses_only_the_same_operation() {
    let slot = CredentialMaintenanceReplay {
        fingerprint: "clear:okx:api_key".into(),
        key: "idem-clear".into(),
    };

    assert_eq!(
        credential_maintenance_replay_key(Some(slot.clone()), "clear:okx:api_key"),
        "idem-clear"
    );
    assert_ne!(
        credential_maintenance_replay_key(Some(slot), "migrate:okx"),
        "idem-clear"
    );
}

#[test]
fn credential_maintenance_success_message_keeps_audit_and_health_context() {
    let message = credential_maintenance_success_message(
        &VenueCredentialMaintenanceResponse {
            venue: "okx".into(),
            label: "OKX".into(),
            operation: VenueCredentialMaintenanceOperation::Clear,
            affected_fields: vec!["api_key".into()],
            missing_fields: vec!["api_key".into()],
            message: "已清空字段".into(),
            secret_storage: shared_types::SecretStorageStatus::keychain("test"),
            action_run_id: Some("act-clear".into()),
            request_id: Some("req-clear".into()),
        },
        "idem-clear",
    );

    assert!(message.contains("已清空"));
    assert!(message.contains("缺失 api_key"));
    assert!(message.contains("act-clear"));
    assert!(message.contains("req-clear"));
    assert!(message.contains("系统 Keychain 加密"));
    assert!(message.contains("idem-clear"));
}
