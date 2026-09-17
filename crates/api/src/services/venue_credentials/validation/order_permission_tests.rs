use super::*;

#[test]
fn evidence_does_not_duplicate_explicit_order_permission_probe() {
    let evidence = evidence(
        VenueCredentialValidationStatus::ReadOnlyOk,
        vec![probe(
            "order_permission",
            VenueCredentialProbeStatus::Ok,
            "bybit_linear_trade_read_write",
            "bybit.GET /v5/user/query-api",
            "read-only order-permission probe succeeded",
        )],
    );

    let order_permission_count = evidence
        .probes
        .iter()
        .filter(|probe| probe.kind == "order_permission")
        .count();

    assert_eq!(order_permission_count, 1);
    assert_eq!(evidence.probes[0].status, VenueCredentialProbeStatus::Ok);
    assert_eq!(evidence.probes[0].source, "bybit.GET /v5/user/query-api");
}

#[test]
fn unproven_order_permission_probe_names_venue_scope() {
    let probe = order_permission_unproven_probe(" Bybit ");

    assert_eq!(probe.kind, "order_permission");
    assert_eq!(probe.status, VenueCredentialProbeStatus::Unknown);
    assert_eq!(probe.scope, "bybit_place_cancel_order_stream");
    assert_eq!(
        probe.source,
        "credential_save.order_permission_unproven.bybit"
    );
    assert!(probe.message.contains("order finality"));
}

#[test]
fn read_only_order_permission_rejects_http_permission_denial() {
    let (status, message) = classify_order_permission_probe_error(&exchange::ExchangeError::Http {
        status: 403,
        body: "permission denied".into(),
    });

    assert_eq!(status, VenueCredentialProbeStatus::Failed);
    assert!(message.contains("rejected credentials"));
}

#[test]
fn safe_order_place_test_success_remains_unknown_for_cancel_and_finality() {
    let probe = safe_order_place_test_probe(
        "binance_usdm_futures_order_test_no_match",
        "binance.POST /fapi/v1/order/test",
    );

    assert_eq!(probe.kind, "order_permission");
    assert_eq!(probe.status, VenueCredentialProbeStatus::Unknown);
    assert!(probe.message.contains("succeeded"));
    assert!(probe.message.contains("cancel permission"));
    assert!(probe.message.contains("order finality"));
}

#[test]
fn safe_order_place_test_permission_denial_is_failed() {
    let (status, message) = classify_safe_order_place_test_error(&exchange::ExchangeError::Http {
        status: 400,
        body: r#"{"code":-2015,"msg":"Invalid API-key, IP, or permissions for action."}"#.into(),
    });

    assert_eq!(status, VenueCredentialProbeStatus::Failed);
    assert!(message.contains("order-test permission"));
}

#[test]
fn safe_order_place_cancel_success_remains_unknown_for_finality() {
    let probe = safe_order_place_cancel_test_probe(
        "binance_usdm_futures_order_test_cancel_no_match",
        "binance.POST /fapi/v1/order/test + DELETE /fapi/v1/order",
    );

    assert_eq!(probe.kind, "order_permission");
    assert_eq!(probe.status, VenueCredentialProbeStatus::Unknown);
    assert!(probe.message.contains("succeeded"));
    assert!(probe.message.contains("private order stream"));
    assert!(probe.message.contains("order finality"));
}

#[test]
fn safe_order_place_cancel_permission_denial_is_failed() {
    let (status, message) =
        classify_safe_order_place_cancel_test_error(&exchange::ExchangeError::Http {
            status: 403,
            body: r#"{"code":-2015,"msg":"Invalid API-key, IP, or permissions for action."}"#
                .into(),
        });

    assert_eq!(status, VenueCredentialProbeStatus::Failed);
    assert!(message.contains("order-test or cancel permission"));
}

#[test]
fn safe_order_pre_check_success_remains_unknown_for_cancel_and_finality() {
    let probe = safe_order_pre_check_probe(
        "bybit_linear_order_pre_check_no_match",
        "bybit.POST /v5/order/pre-check",
    );

    assert_eq!(probe.kind, "order_permission");
    assert_eq!(probe.status, VenueCredentialProbeStatus::Unknown);
    assert!(probe.message.contains("pre-check"));
    assert!(probe.message.contains("cancel permission"));
    assert!(probe.message.contains("order finality"));
}

#[test]
fn safe_order_pre_check_permission_denial_is_failed() {
    let (status, message) = classify_safe_order_pre_check_error(&exchange::ExchangeError::Http {
        status: 403,
        body: r#"{"retCode":10005,"retMsg":"Permission denied"}"#.into(),
    });

    assert_eq!(status, VenueCredentialProbeStatus::Failed);
    assert!(message.contains("pre-check permission"));
}

#[test]
fn safe_order_cancel_no_match_success_remains_unknown_for_place_and_finality() {
    let probe = safe_order_cancel_no_match_probe(
        "bitget_uta_cancel_no_match",
        "bitget.POST /api/v3/trade/cancel-order",
    );

    assert_eq!(probe.kind, "order_permission");
    assert_eq!(probe.status, VenueCredentialProbeStatus::Unknown);
    assert!(probe.message.contains("cancel probe succeeded"));
    assert!(probe.message.contains("place permission"));
    assert!(probe.message.contains("order finality"));
}

#[test]
fn safe_order_cancel_no_match_permission_denial_is_failed() {
    let (status, message) =
        classify_safe_order_cancel_no_match_error(&exchange::ExchangeError::Http {
            status: 403,
            body: r#"{"code":"40037","msg":"Permission denied"}"#.into(),
        });

    assert_eq!(status, VenueCredentialProbeStatus::Failed);
    assert!(message.contains("cancel permission"));
}

#[test]
fn safe_order_cancel_no_match_collision_is_failed() {
    let (status, message) =
        classify_safe_order_cancel_no_match_error(&exchange::ExchangeError::Api {
            exchange: "bitget".into(),
            code: "safe_cancel_collision".into(),
            message: "bitget safe cancel probe unexpectedly matched an order".into(),
        });

    assert_eq!(status, VenueCredentialProbeStatus::Failed);
    assert!(message.contains("unexpectedly matched an order"));
}

#[test]
fn safe_order_noop_success_verifies_exchange_action_signing_permission() {
    let probe = safe_order_noop_probe("hyperliquid_noop", "hyperliquid.POST /exchange action=noop");

    assert_eq!(probe.kind, "order_permission");
    assert_eq!(probe.status, VenueCredentialProbeStatus::Ok);
    assert!(probe.message.contains("noop exchange action succeeded"));
    assert!(probe.message.contains("consumed one nonce"));
    assert!(probe.message.contains("signing permission verified"));
    assert!(probe.message.contains("separate runtime gates"));
}

#[test]
fn safe_order_noop_permission_denial_is_failed() {
    let (status, message) =
        classify_safe_order_noop_error(&exchange::ExchangeError::Auth("invalid signature".into()));

    assert_eq!(status, VenueCredentialProbeStatus::Failed);
    assert!(message.contains("signed action permission"));
}
