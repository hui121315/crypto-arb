use super::*;

#[test]
fn selected_summary_describes_static_write_support() {
    let status = credential_status(true);
    let summary = selected_summary(Some(&status));

    assert!(summary.contains("字段已填写"));
    assert!(summary.contains("静态写侧声明"));
    assert!(summary.contains("未验证"));
    assert!(summary.contains("当前状态待运行态证据"));
    assert!(summary.contains("字段完整"));
    assert!(!summary.contains("已配置"));
    assert!(!summary.contains("实盘写入"));
    assert!(!summary.contains("当前可用"));
}

#[test]
fn credential_field_source_labels_are_explicit() {
    assert_eq!(
        credential_field_source_label(shared_types::VenueCredentialFieldSource::Keychain),
        "Keychain"
    );
    assert_eq!(
        credential_field_source_label(shared_types::VenueCredentialFieldSource::Missing),
        "缺失"
    );
}

#[test]
fn selected_summary_surfaces_saved_validation_evidence_fail_closed() {
    let mut status = credential_status(true);
    status.validation_evidence = Some(validation_evidence(vec![
        validation_probe("balance_read", shared_types::VenueCredentialProbeStatus::Ok),
        validation_probe(
            "positions_read",
            shared_types::VenueCredentialProbeStatus::Ok,
        ),
        validation_probe(
            "open_orders_read",
            shared_types::VenueCredentialProbeStatus::Ok,
        ),
        validation_probe(
            "order_permission",
            shared_types::VenueCredentialProbeStatus::Unknown,
        ),
    ]));

    let summary = selected_summary(Some(&status));

    assert!(summary.contains("只读验证"));
    assert!(summary.contains("未验证（缺少探针）"));
    assert!(summary.contains("订单权限"));
    assert!(summary.contains("账户模式"));
    assert!(!summary.contains("已验证（保存期）"));
    assert!(!summary.contains("当前可用"));
}

#[test]
fn selected_summary_separates_saved_validation_from_current_availability() {
    let mut status = credential_status(true);
    status.validation_evidence = Some(validation_evidence(
        [
            "balance_read",
            "positions_read",
            "open_orders_read",
            "order_permission",
            "account_mode_read",
        ]
        .into_iter()
        .map(|kind| validation_probe(kind, shared_types::VenueCredentialProbeStatus::Ok))
        .collect(),
    ));

    let summary = selected_summary(Some(&status));

    assert!(summary.contains("字段已填写"));
    assert!(summary.contains("已验证（保存期）"));
    assert!(summary.contains("当前状态待运行态证据"));
    assert!(!summary.contains("当前可用"));
}
