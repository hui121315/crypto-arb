use super::*;

#[test]
fn ws_ready_label_is_static_implementation_status() {
    assert_eq!(status_label(ExchangeWsSupportStatus::Ready), "静态实现");
}

#[test]
fn legacy_beta_release_label_is_explicitly_unavailable() {
    let operation = ExchangeWsOperation {
        supported: true,
        status: ExchangeWsSupportStatus::SchemaPending,
        operation: Some("futures.order".to_owned()),
        product: "Futures".to_owned(),
        note: "Beta 不可用：官方 Pro API 禁止生产交易；缺运行态证据".to_owned(),
        evidence: Some(shared_types::ExchangeWsOperationEvidence {
            release_status: ExchangeWsReleaseStatus::BetaUnavailable,
            requires_authenticated_runtime_evidence: false,
            authenticated_runtime_evidence: false,
            checked_at: "2026-07-11".to_owned(),
            doc_version: "kucoin-pro-ws-beta-2026-07-11".to_owned(),
            doc_url: "https://www.kucoin.com/docs-new/change-log".to_owned(),
            parser_test: Some("parser".to_owned()),
            subscription_test: Some("request".to_owned()),
            fixture_id: None,
            fixture_hash: None,
            auth_kind: "signed_wsapi_query".to_owned(),
        }),
    };

    assert_eq!(ws_status_label(&operation), "Beta 不可用");
    assert!(!operation.is_live_submittable());
}

#[test]
fn ws_transport_labels_distinguish_stream_write_and_runtime_gate() {
    let mut operation = ExchangeWsOperation {
        supported: true,
        status: ExchangeWsSupportStatus::Ready,
        operation: Some("order.place".to_owned()),
        product: "Futures".to_owned(),
        note: "typed operation".to_owned(),
        evidence: Some(shared_types::ExchangeWsOperationEvidence {
            release_status: ExchangeWsReleaseStatus::ProductionReady,
            requires_authenticated_runtime_evidence: false,
            authenticated_runtime_evidence: false,
            checked_at: "2026-07-27".to_owned(),
            doc_version: "production".to_owned(),
            doc_url: "https://example.invalid/official-fixture".to_owned(),
            parser_test: Some("parser".to_owned()),
            subscription_test: Some("request".to_owned()),
            fixture_id: None,
            fixture_hash: None,
            auth_kind: "signed".to_owned(),
        }),
    };

    assert_eq!(
        ws_transport_label(&operation, false),
        "WS 实时流；REST 仅冷启动、断线补洞与历史查询"
    );
    assert_eq!(
        ws_transport_label(&operation, true),
        "WS 提交主路径；ACK 不确定时仅按 client id 对账，禁止 REST 重放"
    );

    operation.status = ExchangeWsSupportStatus::RequiresPermission;
    if let Some(evidence) = operation.evidence.as_mut() {
        evidence.requires_authenticated_runtime_evidence = true;
    }
    assert!(operation
        .evidence
        .as_ref()
        .is_some_and(|evidence| { evidence.requires_authenticated_runtime_evidence }));
    assert_eq!(
        ws_transport_label(&operation, true),
        "REST 单次提交；官方 WS 已发布但等待认证运行证据"
    );

    operation.status = ExchangeWsSupportStatus::RestOnly;
    operation.evidence = None;
    assert_eq!(
        ws_transport_label(&operation, true),
        "REST 单次提交；WS 写路径未获生产授权"
    );
}

#[test]
fn credential_field_label_never_claims_validation() {
    assert_eq!(credential_field_label(true), "字段已填写");
    assert_eq!(credential_field_label(false), "字段未填写");
}

#[test]
fn credential_fields_dataset_key_tracks_field_state() {
    let mut fields = okx_credential_status().fields;
    let baseline = credential_fields_dataset_key("okx", &fields);

    fields[0].configured = true;
    let changed = credential_fields_dataset_key("okx", &fields);

    assert_ne!(baseline, changed);
    assert_ne!(baseline, credential_fields_dataset_key("binance", &fields));
}

#[test]
fn values_from_keeps_okx_live_and_normal_credentials_distinct() {
    let spec = okx_credential_status();
    let drafts = vec![
        draft("api_key", "normal-key"),
        draft("live_key", "live-key"),
        draft("api_secret", "normal-secret"),
        draft("live_secret", "live-secret"),
        draft("passphrase", "normal-pass"),
        draft("live_passphrase", "live-pass"),
    ];

    let values = values_from(&spec, &drafts);

    assert!(values.contains(&("api_key".to_owned(), "normal-key".to_owned())));
    assert!(values.contains(&("live_key".to_owned(), "live-key".to_owned())));
    assert!(values.contains(&("api_secret".to_owned(), "normal-secret".to_owned())));
    assert!(values.contains(&("live_secret".to_owned(), "live-secret".to_owned())));
    assert!(values.contains(&("passphrase".to_owned(), "normal-pass".to_owned())));
    assert!(values.contains(&("live_passphrase".to_owned(), "live-pass".to_owned())));
}
