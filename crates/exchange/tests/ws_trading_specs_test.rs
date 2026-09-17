#![allow(clippy::unwrap_used, clippy::panic)]

use exchange::{trading_ws_operation_registry, trading_ws_venues, TRADING_WS_VENUE_COUNT};
use shared_types::{
    ExchangeWsEvidenceScope, ExchangeWsOperation, ExchangeWsReleaseStatus, ExchangeWsSupportStatus,
};
use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

#[test]
fn trading_ws_matrix_covers_exactly_enabled_venues() {
    let response = trading_ws_venues();
    let venues = response
        .venues
        .iter()
        .map(|venue| venue.venue.as_str())
        .collect::<BTreeSet<_>>();

    assert_eq!(venues.len(), TRADING_WS_VENUE_COUNT);
    assert_eq!(
        venues,
        BTreeSet::from([
            "binance",
            "bitget",
            "bybit",
            "gate",
            "gate_crossex",
            "hyperliquid",
            "kraken",
            "kucoin",
            "okx",
        ])
    );
    assert!(!venues.contains("backpack"));
}

#[test]
fn every_enabled_venue_has_private_order_status_stream() {
    let response = trading_ws_venues();

    for venue in response.venues {
        assert!(venue.account_stream.supported, "{}", venue.venue);
        assert!(venue.position_stream.supported, "{}", venue.venue);
        assert!(venue.order_stream.supported, "{}", venue.venue);
        assert!(venue.order_status.supported, "{}", venue.venue);
        assert!(!venue.auth_fields.is_empty(), "{}", venue.venue);
        assert!(!venue.docs.is_empty(), "{}", venue.venue);
    }
}

#[test]
fn kucoin_pro_ws_beta_stays_unavailable_without_runtime_evidence() {
    let response = trading_ws_venues();
    let kucoin = response
        .venues
        .into_iter()
        .find(|venue| venue.venue == "kucoin")
        .unwrap_or_else(|| panic!("kucoin spec missing"));

    assert_eq!(
        kucoin.place_order.status,
        ExchangeWsSupportStatus::SchemaPending
    );
    assert_eq!(
        kucoin.cancel_order.status,
        ExchangeWsSupportStatus::SchemaPending
    );
    assert_eq!(
        kucoin.close_position.status,
        ExchangeWsSupportStatus::SchemaPending
    );
    assert!(kucoin.note.contains("Pro WS"));
    assert!(kucoin.note.contains("beta"));
    assert!(kucoin.note.contains("REST 单次提交"));
    assert!(kucoin.note.contains("禁止 WS 失败后 REST 重放"));
    assert!(kucoin
        .docs
        .iter()
        .any(|doc| doc.url == "https://www.kucoin.com/docs-new/3470252w0"));
    for operation in [&kucoin.place_order, &kucoin.cancel_order] {
        let evidence = operation
            .evidence
            .as_ref()
            .unwrap_or_else(|| panic!("kucoin write evidence missing"));
        assert_eq!(
            evidence.release_status,
            ExchangeWsReleaseStatus::BetaUnavailable
        );
        assert!(!evidence.requires_authenticated_runtime_evidence);
        assert!(!evidence.authenticated_runtime_evidence);
        assert!(!operation.is_live_submittable());
    }
}

#[test]
fn schema_pending_write_ops_never_enter_live_writer() {
    let response = trading_ws_venues();
    for venue in response.venues {
        for (label, op) in [
            ("place_order", &venue.place_order),
            ("cancel_order", &venue.cancel_order),
            ("close_position", &venue.close_position),
        ] {
            if op.status == ExchangeWsSupportStatus::SchemaPending {
                assert!(
                    !op.is_live_submittable(),
                    "{} {label} is schema-pending but marked live-submittable",
                    venue.venue
                );
            }
        }
    }
}

#[test]
fn kucoin_write_path_is_not_live_submittable() {
    let response = trading_ws_venues();
    let venue = response
        .venues
        .iter()
        .find(|venue| venue.venue == "kucoin")
        .unwrap_or_else(|| panic!("kucoin spec missing"));
    assert!(!venue.place_order.is_live_submittable(), "place_order");
    assert!(!venue.cancel_order.is_live_submittable(), "cancel_order");
    assert!(
        !venue.close_position.is_live_submittable(),
        "close_position"
    );
}

#[test]
fn ready_and_permission_write_paths_are_live_submittable() {
    let response = trading_ws_venues();
    for name in [
        "binance",
        "okx",
        "gate",
        "gate_crossex",
        "bitget",
        "hyperliquid",
        "kraken",
        "bybit",
    ] {
        let venue = response
            .venues
            .iter()
            .find(|venue| venue.venue == name)
            .unwrap_or_else(|| panic!("{name} spec missing"));
        assert!(
            venue.place_order.is_live_submittable(),
            "{name} place_order should be live-submittable"
        );
    }
}

#[test]
fn typed_release_status_preserves_the_live_write_venue_matrix() {
    let response = trading_ws_venues();
    let registry = trading_ws_operation_registry();
    let expected_live = BTreeSet::from([
        "binance",
        "bitget",
        "bybit",
        "gate",
        "gate_crossex",
        "hyperliquid",
        "kraken",
        "okx",
    ]);
    let actual_live = response
        .venues
        .iter()
        .filter(|venue| {
            venue.place_order.is_live_submittable() && venue.cancel_order.is_live_submittable()
        })
        .map(|venue| venue.venue.as_str())
        .collect::<BTreeSet<_>>();

    assert_eq!(actual_live, expected_live);
    for venue in &response.venues {
        for operation in [&venue.place_order, &venue.cancel_order] {
            let release_status = operation
                .evidence
                .as_ref()
                .map(|evidence| evidence.release_status)
                .unwrap_or_default();
            if expected_live.contains(venue.venue.as_str()) {
                assert_eq!(
                    release_status,
                    ExchangeWsReleaseStatus::ProductionReady,
                    "{} live write operation must be explicitly production-ready",
                    venue.venue
                );
            }
        }
    }
    for venue in &registry.venues {
        for operation in venue
            .operations
            .iter()
            .filter(|operation| matches!(operation.label.as_str(), "place_order" | "cancel_order"))
        {
            let expected = if venue.venue == "kucoin" {
                ExchangeWsReleaseStatus::BetaUnavailable
            } else {
                ExchangeWsReleaseStatus::ProductionReady
            };
            assert_eq!(
                operation.release_status, expected,
                "{}/{} release status regressed",
                venue.venue, operation.label
            );
        }
    }

    let Some(kucoin) = response.venues.iter().find(|venue| venue.venue == "kucoin") else {
        panic!("kucoin spec missing");
    };
    for operation in [&kucoin.place_order, &kucoin.cancel_order] {
        assert_eq!(
            operation
                .evidence
                .as_ref()
                .map(|evidence| evidence.release_status),
            Some(ExchangeWsReleaseStatus::BetaUnavailable)
        );
        assert!(!operation
            .evidence
            .as_ref()
            .is_some_and(|evidence| evidence.requires_authenticated_runtime_evidence));
        assert!(!operation.is_live_submittable());
    }
}

#[test]
fn live_submittable_write_paths_must_carry_operation_evidence() {
    let response = trading_ws_venues();
    for venue in response.venues {
        for (label, op) in [
            ("place_order", &venue.place_order),
            ("cancel_order", &venue.cancel_order),
            ("close_position", &venue.close_position),
        ] {
            if op.is_live_submittable() {
                assert!(
                    op.evidence.is_some(),
                    "{} {label} is live-submittable without operation evidence",
                    venue.venue
                );
            }
        }
    }
}

#[test]
fn close_position_without_operation_evidence_stays_display_only() {
    let response = trading_ws_venues();
    assert_eq!(response.venues.len(), TRADING_WS_VENUE_COUNT);

    for venue in response.venues {
        assert!(
            venue.close_position.evidence.is_none(),
            "{} close_position gained operation evidence; add a venue fixture/request test before enabling it",
            venue.venue
        );
        assert!(
            !venue.close_position.is_live_submittable(),
            "{} close_position must stay display-only until operation evidence exists",
            venue.venue
        );
    }
}

#[test]
fn close_position_never_enters_operation_registry_without_dedicated_operation_evidence() {
    let response = trading_ws_venues();
    let registry = trading_ws_operation_registry();
    let registry_venues = registry
        .venues
        .iter()
        .map(|venue| venue.venue.as_str())
        .collect::<BTreeSet<_>>();

    let close_position_rows = registry
        .venues
        .iter()
        .flat_map(|venue| venue.operations.iter())
        .filter(|operation| operation.label == "close_position")
        .count();
    assert_eq!(
        0, close_position_rows,
        "close_position rows must stay out of the evidence registry until they have dedicated operation evidence"
    );

    for venue in response.venues {
        assert!(
            registry_venues.contains(venue.venue.as_str()),
            "{} missing from WS operation evidence registry",
            venue.venue
        );
        assert!(
            venue.close_position.evidence.is_none(),
            "{} close_position gained evidence but still has no dedicated registry gate",
            venue.venue
        );
        assert!(
            !venue.close_position.is_live_submittable(),
            "{} close_position must remain display-only until dedicated operation evidence exists",
            venue.venue
        );
    }
}

#[test]
fn binance_live_ws_write_ops_carry_official_evidence() {
    let source = binance_ws_trade_test_source();
    let response = trading_ws_venues();
    let binance = response
        .venues
        .iter()
        .find(|venue| venue.venue == "binance")
        .unwrap_or_else(|| panic!("binance spec missing"));

    for (label, op, operation, parser_test, request_test, fixture) in [
        (
            "place_order",
            &binance.place_order,
            "order.place",
            "binance_ws_place_order_ack_parses_official_fixture",
            "order_place_request_matches_binance_ws_schema",
            "fixtures/binance/ws_order_place_success.json",
        ),
        (
            "cancel_order",
            &binance.cancel_order,
            "order.cancel",
            "binance_ws_cancel_order_ack_parses_official_fixture",
            "order_cancel_request_matches_binance_ws_schema",
            "fixtures/binance/ws_order_cancel_success.json",
        ),
    ] {
        assert!(op.is_live_submittable(), "binance {label}");
        assert_eq!(op.operation.as_deref(), Some(operation), "{label}");
        assert_eq!(op.product, "USD-M", "{label}");
        let evidence = op
            .evidence
            .as_ref()
            .unwrap_or_else(|| panic!("binance {label} missing evidence"));
        assert_eq!(evidence.checked_at, "2026-07-02", "{label}");
        assert!(evidence.doc_url.contains("/websocket-api/"), "{label}");
        assert_eq!(evidence.auth_kind, "api_key_signature", "{label}");
        assert_eq!(
            evidence.parser_test.as_deref(),
            Some(parser_test),
            "{label}"
        );
        assert_eq!(
            evidence.subscription_test.as_deref(),
            Some(request_test),
            "{label}"
        );
        assert_parser_test_includes_fixture(
            &source,
            evidence.parser_test.as_deref(),
            fixture,
            "binance",
            label,
        );
        assert_test_exists(
            &source,
            evidence.subscription_test.as_deref(),
            "binance",
            label,
        );
    }
}

#[test]
fn okx_live_ws_write_ops_carry_official_evidence() {
    let source = okx_ws_trade_test_source();
    let response = trading_ws_venues();
    let okx = response
        .venues
        .iter()
        .find(|venue| venue.venue == "okx")
        .unwrap_or_else(|| panic!("okx spec missing"));

    for (label, op, operation, parser_test, request_test, fixture) in [
        (
            "place_order",
            &okx.place_order,
            "order",
            "okx_ws_place_order_ack_parses_official_fixture",
            "order_request_matches_okx_ws_schema",
            "fixtures/okx/ws_trade_place_order_ack.json",
        ),
        (
            "cancel_order",
            &okx.cancel_order,
            "cancel-order",
            "okx_ws_cancel_order_ack_parses_official_fixture",
            "cancel_request_matches_okx_ws_schema",
            "fixtures/okx/ws_trade_cancel_order_ack.json",
        ),
    ] {
        assert!(op.is_live_submittable(), "okx {label}");
        assert_eq!(op.operation.as_deref(), Some(operation), "{label}");
        let evidence = op
            .evidence
            .as_ref()
            .unwrap_or_else(|| panic!("okx {label} missing evidence"));
        assert_eq!(evidence.checked_at, "2026-07-02", "{label}");
        assert!(evidence.doc_url.contains("trade-ws-"), "{label}");
        assert_eq!(evidence.auth_kind, "login", "{label}");
        assert_eq!(
            evidence.parser_test.as_deref(),
            Some(parser_test),
            "{label}"
        );
        assert_eq!(
            evidence.subscription_test.as_deref(),
            Some(request_test),
            "{label}"
        );
        assert_parser_test_includes_fixture(
            &source,
            evidence.parser_test.as_deref(),
            fixture,
            "okx",
            label,
        );
        assert_test_exists(&source, evidence.subscription_test.as_deref(), "okx", label);
    }
}

#[test]
fn gate_live_ws_write_ops_carry_official_evidence() {
    let source = gate_ws_trade_test_source();
    let response = trading_ws_venues();
    let gate = response
        .venues
        .iter()
        .find(|venue| venue.venue == "gate")
        .unwrap_or_else(|| panic!("gate spec missing"));

    for (label, op, parser_test, request_test, fixture) in [
        (
            "place_order",
            &gate.place_order,
            "gate_ws_order_place_ack_response_parses_official_fixture",
            "place_request_matches_gate_ws_schema",
            "fixtures/gate/ws_futures_order_place_success.json",
        ),
        (
            "cancel_order",
            &gate.cancel_order,
            "gate_ws_order_cancel_ack_response_parses_official_fixture",
            "cancel_request_matches_gate_ws_schema",
            "fixtures/gate/ws_futures_order_cancel_success.json",
        ),
    ] {
        assert!(op.is_live_submittable(), "gate {label}");
        let evidence = op
            .evidence
            .as_ref()
            .unwrap_or_else(|| panic!("gate {label} missing evidence"));
        assert_eq!(evidence.checked_at, "2026-07-02", "gate {label}");
        assert!(
            evidence.doc_url.starts_with("https://www.gate.com/docs/"),
            "gate {label}"
        );
        assert_eq!(evidence.auth_kind, "login_then_api", "gate {label}");
        assert_eq!(
            evidence.parser_test.as_deref(),
            Some(parser_test),
            "gate {label}"
        );
        assert_eq!(
            evidence.subscription_test.as_deref(),
            Some(request_test),
            "gate {label}"
        );
        assert_parser_test_includes_fixture(
            &source,
            evidence.parser_test.as_deref(),
            fixture,
            "gate",
            label,
        );
        assert_test_exists(
            &source,
            evidence.subscription_test.as_deref(),
            "gate",
            label,
        );
    }
}

#[test]
fn hyperliquid_live_ws_write_ops_carry_official_evidence() {
    let source = hyperliquid_ws_trade_test_source();
    let response = trading_ws_venues();
    let hyperliquid = response
        .venues
        .iter()
        .find(|venue| venue.venue == "hyperliquid")
        .unwrap_or_else(|| panic!("hyperliquid spec missing"));

    for (label, op, operation, parser_test, request_test, fixture) in [
        (
            "place_order",
            &hyperliquid.place_order,
            "post/order",
            "hyperliquid_ws_place_order_ack_parses_official_fixture",
            "signed_post_order_request_matches_hyperliquid_ws_schema",
            "fixtures/hyperliquid/ws_post_order_resting.json",
        ),
        (
            "cancel_order",
            &hyperliquid.cancel_order,
            "post/cancel",
            "hyperliquid_ws_cancel_order_ack_parses_official_fixture",
            "signed_post_request_matches_hyperliquid_ws_schema",
            "fixtures/hyperliquid/ws_post_cancel_success.json",
        ),
    ] {
        assert!(op.is_live_submittable(), "hyperliquid {label}");
        assert_eq!(op.operation.as_deref(), Some(operation), "{label}");
        assert_eq!(
            op.product, "Perp",
            "hyperliquid WS write specs must not claim spot write support"
        );
        let evidence = op
            .evidence
            .as_ref()
            .unwrap_or_else(|| panic!("hyperliquid {label} missing evidence"));
        assert_eq!(evidence.checked_at, "2026-07-02", "{label}");
        assert!(
            evidence.doc_url.contains("/websocket/post-requests"),
            "{label}"
        );
        assert_eq!(evidence.auth_kind, "signed_action", "{label}");
        assert_eq!(
            evidence.parser_test.as_deref(),
            Some(parser_test),
            "{label}"
        );
        assert_eq!(
            evidence.subscription_test.as_deref(),
            Some(request_test),
            "{label}"
        );
        assert_parser_test_includes_fixture(
            &source,
            evidence.parser_test.as_deref(),
            fixture,
            "hyperliquid",
            label,
        );
        assert_test_exists(
            &source,
            evidence.subscription_test.as_deref(),
            "hyperliquid",
            label,
        );
    }
}

#[test]
fn bybit_live_ws_write_ops_carry_official_evidence() {
    let source = bybit_ws_trade_test_source();
    let response = trading_ws_venues();
    let bybit = response
        .venues
        .iter()
        .find(|venue| venue.venue == "bybit")
        .unwrap_or_else(|| panic!("bybit spec missing"));

    for (label, op, operation, parser_test, request_test, fixture) in [
        (
            "place_order",
            &bybit.place_order,
            "order.create",
            "bybit_ws_place_order_ack_parses_official_fixture",
            "order_create_request_matches_bybit_ws_schema",
            "fixtures/bybit/ws_order_create_ack.json",
        ),
        (
            "cancel_order",
            &bybit.cancel_order,
            "order.cancel",
            "bybit_ws_cancel_order_ack_parses_official_fixture",
            "order_cancel_request_matches_bybit_ws_schema",
            "fixtures/bybit/ws_order_cancel_ack.json",
        ),
    ] {
        assert!(op.is_live_submittable(), "bybit {label}");
        assert_eq!(op.operation.as_deref(), Some(operation), "{label}");
        assert_eq!(op.product, "Linear", "{label}");
        let evidence = op
            .evidence
            .as_ref()
            .unwrap_or_else(|| panic!("bybit {label} missing evidence"));
        assert_eq!(evidence.checked_at, "2026-07-02", "{label}");
        assert!(
            evidence.doc_url.contains("/websocket/trade/guideline"),
            "{label}"
        );
        assert_eq!(evidence.auth_kind, "signed_trade_ws", "{label}");
        assert_eq!(
            evidence.parser_test.as_deref(),
            Some(parser_test),
            "{label}"
        );
        assert_eq!(
            evidence.subscription_test.as_deref(),
            Some(request_test),
            "{label}"
        );
        assert_parser_test_includes_fixture(
            &source,
            evidence.parser_test.as_deref(),
            fixture,
            "bybit",
            label,
        );
        assert_test_exists(
            &source,
            evidence.subscription_test.as_deref(),
            "bybit",
            label,
        );
    }
}

#[test]
fn bitget_live_ws_write_ops_carry_official_evidence() {
    let source = bitget_uta_ws_trade_test_source();
    let response = trading_ws_venues();
    let bitget = response
        .venues
        .iter()
        .find(|venue| venue.venue == "bitget")
        .unwrap_or_else(|| panic!("bitget spec missing"));

    for (label, op, operation, doc_tail, parser_test, request_test, fixture) in [
        (
            "place_order",
            &bitget.place_order,
            "place-order",
            "/Place-Order-Channel",
            "bitget_uta_ws_place_order_ack_parses_official_fixture",
            "place_request_matches_v3_envelope_shape",
            "fixtures/bitget/uta_ws_place_order_ack.json",
        ),
        (
            "cancel_order",
            &bitget.cancel_order,
            "cancel-order",
            "/Cancel-Order-Channel",
            "bitget_uta_ws_cancel_order_ack_parses_official_fixture",
            "cancel_request_envelope_carries_client_oid_only",
            "fixtures/bitget/uta_ws_cancel_order_ack.json",
        ),
    ] {
        assert!(op.is_live_submittable(), "bitget {label}");
        assert_eq!(op.operation.as_deref(), Some(operation), "{label}");
        assert_eq!(op.product, "USDT/USDC-FUTURES", "{label}");
        let evidence = op
            .evidence
            .as_ref()
            .unwrap_or_else(|| panic!("bitget {label} missing evidence"));
        assert_eq!(evidence.checked_at, "2026-07-02", "{label}");
        assert!(evidence.doc_url.ends_with(doc_tail), "{label}");
        assert_eq!(evidence.auth_kind, "login", "{label}");
        assert_eq!(
            evidence.parser_test.as_deref(),
            Some(parser_test),
            "{label}"
        );
        assert_eq!(
            evidence.subscription_test.as_deref(),
            Some(request_test),
            "{label}"
        );
        assert_parser_test_includes_fixture(
            &source,
            evidence.parser_test.as_deref(),
            fixture,
            "bitget",
            label,
        );
        assert_test_exists(
            &source,
            evidence.subscription_test.as_deref(),
            "bitget",
            label,
        );
    }
}

#[test]
fn kucoin_beta_write_ops_remain_schema_only() {
    let source = kucoin_ws_user_test_source();
    let response = trading_ws_venues();
    let kucoin = response
        .venues
        .iter()
        .find(|venue| venue.venue == "kucoin")
        .unwrap_or_else(|| panic!("kucoin spec missing"));

    for (label, op, operation, fixture) in [
        (
            "place_order",
            &kucoin.place_order,
            "futures.order",
            "fixtures/kucoin/wsapi_pro_order_ack.json",
        ),
        (
            "cancel_order",
            &kucoin.cancel_order,
            "futures.cancel",
            "fixtures/kucoin/wsapi_pro_cancel_ack.json",
        ),
    ] {
        assert_kucoin_beta_write_evidence(&source, label, op, operation, fixture);
    }
}

fn assert_kucoin_beta_write_evidence(
    source: &str,
    label: &str,
    operation: &ExchangeWsOperation,
    operation_name: &str,
    fixture: &str,
) {
    assert!(!operation.is_live_submittable(), "kucoin {label}");
    assert_eq!(
        operation.status,
        ExchangeWsSupportStatus::SchemaPending,
        "{label}"
    );
    assert_eq!(
        operation.operation.as_deref(),
        Some(operation_name),
        "{label}"
    );
    let evidence = operation
        .evidence
        .as_ref()
        .unwrap_or_else(|| panic!("kucoin {label} missing schema evidence"));
    assert_eq!(evidence.checked_at, "2026-07-30", "{label}");
    assert!(
        evidence
            .doc_url
            .starts_with("https://www.kucoin.com/docs-new/"),
        "{label}"
    );
    assert_eq!(
        evidence.auth_kind, "signed_wsapi_query_challenge",
        "{label}"
    );
    assert_eq!(
        evidence.release_status,
        ExchangeWsReleaseStatus::BetaUnavailable,
        "{label}"
    );
    assert!(!evidence.requires_authenticated_runtime_evidence, "{label}");
    assert!(!evidence.authenticated_runtime_evidence, "{label}");
    for marker in ["Pro WS schema", "beta", "REST 单次提交"] {
        assert!(operation.note.contains(marker), "{label}: {marker}");
    }
    assert_eq!(
        evidence.parser_test.as_deref(),
        Some("parses_order_balance_position_and_pro_ack"),
        "{label}"
    );
    assert_eq!(
        evidence.subscription_test.as_deref(),
        Some("pro_order_and_cancel_payloads_use_official_ops"),
        "{label}"
    );
    assert_parser_test_includes_fixture(
        source,
        evidence.parser_test.as_deref(),
        fixture,
        "kucoin",
        label,
    );
    assert_test_exists(
        source,
        evidence.subscription_test.as_deref(),
        "kucoin",
        label,
    );
}

#[test]
fn ready_private_ws_streams_have_operation_evidence() {
    let response = trading_ws_venues();
    for venue in response.venues {
        for (label, op) in private_stream_ops(&venue) {
            if !op.supported {
                continue;
            }
            let evidence = op
                .evidence
                .as_ref()
                .unwrap_or_else(|| panic!("{} {label} missing evidence", venue.venue));
            assert!(!evidence.checked_at.is_empty(), "{} {label}", venue.venue);
            assert!(!evidence.doc_version.is_empty(), "{} {label}", venue.venue);
            assert!(
                evidence.doc_url.starts_with("https://"),
                "{} {label}",
                venue.venue
            );
            assert!(!evidence.auth_kind.is_empty(), "{} {label}", venue.venue);
        }
    }
}

#[test]
fn private_ws_evidence_points_to_existing_tests() {
    let response = trading_ws_venues();
    for venue in response.venues {
        for (label, op) in private_stream_ops(&venue) {
            let Some(evidence) = op.evidence.as_ref() else {
                continue;
            };
            let test_source = ws_operation_test_source(venue.venue.as_str(), label);
            assert_test_exists(
                &test_source,
                evidence.parser_test.as_deref(),
                &venue.venue,
                label,
            );
            assert_test_exists(
                &test_source,
                evidence.subscription_test.as_deref(),
                &venue.venue,
                label,
            );
        }
    }
}

#[test]
fn binance_private_order_stream_fixture_is_recorded() {
    let registry = trading_ws_operation_registry();
    let binance = registry
        .venues
        .iter()
        .find(|venue| venue.venue == "binance")
        .unwrap_or_else(|| panic!("binance registry missing"));
    let source = ws_user_test_source("binance");
    let fixture = "crates/exchange/fixtures/binance/usdm_order_trade_update_partial_fill.json";
    let fixture_suffix = "fixtures/binance/usdm_order_trade_update_partial_fill.json";
    let hash = "sha256:590ce8eeaa980adeed148aeaac453da49bd130571bfb6fe0aeed47178ea26127";

    for (label, scope) in [
        ("fill_stream", ExchangeWsEvidenceScope::PrivateFillStream),
        ("order_stream", ExchangeWsEvidenceScope::PrivateOrderStream),
    ] {
        let operation = registry_operation(binance, label);
        assert_eq!(scope, operation.evidence_scope, "{label}");
        assert_eq!(Some(fixture), operation.fixture_id.as_deref(), "{label}");
        assert_eq!(Some(hash), operation.fixture_hash.as_deref(), "{label}");
        assert_parser_test_includes_fixture(
            &source,
            operation.parser_test.as_deref(),
            fixture_suffix,
            "binance",
            label,
        );
    }

    let status = registry_operation(binance, "order_status");
    let status_fixture = "crates/exchange/fixtures/binance/ws_order_status_filled.json";
    assert_eq!(
        ExchangeWsEvidenceScope::OrderStatusRead,
        status.evidence_scope
    );
    assert_eq!(Some(status_fixture), status.fixture_id.as_deref());
    assert_eq!(
        Some("sha256:adf8ea0c2c78ff70746441276d83061e61558a7a5abdc1a88d44e72552b0ee5f"),
        status.fixture_hash.as_deref()
    );
    assert_parser_test_includes_fixture(
        &binance_ws_trade_test_source(),
        status.parser_test.as_deref(),
        "fixtures/binance/ws_order_status_filled.json",
        "binance",
        "order_status",
    );
}

#[test]
fn bybit_private_stream_fixtures_are_recorded() {
    let registry = trading_ws_operation_registry();
    let bybit = registry
        .venues
        .iter()
        .find(|venue| venue.venue == "bybit")
        .unwrap_or_else(|| panic!("bybit registry missing"));
    let source = ws_user_test_source("bybit");
    for (label, scope, fixture, hash) in [
        (
            "account_stream",
            ExchangeWsEvidenceScope::PrivateAccountStream,
            "crates/exchange/fixtures/bybit/ws_user_wallet_snapshot.json",
            "sha256:aa7b33dddabafac0c57e34644bf15d9a6363f8bed48fd8a2d9fec7d9af5f8799",
        ),
        (
            "position_stream",
            ExchangeWsEvidenceScope::PrivatePositionStream,
            "crates/exchange/fixtures/bybit/ws_user_position_snapshot.json",
            "sha256:85aa610f0572bc1de40bb72500927ea456b63d815aa0edc787e7a146a606fd56",
        ),
        (
            "fill_stream",
            ExchangeWsEvidenceScope::PrivateFillStream,
            "crates/exchange/fixtures/bybit/ws_user_execution_fill.json",
            "sha256:0a74cdbfda5e62ad0fdd105592fa5493fda63bdf396a515adcd489213b1bdb39",
        ),
        (
            "order_stream",
            ExchangeWsEvidenceScope::PrivateOrderStream,
            "crates/exchange/fixtures/bybit/ws_user_order_filled.json",
            "sha256:92185c0e848f978cfe0e301a86439ea47d95984b583304c25c5897e5b9104536",
        ),
        (
            "order_status",
            ExchangeWsEvidenceScope::OrderStatusRead,
            "crates/exchange/fixtures/bybit/ws_user_order_filled.json",
            "sha256:92185c0e848f978cfe0e301a86439ea47d95984b583304c25c5897e5b9104536",
        ),
    ] {
        let operation = registry_operation(bybit, label);
        assert_eq!(scope, operation.evidence_scope, "{label}");
        assert_eq!(Some(fixture), operation.fixture_id.as_deref(), "{label}");
        assert_eq!(Some(hash), operation.fixture_hash.as_deref(), "{label}");
        assert_parser_test_includes_fixture(
            &source,
            operation.parser_test.as_deref(),
            fixture.strip_prefix("crates/exchange/").unwrap_or(fixture),
            "bybit",
            label,
        );
    }
}

#[test]
fn kucoin_classic_fill_stream_records_identity_evidence_without_invented_fee() {
    let response = trading_ws_venues();
    let kucoin = response
        .venues
        .into_iter()
        .find(|venue| venue.venue == "kucoin")
        .unwrap_or_else(|| panic!("kucoin spec missing"));

    assert!(kucoin.fill_stream.supported);
    assert_eq!(kucoin.fill_stream.status, ExchangeWsSupportStatus::Ready);
    assert!(kucoin.fill_stream.note.contains("signed REST fills"));

    let registry = trading_ws_operation_registry();
    let kucoin = registry
        .venues
        .iter()
        .find(|venue| venue.venue == "kucoin")
        .unwrap_or_else(|| panic!("kucoin registry missing"));
    let fill = registry_operation(kucoin, "fill_stream");
    assert_eq!(
        ExchangeWsEvidenceScope::PrivateFillStream,
        fill.evidence_scope
    );
    assert_eq!(
        Some("crates/exchange/fixtures/kucoin/classic_ws_trade_orders_match.json"),
        fill.fixture_id.as_deref()
    );
    assert!(fill.note.contains("实际 fee"));
}

#[test]
fn operation_matrix_boundaries_match_ws_registry_projection() {
    let matrix_rows = operation_matrix_rows();
    let venues = trading_ws_venues();
    let registry = trading_ws_operation_registry();

    assert_eq!(TRADING_WS_VENUE_COUNT, matrix_rows.len());

    for row in &matrix_rows {
        let venue = venues
            .venues
            .iter()
            .find(|venue| venue.venue == row.venue)
            .unwrap_or_else(|| panic!("{} venue spec missing", row.venue_title));
        let registry_venue = registry
            .venues
            .iter()
            .find(|venue| venue.venue == row.venue)
            .unwrap_or_else(|| panic!("{} registry projection missing", row.venue_title));

        assert_eq!(
            "display_only_without_operation_evidence", row.close_position_boundary,
            "{} close_position matrix boundary changed",
            row.venue_title
        );
        assert!(
            venue.close_position.evidence.is_none(),
            "{} close_position gained operation evidence without a matrix boundary update",
            row.venue_title
        );
        assert!(
            !venue.close_position.is_live_submittable(),
            "{} close_position must remain display-only",
            row.venue_title
        );
        assert!(
            registry_venue
                .operations
                .iter()
                .all(|operation| operation.label != "close_position"),
            "{} close_position must stay out of the runtime operation registry",
            row.venue_title
        );

        match row.ws_live_write_path.as_str() {
            "recorded_place_cancel" => {
                assert_recorded_place_cancel_projection(row, venue, registry_venue);
            }
            "display_only_schema_pending" => {
                assert_schema_pending_place_cancel_projection(row, venue, registry_venue);
            }
            "display_only_runtime_evidence" => {
                assert_runtime_gated_place_cancel_projection(row, venue, registry_venue);
            }
            value => panic!("{} has unknown ws_live_write_path {value}", row.venue_title),
        }

        assert_eq!(
            "ack_not_final", row.finality_boundary,
            "{} finality matrix boundary changed",
            row.venue_title
        );
        for label in ["place_order", "cancel_order"] {
            let operation = registry_operation(registry_venue, label);
            assert_eq!(
                ExchangeWsEvidenceScope::AckOnly,
                operation.evidence_scope,
                "{}/{} matrix finality boundary must stay ACK-only",
                row.venue,
                label
            );
        }
    }
}

#[test]
fn operation_matrix_ws_fixtures_match_runtime_registry_evidence() {
    let matrix_rows = operation_matrix_rows();
    let registry = trading_ws_operation_registry();

    assert_eq!(TRADING_WS_VENUE_COUNT, matrix_rows.len());

    for row in &matrix_rows {
        let registry_venue = registry
            .venues
            .iter()
            .find(|venue| venue.venue == row.venue)
            .unwrap_or_else(|| panic!("{} registry projection missing", row.venue_title));

        for label in ["place_order", "cancel_order"] {
            let expected = row.ws_operation_fixture(label);
            let operation = registry_operation(registry_venue, label);
            assert_eq!(
                Some(expected.id.as_str()),
                operation.fixture_id.as_deref(),
                "{}/{} fixture id must match operation matrix",
                row.venue,
                label
            );
            assert_eq!(
                Some(expected.hash.as_str()),
                operation.fixture_hash.as_deref(),
                "{}/{} fixture hash must match operation matrix",
                row.venue,
                label
            );
        }

        for operation in registry_venue
            .operations
            .iter()
            .filter(|operation| !matches!(operation.label.as_str(), "place_order" | "cancel_order"))
        {
            if operation.fixture_id.is_some() || operation.fixture_hash.is_some() {
                assert_ne!(
                    ExchangeWsEvidenceScope::AckOnly,
                    operation.evidence_scope,
                    "{}/{} private/read fixture cannot inherit ACK scope",
                    row.venue,
                    operation.label
                );
                assert!(operation.fixture_id.is_some() && operation.fixture_hash.is_some());
            }
        }
    }
}

fn assert_recorded_place_cancel_projection(
    row: &OperationMatrixRow,
    venue: &shared_types::ExchangeWsVenue,
    registry_venue: &shared_types::ExchangeWsOperationVenue,
) {
    for (label, operation) in [
        ("place_order", &venue.place_order),
        ("cancel_order", &venue.cancel_order),
    ] {
        assert!(
            operation.is_live_submittable(),
            "{} {label} must be live-submittable when matrix says recorded_place_cancel",
            row.venue_title
        );
        assert_ne!(
            ExchangeWsSupportStatus::SchemaPending,
            operation.status,
            "{} {label} cannot be schema-pending when matrix says recorded_place_cancel",
            row.venue_title
        );
        let registry_operation = registry_operation(registry_venue, label);
        assert_eq!(
            operation.status, registry_operation.status,
            "{}/{}",
            row.venue, label
        );
        assert_eq!(
            operation.operation.as_deref(),
            registry_operation.operation.as_deref(),
            "{}/{}",
            row.venue,
            label
        );
        assert_eq!(
            operation.product, registry_operation.product,
            "{}/{}",
            row.venue, label
        );
        assert!(
            registry_operation.parser_test.is_some(),
            "{}/{}",
            row.venue,
            label
        );
        assert!(
            registry_operation.subscription_test.is_some(),
            "{}/{}",
            row.venue,
            label
        );
    }
}

fn assert_schema_pending_place_cancel_projection(
    row: &OperationMatrixRow,
    venue: &shared_types::ExchangeWsVenue,
    registry_venue: &shared_types::ExchangeWsOperationVenue,
) {
    for (label, operation) in [
        ("place_order", &venue.place_order),
        ("cancel_order", &venue.cancel_order),
    ] {
        assert_eq!(
            ExchangeWsSupportStatus::SchemaPending,
            operation.status,
            "{} {label} must remain schema-pending while matrix says display_only_schema_pending",
            row.venue_title
        );
        assert!(
            !operation.is_live_submittable(),
            "{} {label} must not enter live writer while matrix says display_only_schema_pending",
            row.venue_title
        );
        let registry_operation = registry_operation(registry_venue, label);
        assert_eq!(
            ExchangeWsSupportStatus::SchemaPending,
            registry_operation.status,
            "{}/{} runtime registry must preserve schema-pending status",
            row.venue,
            label
        );
        assert_eq!(
            ExchangeWsEvidenceScope::AckOnly,
            registry_operation.evidence_scope,
            "{}/{} schema evidence is ACK-only and not finality",
            row.venue,
            label
        );
    }
}

fn assert_runtime_gated_place_cancel_projection(
    row: &OperationMatrixRow,
    venue: &shared_types::ExchangeWsVenue,
    registry_venue: &shared_types::ExchangeWsOperationVenue,
) {
    for (label, operation) in [
        ("place_order", &venue.place_order),
        ("cancel_order", &venue.cancel_order),
    ] {
        assert_eq!(
            ExchangeWsSupportStatus::RequiresPermission,
            operation.status,
            "{} {label} must remain permission-gated while matrix says display_only_runtime_evidence",
            row.venue_title
        );
        let evidence = operation
            .evidence
            .as_ref()
            .unwrap_or_else(|| panic!("{} {label} missing WS evidence", row.venue_title));
        assert_eq!(
            ExchangeWsReleaseStatus::ProductionReady,
            evidence.release_status,
            "{} {label} must use the current production release status",
            row.venue_title
        );
        assert!(evidence.requires_authenticated_runtime_evidence);
        assert!(!evidence.authenticated_runtime_evidence);
        assert!(!operation.is_live_submittable());

        let registry_operation = registry_operation(registry_venue, label);
        assert_eq!(operation.status, registry_operation.status);
        assert_eq!(
            ExchangeWsEvidenceScope::AckOnly,
            registry_operation.evidence_scope,
            "{}/{} schema evidence is ACK-only and not finality",
            row.venue,
            label
        );
    }
}

fn registry_operation<'a>(
    venue: &'a shared_types::ExchangeWsOperationVenue,
    label: &str,
) -> &'a shared_types::ExchangeWsOperationRegistryRow {
    venue
        .operations
        .iter()
        .find(|operation| operation.label == label)
        .unwrap_or_else(|| panic!("{} missing registry operation {label}", venue.venue))
}

#[derive(Debug)]
struct OperationMatrixRow {
    venue_title: String,
    venue: String,
    ws_live_write_path: String,
    close_position_boundary: String,
    ws_operation_fixtures: Vec<OperationMatrixFixture>,
    finality_boundary: String,
}

impl OperationMatrixRow {
    fn ws_operation_fixture(&self, label: &str) -> &OperationMatrixFixture {
        let index = match label {
            "place_order" => 0,
            "cancel_order" => 1,
            value => panic!(
                "{} has unknown WS operation label {value}",
                self.venue_title
            ),
        };
        self.ws_operation_fixtures.get(index).unwrap_or_else(|| {
            panic!(
                "{} missing matrix fixture metadata for {label}",
                self.venue_title
            )
        })
    }
}

#[derive(Debug)]
struct OperationMatrixFixture {
    id: String,
    hash: String,
}

fn operation_matrix_rows() -> Vec<OperationMatrixRow> {
    const MATRIX: &str = include_str!("../../../scripts/exchange_operation_evidence_matrix.tsv");
    const HEADER: &str = "venue\trest_trade_write_order_ack\trest_private_order_status\trest_private_account_balance\trest_private_account_position\tws_live_write_path\tws_private_stream_evidence\tclose_position_boundary\tdiagnostic_fixture_boundary\tdiagnostic_fixture_ids\tdiagnostic_fixture_hashes\tws_operation_fixture_ids\tws_operation_fixture_hashes\tfinality_boundary";

    let mut lines = MATRIX.lines();
    assert_eq!(Some(HEADER), lines.next());
    lines
        .enumerate()
        .map(|(index, line)| {
            let columns = line.split('\t').collect::<Vec<_>>();
            assert_eq!(14, columns.len(), "bad operation matrix row {}", index + 2);
            let fixture_ids = csv_column(columns[11]);
            let fixture_hashes = csv_column(columns[12]);
            assert_eq!(
                fixture_ids.len(),
                fixture_hashes.len(),
                "{} WS operation fixture/hash count mismatch",
                columns[0]
            );
            assert_eq!(
                2,
                fixture_ids.len(),
                "{} must keep place/cancel fixture metadata",
                columns[0]
            );
            OperationMatrixRow {
                venue_title: columns[0].to_owned(),
                venue: columns[0].to_ascii_lowercase(),
                ws_live_write_path: columns[5].to_owned(),
                close_position_boundary: columns[7].to_owned(),
                ws_operation_fixtures: fixture_ids
                    .into_iter()
                    .zip(fixture_hashes)
                    .map(|(id, hash)| OperationMatrixFixture { id, hash })
                    .collect(),
                finality_boundary: columns[13].to_owned(),
            }
        })
        .collect()
}

fn csv_column(value: &str) -> Vec<String> {
    value.split(',').map(str::to_owned).collect()
}

fn private_stream_ops(
    venue: &shared_types::ExchangeWsVenue,
) -> [(&'static str, &ExchangeWsOperation); 5] {
    [
        ("account_stream", &venue.account_stream),
        ("position_stream", &venue.position_stream),
        ("fill_stream", &venue.fill_stream),
        ("order_stream", &venue.order_stream),
        ("order_status", &venue.order_status),
    ]
}

fn ws_user_test_source(venue: &str) -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let files: &[&str] = match venue {
        "binance" => &["binance_ws_user_tests.rs"],
        "okx" => &["okx_ws_user_tests.rs"],
        "bybit" => &["bybit_ws_user_tests.rs"],
        "bitget" => &["bitget_uta_ws_user_tests.rs"],
        "gate" => &["gate_ws_user_tests.rs"],
        "gate_crossex" => &[
            "gate_crossex_private_data_tests.rs",
            "gate_crossex_ws_private.rs",
        ],
        "kraken" => &[
            "kraken_spot_private_data.rs",
            "kraken_futures_private_data.rs",
            "kraken_spot_ws_private.rs",
            "kraken_futures_ws_private.rs",
        ],
        "kucoin" => &["kucoin_ws_user_tests.rs"],
        "hyperliquid" => &["hyperliquid_ws_user_tests.rs"],
        _ => panic!("{venue} has no venue-owned private WS evidence test source"),
    };
    files
        .iter()
        .map(|file| {
            fs::read_to_string(root.join("src/adapters").join(file))
                .unwrap_or_else(|error| panic!("{file}: {error}"))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn ws_operation_test_source(venue: &str, operation: &str) -> String {
    match (venue, operation) {
        ("binance", "order_status") => binance_ws_trade_test_source(),
        ("gate", "order_status") => gate_ws_trade_test_source(),
        ("hyperliquid", "order_status") => hyperliquid_ws_info_test_source(),
        _ => ws_user_test_source(venue),
    }
}

fn binance_ws_trade_test_source() -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    fs::read_to_string(root.join("src/adapters/binance_ws_trade_tests.rs"))
        .unwrap_or_else(|error| panic!("binance_ws_trade_tests.rs: {error}"))
}

fn okx_ws_trade_test_source() -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    fs::read_to_string(root.join("src/adapters/okx_ws_trade_tests.rs"))
        .unwrap_or_else(|error| panic!("okx_ws_trade_tests.rs: {error}"))
}

fn gate_ws_trade_test_source() -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    fs::read_to_string(root.join("src/adapters/gate_ws_trade_tests.rs"))
        .unwrap_or_else(|error| panic!("gate_ws_trade_tests.rs: {error}"))
}

fn hyperliquid_ws_trade_test_source() -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    fs::read_to_string(root.join("src/adapters/hyperliquid_ws_trade_tests.rs"))
        .unwrap_or_else(|error| panic!("hyperliquid_ws_trade_tests.rs: {error}"))
}

fn hyperliquid_ws_info_test_source() -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    fs::read_to_string(root.join("src/adapters/hyperliquid_ws_info_tests.rs"))
        .unwrap_or_else(|error| panic!("hyperliquid_ws_info_tests.rs: {error}"))
}

fn bybit_ws_trade_test_source() -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    fs::read_to_string(root.join("src/adapters/bybit_ws_trade_tests.rs"))
        .unwrap_or_else(|error| panic!("bybit_ws_trade_tests.rs: {error}"))
}

fn bitget_uta_ws_trade_test_source() -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    fs::read_to_string(root.join("src/adapters/bitget_uta_ws_trade_tests.rs"))
        .unwrap_or_else(|error| panic!("bitget_uta_ws_trade_tests.rs: {error}"))
}

fn kucoin_ws_user_test_source() -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    fs::read_to_string(root.join("src/adapters/kucoin_ws_user_tests.rs"))
        .unwrap_or_else(|error| panic!("kucoin_ws_user_tests.rs: {error}"))
}

fn assert_test_exists(sources: &str, test_name: Option<&str>, venue: &str, operation_label: &str) {
    let test_name =
        test_name.unwrap_or_else(|| panic!("{venue} {operation_label} missing evidence test name"));
    assert!(
        has_runnable_test(sources, test_name),
        "{venue} {operation_label} references missing runnable test {test_name}"
    );
}

fn assert_parser_test_includes_fixture(
    sources: &str,
    test_name: Option<&str>,
    fixture: &str,
    venue: &str,
    operation_label: &str,
) {
    let test_name =
        test_name.unwrap_or_else(|| panic!("{venue} {operation_label} missing parser test name"));
    let body = runnable_test_body(sources, test_name).unwrap_or_else(|| {
        panic!("{venue} {operation_label} references missing runnable parser test {test_name}")
    });

    assert!(
        body_includes_fixture(&body, fixture),
        "{venue} {operation_label} parser test {test_name} must directly include {fixture}"
    );
}

fn has_runnable_test(sources: &str, test_name: &str) -> bool {
    runnable_test_body(sources, test_name).is_some()
}

fn runnable_test_body(sources: &str, test_name: &str) -> Option<String> {
    let sources = strip_rust_block_comments(sources);
    let lines = sources.lines().collect::<Vec<_>>();
    let fn_needle = format!("fn {test_name}(");
    let async_fn_needle = format!("async fn {test_name}(");
    let mut disabled_module_depths = Vec::new();
    let mut brace_depth = 0isize;

    for (index, line) in lines.iter().enumerate() {
        while matches!(disabled_module_depths.last(), Some(depth) if brace_depth < *depth) {
            disabled_module_depths.pop();
        }

        let inside_disabled_parent = !disabled_module_depths.is_empty();
        let delta = brace_delta(line);
        if line_starts_inline_module(line) {
            let module_depth = brace_depth + delta;
            if module_depth > brace_depth
                && (inside_disabled_parent || module_attrs_disable_scope(&lines[..index]))
            {
                disabled_module_depths.push(module_depth);
            }
        }

        let line = line.trim_start();
        if !inside_disabled_parent
            && (line.starts_with(&fn_needle) || line.starts_with(&async_fn_needle))
            && test_attrs_allow_running(&lines[..index])
        {
            return collect_rust_item_body(&lines[index..]);
        }

        brace_depth += delta;
        while matches!(disabled_module_depths.last(), Some(depth) if brace_depth < *depth) {
            disabled_module_depths.pop();
        }
    }

    None
}

fn collect_rust_item_body(lines: &[&str]) -> Option<String> {
    let mut body = String::new();
    let mut brace_depth = 0isize;
    let mut saw_open = false;

    for line in lines {
        let delta = brace_delta(line);
        saw_open |= line.contains('{');
        body.push_str(line);
        body.push('\n');
        brace_depth += delta;
        if saw_open && brace_depth <= 0 {
            return Some(body);
        }
    }

    None
}

fn body_includes_fixture(body: &str, fixture: &str) -> bool {
    include_macro_string_literals(body).any(|literal| {
        let normalized = literal.trim_start_matches('/');
        normalized.ends_with(fixture)
    })
}

fn include_macro_string_literals(body: &str) -> impl Iterator<Item = String> + '_ {
    let mut literals = Vec::new();
    let bytes = body.as_bytes();
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'/') {
            index = skip_line_comment(body, index);
            continue;
        }
        if let Some((_, end)) = rust_string_literal_at(body, index) {
            index = end;
            continue;
        }
        let Some(macro_len) = include_macro_len(&body[index..]) else {
            index += 1;
            continue;
        };
        let Some(open_index) = first_open_paren_after(body, index + macro_len) else {
            index += macro_len;
            continue;
        };
        let Some(close_index) = matching_close_paren(body, open_index) else {
            index += macro_len;
            continue;
        };

        literals.extend(rust_string_literals(&body[open_index + 1..close_index]));
        index = close_index + 1;
    }

    literals.into_iter()
}

fn include_macro_len(text: &str) -> Option<usize> {
    ["include_str!", "include_bytes!"]
        .into_iter()
        .find_map(|needle| text.starts_with(needle).then_some(needle.len()))
}

fn first_open_paren_after(text: &str, mut index: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    while index < bytes.len() && bytes[index].is_ascii_whitespace() {
        index += 1;
    }
    (bytes.get(index) == Some(&b'(')).then_some(index)
}

fn matching_close_paren(text: &str, open_index: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut depth = 0usize;
    let mut index = open_index;

    while index < bytes.len() {
        if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'/') {
            index = skip_line_comment(text, index);
            continue;
        }
        if let Some((_, end)) = rust_string_literal_at(text, index) {
            index = end;
            continue;
        }

        match bytes[index] {
            b'(' => depth += 1,
            b')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
        index += 1;
    }

    None
}

fn rust_string_literals(text: &str) -> Vec<String> {
    let mut literals = Vec::new();
    let bytes = text.as_bytes();
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'/') {
            index = skip_line_comment(text, index);
            continue;
        }
        if let Some((literal, end)) = rust_string_literal_at(text, index) {
            literals.push(literal);
            index = end;
            continue;
        }
        index += 1;
    }

    literals
}

fn rust_string_literal_at(text: &str, index: usize) -> Option<(String, usize)> {
    if text.as_bytes().get(index) == Some(&b'"') {
        return normal_string_literal_at(text, index);
    }
    raw_string_literal_at(text, index)
}

fn normal_string_literal_at(text: &str, index: usize) -> Option<(String, usize)> {
    let bytes = text.as_bytes();
    let mut value = String::new();
    let mut cursor = index + 1;

    while cursor < bytes.len() {
        match bytes[cursor] {
            b'\\' => {
                if let Some(next) = bytes.get(cursor + 1) {
                    value.push(char::from(*next));
                }
                cursor += 2;
            }
            b'"' => return Some((value, cursor + 1)),
            byte => {
                value.push(char::from(byte));
                cursor += 1;
            }
        }
    }

    None
}

fn raw_string_literal_at(text: &str, index: usize) -> Option<(String, usize)> {
    let bytes = text.as_bytes();
    if bytes.get(index) != Some(&b'r') {
        return None;
    }

    let mut cursor = index + 1;
    while bytes.get(cursor) == Some(&b'#') {
        cursor += 1;
    }
    if bytes.get(cursor) != Some(&b'"') {
        return None;
    }

    let hashes = cursor - index - 1;
    let body_start = cursor + 1;
    let end_marker = format!("\"{}", "#".repeat(hashes));
    text[body_start..].find(&end_marker).map(|offset| {
        let body_end = body_start + offset;
        (
            text[body_start..body_end].to_owned(),
            body_end + end_marker.len(),
        )
    })
}

fn skip_line_comment(text: &str, index: usize) -> usize {
    text[index..]
        .find('\n')
        .map(|offset| index + offset + 1)
        .unwrap_or(text.len())
}

fn line_starts_inline_module(line: &str) -> bool {
    let line = line.trim_start();
    let line = if let Some(rest) = line.strip_prefix("pub ") {
        rest.trim_start()
    } else if let Some(rest) = line.strip_prefix("pub(") {
        rest.find(')')
            .map(|end| rest[end + 1..].trim_start())
            .unwrap_or(line)
    } else {
        line
    };

    line.starts_with("mod ") && line.contains('{')
}

fn module_attrs_disable_scope(lines_before_mod: &[&str]) -> bool {
    attr_blocks_before(lines_before_mod)
        .iter()
        .any(|attr| attr_disables_parent_scope(attr))
}

fn attr_disables_parent_scope(attr: &str) -> bool {
    let compact = attr.split_whitespace().collect::<String>();
    if compact == "#[cfg(test)]" {
        return false;
    }

    compact.starts_with("#[ignore")
        || compact.starts_with("#[should_panic")
        || compact.starts_with("#[cfg(")
        || (compact.starts_with("#[cfg_attr(")
            && (compact.contains("ignore")
                || compact.contains("should_panic")
                || compact.contains("cfg(")))
}

fn brace_delta(line: &str) -> isize {
    let bytes = line.as_bytes();
    let mut index = 0;
    let mut delta = 0;

    while index < bytes.len() {
        if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'/') {
            break;
        }

        if bytes[index] == b'r' {
            let mut quote_index = index + 1;
            while bytes.get(quote_index) == Some(&b'#') {
                quote_index += 1;
            }
            if bytes.get(quote_index) == Some(&b'"') {
                let hash_count = quote_index.saturating_sub(index + 1);
                let mut end = quote_index + 1;
                while end < bytes.len() {
                    let closes_raw = bytes[end] == b'"'
                        && (0..hash_count).all(|offset| bytes.get(end + 1 + offset) == Some(&b'#'));
                    if closes_raw {
                        index = end + 1 + hash_count;
                        break;
                    }
                    end += 1;
                }
                if end >= bytes.len() {
                    break;
                }
                continue;
            }
        }

        if bytes[index] == b'"' {
            index += 1;
            while index < bytes.len() {
                if bytes[index] == b'\\' {
                    index += 2;
                } else if bytes[index] == b'"' {
                    index += 1;
                    break;
                } else {
                    index += 1;
                }
            }
            continue;
        }

        match bytes[index] {
            b'{' => delta += 1,
            b'}' => delta -= 1,
            _ => {}
        }
        index += 1;
    }

    delta
}

fn strip_rust_block_comments(source: &str) -> String {
    let mut stripped = String::with_capacity(source.len());
    let mut chars = source.chars().peekable();
    let mut block_depth = 0usize;

    while let Some(ch) = chars.next() {
        if block_depth > 0 {
            if ch == '/' && chars.peek() == Some(&'*') {
                chars.next();
                block_depth += 1;
            } else if ch == '*' && chars.peek() == Some(&'/') {
                chars.next();
                block_depth -= 1;
            } else if ch == '\n' {
                stripped.push('\n');
            }
            continue;
        }

        if ch == '/' && chars.peek() == Some(&'*') {
            chars.next();
            block_depth = 1;
            continue;
        }

        stripped.push(ch);
    }

    stripped
}

fn test_attrs_allow_running(lines_before_fn: &[&str]) -> bool {
    let attrs = attr_blocks_before(lines_before_fn);
    let mut saw_test = false;
    let mut invalid = false;

    for attr in attrs {
        let compact = attr.split_whitespace().collect::<String>();
        saw_test |= compact == "#[test]"
            || compact.starts_with("#[test(")
            || compact == "#[tokio::test]"
            || compact.starts_with("#[tokio::test(");
        invalid |= compact.starts_with("#[ignore")
            || compact.starts_with("#[should_panic")
            || compact.starts_with("#[cfg(")
            || (compact.starts_with("#[cfg_attr(")
                && (compact.contains("ignore")
                    || compact.contains("should_panic")
                    || compact.contains("cfg(")));
    }

    saw_test && !invalid
}

fn attr_blocks_before(lines_before_fn: &[&str]) -> Vec<String> {
    let mut attrs = Vec::new();
    let mut index = lines_before_fn.len();

    while index > 0 {
        index -= 1;
        let line = lines_before_fn[index].trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with("//") {
            continue;
        }
        if !(line.starts_with("#[") || line.ends_with(']') || line.ends_with(")]")) {
            break;
        }

        let attr = if line.starts_with("#[") {
            line.to_owned()
        } else {
            let mut block = Vec::new();
            loop {
                let part = lines_before_fn[index].trim();
                block.push(part);
                if part.starts_with("#[") || index == 0 {
                    break;
                }
                index -= 1;
            }
            block.reverse();
            block.join(" ")
        };

        if !attr.starts_with("#[") {
            break;
        }

        attrs.push(attr);
    }

    attrs.reverse();
    attrs
}

const EVIDENCE_LOOKUP_FUNCTION_ATTR_SOURCE: &str = concat!(
    "#[test]\n",
    "fn accepted_plain_test() {}\n",
    "\n",
    "#[tokio::test]\n",
    "async fn accepted_tokio_test() {}\n",
    "\n",
    "mod enabled_parent {\n",
    "    #[test]\n",
    "    fn accepted_nested_test() {}\n",
    "}\n",
    "\n",
    "#[ignore]\n",
    "#[test]\n",
    "fn ignored_test() {}\n",
    "\n",
    "#[should_panic]\n",
    "#[test]\n",
    "fn panic_expected_test() {}\n",
    "\n",
    "#[cfg_attr(any(), should_panic)]\n",
    "#[test]\n",
    "fn cfg_panic_expected_test() {}\n",
    "\n",
    "#[cfg(any())]\n",
    "// cfg reason must not hide the disabling attribute\n",
    "#[test]\n",
    "fn cfg_disabled_with_comment_gap_test() {}\n",
    "\n",
    "#[ignore]\n",
    "// ignore reason must not hide the disabling attribute\n",
    "#[test]\n",
    "fn ignored_with_comment_gap_test() {}\n",
    "\n",
    "#[cfg_attr(all(), cfg(any()))]\n",
    "// cfg_attr reason must not hide the disabling attribute\n",
    "#[test]\n",
    "fn cfg_attr_cfg_comment_gap_test() {}\n",
    "\n",
    "#[cfg(any())]\n",
    "#[test]\n",
    "fn cfg_disabled_test() {}\n",
    "\n",
    "#[cfg_attr(\n",
    "    all(),\n",
    "    cfg(any())\n",
    ")]\n",
    "#[test]\n",
    "fn cfg_attr_cfg_disabled_test() {}\n",
    "\n",
    "#[cfg_attr(\n",
    "    any(),\n",
    "    ignore\n",
    ")]\n",
    "#[test]\n",
    "fn multiline_cfg_ignore_test() {}\n",
    "\n",
    "#[cfg_attr(\n",
    "    any(),\n",
    "    should_panic\n",
    ")]\n",
    "#[test]\n",
    "fn multiline_cfg_panic_expected_test() {}\n",
    "\n",
);

const EVIDENCE_LOOKUP_PARENT_SCOPE_SOURCE: &str = concat!(
    "#[cfg(any())]\n",
    "mod disabled_parent {\n",
    "    #[test]\n",
    "    fn parent_cfg_disabled_test() {}\n",
    "}\n",
    "\n",
    "#[cfg_attr(all(), cfg(any()))]\n",
    "mod cfg_attr_disabled_parent {\n",
    "    #[test]\n",
    "    fn parent_cfg_attr_disabled_test() {}\n",
    "}\n",
    "\n",
    "#[cfg(any())]\n",
    "// cfg reason must not hide the disabling parent module\n",
    "mod disabled_parent_with_comment_gap {\n",
    "    #[test]\n",
    "    fn parent_cfg_comment_gap_test() {}\n",
    "}\n",
    "\n",
    "#[cfg_attr(any(), ignore)]\n",
    "mod ignored_parent {\n",
    "    #[test]\n",
    "    fn parent_cfg_attr_ignore_test() {}\n",
    "}\n",
    "\n",
    "#[cfg(test)]\n",
    "mod cfg_test_parent {\n",
    "    #[test]\n",
    "    fn accepted_cfg_test_nested_test() {}\n",
    "}\n",
    "\n",
    "/*\n",
    "#[test]\n",
    "fn block_commented_test() {}\n",
    "*/\n",
    "\n",
    "/* outer\n",
    "/* nested */\n",
    "#[test]\n",
    "fn nested_block_commented_test() {}\n",
    "*/\n",
);

fn evidence_lookup_regression_source() -> String {
    [
        EVIDENCE_LOOKUP_FUNCTION_ATTR_SOURCE,
        EVIDENCE_LOOKUP_PARENT_SCOPE_SOURCE,
    ]
    .concat()
}

fn assert_runnable_evidence_tests(sources: &str) {
    for test_name in [
        "accepted_plain_test",
        "accepted_tokio_test",
        "accepted_nested_test",
        "accepted_cfg_test_nested_test",
    ] {
        assert!(has_runnable_test(sources, test_name), "{test_name}");
    }
}

fn assert_non_runnable_evidence_tests(sources: &str) {
    for test_name in [
        "ignored_test",
        "panic_expected_test",
        "cfg_panic_expected_test",
        "cfg_disabled_with_comment_gap_test",
        "ignored_with_comment_gap_test",
        "cfg_attr_cfg_comment_gap_test",
        "cfg_disabled_test",
        "cfg_attr_cfg_disabled_test",
        "multiline_cfg_ignore_test",
        "multiline_cfg_panic_expected_test",
        "parent_cfg_disabled_test",
        "parent_cfg_attr_disabled_test",
        "parent_cfg_comment_gap_test",
        "parent_cfg_attr_ignore_test",
        "block_commented_test",
        "nested_block_commented_test",
    ] {
        assert!(!has_runnable_test(sources, test_name), "{test_name}");
    }
}

fn fixture_lookup_regression_source() -> &'static str {
    concat!(
        "#[test]\n",
        "fn direct_fixture_ok() {\n",
        "    let _ = include_str!(\"../../fixtures/venue/direct.json\");\n",
        "}\n",
        "\n",
        "#[test]\n",
        "fn multiline_fixture_ok() {\n",
        "    let _ = include_bytes!(\n",
        "        \"../../fixtures/venue/multiline.json\"\n",
        "    );\n",
        "}\n",
        "\n",
        "#[test]\n",
        "fn concat_fixture_ok() {\n",
        "    let _ = include_str!(concat!(\n",
        "        env!(\"CARGO_MANIFEST_DIR\"),\n",
        "        \"/fixtures/venue/concat.json\"\n",
        "    ));\n",
        "}\n",
        "\n",
        "const INDIRECT_FIXTURE: &str = include_str!(\"../../fixtures/venue/indirect.json\");\n",
        "\n",
        "#[test]\n",
        "fn indirect_const_fixture_fails() {\n",
        "    let _ = INDIRECT_FIXTURE;\n",
        "}\n",
        "\n",
        "fn helper_fixture() -> &'static str {\n",
        "    include_str!(\"../../fixtures/venue/helper.json\")\n",
        "}\n",
        "\n",
        "#[test]\n",
        "fn helper_fixture_fails() {\n",
        "    let _ = helper_fixture();\n",
        "}\n",
        "\n",
        "#[test]\n",
        "fn comment_fixture_fails() {\n",
        "    // include_str!(\"../../fixtures/venue/comment.json\");\n",
        "}\n",
        "\n",
        "#[test]\n",
        "fn string_fixture_fails() {\n",
        "    let _ = \"include_str!(\\\"../../fixtures/venue/string.json\\\")\";\n",
        "}\n",
        "\n",
        "#[test]\n",
        "fn wrong_fixture_fails() {\n",
        "    let _ = include_str!(\"../../fixtures/venue/wrong.json\");\n",
        "}\n",
        "\n",
        "#[cfg(any())]\n",
        "mod disabled_parent {\n",
        "    #[test]\n",
        "    fn disabled_fixture_fails() {\n",
        "        let _ = include_str!(\"../../fixtures/venue/disabled.json\");\n",
        "    }\n",
        "}\n",
    )
}

#[test]
fn evidence_test_lookup_rejects_skipped_and_panic_expected_tests() {
    let sources = evidence_lookup_regression_source();
    assert_runnable_evidence_tests(&sources);
    assert_non_runnable_evidence_tests(&sources);
}

#[test]
fn evidence_fixture_lookup_requires_direct_include_macro() {
    let sources = fixture_lookup_regression_source();
    for (test_name, fixture) in [
        ("direct_fixture_ok", "fixtures/venue/direct.json"),
        ("multiline_fixture_ok", "fixtures/venue/multiline.json"),
        ("concat_fixture_ok", "fixtures/venue/concat.json"),
    ] {
        let body = runnable_test_body(sources, test_name).unwrap_or_else(|| panic!("{test_name}"));
        assert!(body_includes_fixture(&body, fixture), "{test_name}");
    }

    for (test_name, fixture) in [
        ("direct_fixture_ok", "fixtures/venue/wrong.json"),
        (
            "indirect_const_fixture_fails",
            "fixtures/venue/indirect.json",
        ),
        ("helper_fixture_fails", "fixtures/venue/helper.json"),
        ("comment_fixture_fails", "fixtures/venue/comment.json"),
        ("string_fixture_fails", "fixtures/venue/string.json"),
        ("wrong_fixture_fails", "fixtures/venue/expected.json"),
    ] {
        let body = runnable_test_body(sources, test_name).unwrap_or_else(|| panic!("{test_name}"));
        assert!(!body_includes_fixture(&body, fixture), "{test_name}");
    }

    assert!(runnable_test_body(sources, "disabled_fixture_fails").is_none());
}
