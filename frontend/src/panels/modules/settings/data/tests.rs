//! Settings data 层单测（落态保留 / 成功文案 / 幂等重放键）。

mod api_base;
mod credential_maintenance;

use super::actions::{
    credential_save_fingerprint, credential_save_replay_key, risk_config_replay_slot,
    should_reuse_credential_replay_key, CredentialSaveReplay, RiskConfigReplay,
};
use super::format::credential_success_message;
use super::resources::{
    apply_settings_result, mark_action_run_detail_loading, scoped_venue_health_keeps_value,
};
use crate::api::rest::ApiError;
use crate::state::load_state::LoadState;
use shared_types::{
    problem::codes, ActionRun, VenueCredentialUpdateResponse, VenueCredentialValidationStatus,
    VenueOperationHealthSnapshot,
};

#[test]
fn timeout_error_uses_typed_problem() {
    let error = ApiError::client("TIMEOUT", "保存超时：请检查 API Base 或后端服务");

    assert_eq!(error.problem.code, "TIMEOUT");
    assert_eq!(error.problem.source.as_deref(), Some("frontend"));
}

#[test]
fn settings_error_after_ready_preserves_stale_value() {
    let mut state = LoadState::Ready(42_u64);

    apply_settings_result(
        &mut state,
        Err(ApiError::client("NETWORK", "后端暂时不可用")),
    );

    assert_eq!(state.value(), Some(&42));
    assert_eq!(
        state.problem().map(|problem| problem.code.as_str()),
        Some("NETWORK")
    );
}

#[test]
fn same_scoped_venue_refresh_keeps_snapshot_for_stale_error() {
    let snapshot = VenueOperationHealthSnapshot::new(Vec::new(), 10);
    let mut state = LoadState::Ready(snapshot);

    assert!(scoped_venue_health_keeps_value(&state, Some("okx"), "okx"));

    apply_settings_result(
        &mut state,
        Err(ApiError::client("RATE_LIMITED", "runtime health slow")),
    );

    assert!(state.value().is_some());
    assert_eq!(
        state.problem().map(|problem| problem.code.as_str()),
        Some("RATE_LIMITED")
    );
}

#[test]
fn changed_scoped_venue_refresh_clears_previous_snapshot() {
    let snapshot = VenueOperationHealthSnapshot::new(Vec::new(), 10);
    let state = LoadState::Ready(snapshot);

    assert!(!scoped_venue_health_keeps_value(
        &state,
        Some("okx"),
        "binance"
    ));
}

#[test]
fn credential_success_message_keeps_business_result_out_of_technical_ids() {
    let message = credential_success_message(&VenueCredentialUpdateResponse {
        venue: "okx".into(),
        label: "OKX".into(),
        configured_count: 3,
        field_count: 3,
        message: "已保存".into(),
        secret_storage: shared_types::SecretStorageStatus::env_file_atomic(Some(
            "/tmp/.env".into(),
        )),
        validation_evidence: Some(shared_types::VenueCredentialValidationEvidence {
            status: VenueCredentialValidationStatus::ReadOnlyOk,
            checked_at_ms: 10,
            probes: vec![
                shared_types::VenueCredentialProbe {
                    kind: "balance_read".into(),
                    status: shared_types::VenueCredentialProbeStatus::Ok,
                    scope: "USDT".into(),
                    source: "exchange_adapter.get_balance".into(),
                    message: "ok".into(),
                    checked_at_ms: 10,
                    request_id: None,
                },
                shared_types::VenueCredentialProbe {
                    kind: "positions_read".into(),
                    status: shared_types::VenueCredentialProbeStatus::Ok,
                    scope: "private_read.positions".into(),
                    source: "exchange_adapter.get_positions".into(),
                    message: "ok".into(),
                    checked_at_ms: 10,
                    request_id: None,
                },
                shared_types::VenueCredentialProbe {
                    kind: "open_orders_read".into(),
                    status: shared_types::VenueCredentialProbeStatus::Unknown,
                    scope: "private_read.open_orders".into(),
                    source: "exchange_adapter.get_open_orders".into(),
                    message: "not proven".into(),
                    checked_at_ms: 10,
                    request_id: None,
                },
                shared_types::VenueCredentialProbe {
                    kind: "order_permission".into(),
                    status: shared_types::VenueCredentialProbeStatus::Unknown,
                    scope: "place_cancel_order_stream".into(),
                    source: "not_probed".into(),
                    message: "not probed".into(),
                    checked_at_ms: 10,
                    request_id: None,
                },
            ],
            permission_evidence: Vec::new(),
        }),
        action_run_id: Some("act-1".into()),
        request_id: Some("req-1".into()),
    });

    assert!(!message.contains("act-1"));
    assert!(!message.contains("req-1"));
    assert!(message.contains("只读接口通过"));
    assert!(message.contains("私有读通过"));
    assert!(message.contains("余额"));
    assert!(message.contains("持仓"));
    assert!(message.contains("未验证：挂单"));
    assert!(message.contains("订单权限"));
    assert!(!message.contains("订单权限未验证"));
    assert!(message.contains(".env 原子写入"));
    assert!(message.contains("未加密"));
    assert!(!message.contains("Idempotency"));
}

#[test]
fn same_action_run_detail_refresh_keeps_stale_value_available() {
    let mut state = LoadState::Ready(Some(ActionRun {
        id: "act-1".into(),
        kind: shared_types::ActionRunKind::TradingOrderCancel,
        status: shared_types::ActionRunStatus::Succeeded,
        actor: "tester".into(),
        target: None,
        idempotency_key: Some("cancel:o1".into()),
        request_id: Some("req-1".into()),
        message: "done".into(),
        problem: None,
        result: None,
        mutation: None,
        started_at_ms: 1,
        updated_at_ms: 2,
    }));

    mark_action_run_detail_loading(&mut state, "act-1");
    apply_settings_result(
        &mut state,
        Err(ApiError::client("NETWORK", "detail refresh failed")),
    );

    assert_eq!(
        state
            .value()
            .and_then(Option::as_ref)
            .map(|run| run.id.as_str()),
        Some("act-1")
    );
    assert_eq!(
        state.problem().map(|problem| problem.code.as_str()),
        Some("NETWORK")
    );
}

#[test]
fn different_action_run_detail_refresh_clears_previous_value() {
    let mut state = LoadState::Ready(Some(ActionRun {
        id: "act-1".into(),
        kind: shared_types::ActionRunKind::TradingOrderCancel,
        status: shared_types::ActionRunStatus::Succeeded,
        actor: "tester".into(),
        target: None,
        idempotency_key: Some("cancel:o1".into()),
        request_id: Some("req-1".into()),
        message: "done".into(),
        problem: None,
        result: None,
        mutation: None,
        started_at_ms: 1,
        updated_at_ms: 2,
    }));

    mark_action_run_detail_loading(&mut state, "act-2");

    assert!(matches!(state, LoadState::Loading));
}

#[test]
fn credential_save_fingerprint_is_order_insensitive() {
    let left = credential_save_fingerprint(
        " OKX ",
        &[
            ("api_secret".into(), "secret".into()),
            ("api_key".into(), "key".into()),
        ],
    );
    let right = credential_save_fingerprint(
        "okx",
        &[
            ("api_key".into(), "key".into()),
            ("api_secret".into(), "secret".into()),
        ],
    );

    assert_eq!(left, right);
}

#[test]
fn credential_replay_key_reuses_only_same_fingerprint() {
    let existing = CredentialSaveReplay {
        fingerprint: "okx\napi_key=key".into(),
        key: "settings-credentials-fixed".into(),
    };

    assert_eq!(
        credential_save_replay_key(Some(existing.clone()), "okx\napi_key=key"),
        "settings-credentials-fixed"
    );
    assert_ne!(
        credential_save_replay_key(Some(existing), "binance\napi_key=key"),
        "settings-credentials-fixed"
    );
}

#[test]
fn credential_replay_key_kept_for_transport_or_in_flight_errors() {
    assert!(should_reuse_credential_replay_key(&ApiError::client(
        "TIMEOUT", "slow"
    )));
    assert!(should_reuse_credential_replay_key(&ApiError::client(
        codes::ACTION_RUN_IN_FLIGHT,
        "busy"
    )));
    assert!(!should_reuse_credential_replay_key(&ApiError::client(
        "CREDENTIAL_VALIDATION_FAILED",
        "bad key"
    )));
}

#[test]
fn risk_config_replay_key_reuses_only_the_same_patch() {
    let patch = shared_types::RiskConfigPatch {
        max_order_notional: Some(1_000.0),
        max_open_orders: Some(4),
        max_hedge_imbalance_pct: None,
        allowed_exchanges: Some(vec!["okx".into()]),
        allowed_symbols: None,
        protected_positions: None,
        auto_profit_close: None,
    };
    let existing = RiskConfigReplay {
        patch: patch.clone(),
        key: "settings-risk-config-fixed".into(),
    };

    assert_eq!(
        risk_config_replay_slot(Some(existing.clone()), &patch).key,
        "settings-risk-config-fixed"
    );
    let mut changed = patch;
    changed.max_open_orders = Some(5);
    assert_ne!(
        risk_config_replay_slot(Some(existing), &changed).key,
        "settings-risk-config-fixed"
    );
}
