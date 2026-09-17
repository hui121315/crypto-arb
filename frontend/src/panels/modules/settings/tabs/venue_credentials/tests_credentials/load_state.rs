use super::*;

#[test]
fn save_status_preserves_cold_load_error_problem() {
    let problem = ApiProblem::new("RATE_LIMITED", "slow down")
        .with_status(429)
        .with_request_id(Some("req-venue".into()))
        .with_retry_after_ms(Some(3_000));
    let state = LoadState::Error(problem);

    let result = credential_spec_state(&state, "okx");

    assert!(matches!(
        result,
        CredentialSpecState::Error(ApiProblem {
            code,
            request_id,
            retry_after_ms,
            ..
        }) if code == "RATE_LIMITED"
            && request_id.as_deref() == Some("req-venue")
            && retry_after_ms == Some(3_000)
    ));
}

#[test]
fn cold_credential_error_keeps_selector_context_and_blocks_static_success_copy() {
    let problem = ApiProblem::new("CREDENTIALS_UNAVAILABLE", "credential registry unavailable")
        .with_source("settings.credentials")
        .with_status(502)
        .with_request_id(Some("req-credentials-cold".into()))
        .with_retry_after_ms(Some(4_000));

    let selector = venue_options_error_label(&problem);
    let save = credential_spec_state(&LoadState::Error(problem), "okx");

    assert!(selector.contains("读取交易所列表失败：credential registry unavailable"));
    assert!(selector.contains("source settings.credentials"));
    assert!(selector.contains("HTTP 502"));
    assert!(selector.contains("request_id req-credentials-cold"));
    assert!(selector.contains("retry 4000ms"));
    assert!(!selector.contains("读取中"));
    assert!(matches!(
        save,
        CredentialSpecState::Error(ApiProblem { code, .. })
            if code == "CREDENTIALS_UNAVAILABLE"
    ));
}

#[test]
fn save_status_uses_stale_credential_spec() {
    let state = LoadState::Stale {
        value: VenueCredentialsResponse {
            venues: vec![okx_credential_status()],
            secret_storage: SecretStorageStatus::runtime_only(),
        },
        problem: ApiProblem::new("UPSTREAM_HTTP", "temporary"),
    };

    let result = credential_spec_state(&state, "okx");

    assert!(matches!(
        result,
        CredentialSpecState::Ready(VenueCredentialStatus { venue, .. }) if venue == "okx"
    ));
    assert!(credential_spec_state(&state, "okx").can_save());
}

#[test]
fn credential_spec_state_separates_loading_selection_and_missing_route() {
    assert!(matches!(
        credential_spec_state(&LoadState::Loading, "okx"),
        CredentialSpecState::Loading
    ));
    let ready = LoadState::Ready(VenueCredentialsResponse {
        venues: vec![okx_credential_status()],
        secret_storage: SecretStorageStatus::runtime_only(),
    });
    assert!(matches!(
        credential_spec_state(&ready, ""),
        CredentialSpecState::SelectVenue
    ));
    assert!(matches!(
        credential_spec_state(&ready, "bybit"),
        CredentialSpecState::MissingSpec { venue } if venue == "bybit"
    ));
    assert!(!credential_spec_state(&LoadState::Loading, "okx").can_save());
}

#[test]
fn credentials_stale_message_keeps_request_and_retry_context() {
    let problem = ApiProblem::new("RATE_LIMITED", "credential refresh slow")
        .with_status(429)
        .with_request_id(Some("req-credentials".into()))
        .with_retry_after_ms(Some(4_000));

    let message = credentials_stale_message(Some(&problem)).unwrap_or_default();

    assert!(message.contains("凭证状态刷新失败，显示上次结果"));
    assert!(message.contains("HTTP 429"));
    assert!(message.contains("request_id req-credentials"));
    assert!(message.contains("retry 4000ms"));
}

#[test]
fn credentials_stale_message_is_absent_without_problem() {
    assert!(credentials_stale_message(None).is_none());
}

#[test]
fn venue_options_error_label_keeps_request_and_retry_context() {
    let problem = ApiProblem::new("RATE_LIMITED", "credential venues slow")
        .with_status(429)
        .with_request_id(Some("req-venue-options".into()))
        .with_retry_after_ms(Some(6_000));

    let message = venue_options_error_label(&problem);

    assert!(message.contains("读取交易所列表失败：credential venues slow"));
    assert!(message.contains("HTTP 429"));
    assert!(message.contains("request_id req-venue-options"));
    assert!(message.contains("retry 6000ms"));
}

#[test]
fn ws_stale_message_keeps_request_and_retry_context() {
    let problem = ApiProblem::new("RATE_LIMITED", "ws capability refresh slow")
        .with_status(429)
        .with_request_id(Some("req-ws".into()))
        .with_retry_after_ms(Some(5_000));

    let message = ws_stale_message(Some(&problem)).unwrap_or_default();

    assert!(message.contains("WS 能力刷新失败，显示上次结果"));
    assert!(message.contains("HTTP 429"));
    assert!(message.contains("request_id req-ws"));
    assert!(message.contains("retry 5000ms"));
}

#[test]
fn ws_stale_message_is_absent_without_problem() {
    assert!(ws_stale_message(None).is_none());
}
