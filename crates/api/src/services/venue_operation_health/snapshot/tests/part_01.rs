#[path = "../../tests/safe_probe_endpoint_tests.rs"]
mod safe_probe_endpoint_tests;

use super::*;
use crate::services::market_data::MarketSource;
use shared_types::ExchangeProblem;
use shared_types::HistoryMigrationStatus;
use shared_types::VenueCredentialField;

#[test]
fn configured_credentials_stay_unknown_not_ok() {
    let rows = credential_rows(&[credential(true, true)], 10);

    assert_eq!(rows[0].status, VenueOperationStatus::Unknown);
    assert_eq!(rows[0].configured, Some(true));
    assert_eq!(rows[0].supported, Some(true));
}

#[test]
fn kraken_accepts_either_complete_product_credential_pair() {
    let spot = kraken_credential_fields([true, true, false, false]);
    let futures = kraken_credential_fields([false, false, true, true]);
    let incomplete = kraken_credential_fields([true, false, true, false]);

    assert!(credentials_configured(&spot));
    assert!(credentials_configured(&futures));
    assert!(!credentials_configured(&incomplete));
    assert!(missing_credential_fields(&incomplete).contains("至少一套完整凭证"));
}

fn kraken_credential_fields(configured: [bool; 4]) -> VenueCredentialStatus {
    let definitions = [
        ("spot_api_key", "Spot API Key", "KRAKEN_SPOT_API_KEY"),
        (
            "spot_api_secret",
            "Spot API Secret",
            "KRAKEN_SPOT_API_SECRET",
        ),
        (
            "futures_api_key",
            "Futures API Key",
            "KRAKEN_FUTURES_API_KEY",
        ),
        (
            "futures_api_secret",
            "Futures API Secret",
            "KRAKEN_FUTURES_API_SECRET",
        ),
    ];
    VenueCredentialStatus {
        venue: "kraken".to_owned(),
        label: "Kraken".to_owned(),
        fields: definitions
            .into_iter()
            .zip(configured)
            .map(|((key, label, env_key), configured)| VenueCredentialField {
                key: key.to_owned(),
                label: label.to_owned(),
                env_key: env_key.to_owned(),
                configured,
                secret: true,
                required: false,
                source: if configured {
                    shared_types::VenueCredentialFieldSource::Runtime
                } else {
                    shared_types::VenueCredentialFieldSource::Missing
                },
            })
            .collect(),
        public_market: true,
        private_read: true,
        testnet_write: false,
        live_write: true,
        note: String::new(),
        missing_fields: Vec::new(),
        validation_evidence: None,
    }
}

#[test]
fn credential_validation_probe_rows_preserve_unknown_order_permission() {
    let rows = credential_validation_rows(
        &[credential(true, true)],
        vec![venue_credentials::VenueCredentialValidationSnapshot {
            venue: "binance".to_owned(),
            credential_fingerprint: None,
            evidence: VenueCredentialValidationEvidence {
                status: VenueCredentialValidationStatus::ReadOnlyOk,
                checked_at_ms: 1_000,
                probes: vec![
                    credential_probe(
                        "balance_read",
                        VenueCredentialProbeStatus::Ok,
                        "USDT",
                        "exchange_adapter.get_balance",
                    ),
                    credential_probe(
                        "order_permission",
                        VenueCredentialProbeStatus::Unknown,
                        "place_cancel_order_stream",
                        "not_probed",
                    ),
                ],
                permission_evidence: Vec::new(),
            },
        }],
        1_500,
    );

    let balance = rows
        .iter()
        .find(|row| row.operation == "credential_probe:balance_read")
        .expect("balance probe row");
    assert_eq!(balance.status, VenueOperationStatus::Ok);
    assert_eq!(balance.source, SOURCE_CREDENTIAL_VALIDATION);
    assert_eq!(balance.requested, Some(1));
    assert_eq!(balance.rows, Some(1));
    assert_eq!(balance.freshness_ms, Some(500));

    let order = rows
        .iter()
        .find(|row| row.operation == "credential_probe:order_permission")
        .expect("order permission row");
    assert_eq!(order.status, VenueOperationStatus::Unknown);
    assert_eq!(order.configured, Some(true));
    assert_eq!(order.supported, Some(true));
    assert_eq!(order.requested, Some(1));
    assert_eq!(order.rows, Some(0));
    assert!(order.error.is_none());
    assert!(order
        .evidence
        .as_ref()
        .is_some_and(|evidence| evidence.path == "place_cancel_order_stream"));
}

#[test]
fn credential_private_read_probe_uses_endpoint_evidence_registry() {
    let rows = credential_validation_rows(
        &[credential(true, true)],
        vec![venue_credentials::VenueCredentialValidationSnapshot {
            venue: "binance".to_owned(),
            credential_fingerprint: None,
            evidence: VenueCredentialValidationEvidence {
                status: VenueCredentialValidationStatus::ReadOnlyOk,
                checked_at_ms: 1_000,
                probes: vec![
                    credential_probe(
                        "balance_read",
                        VenueCredentialProbeStatus::Ok,
                        "USDT",
                        "exchange_adapter.get_balance",
                    ),
                    credential_probe(
                        "open_orders_read",
                        VenueCredentialProbeStatus::Ok,
                        "private_read.open_orders",
                        "exchange_adapter.get_open_orders",
                    ),
                ],
                permission_evidence: Vec::new(),
            },
        }],
        1_500,
    );

    let balance = rows
        .iter()
        .find(|row| row.operation == "credential_probe:balance_read")
        .expect("balance probe row");
    let balance_evidence = balance.evidence.as_ref().expect("balance evidence");
    assert_eq!(balance_evidence.path, "/fapi/v3/balance");
    assert_ne!(balance_evidence.checked_at, UNRECORDED_EVIDENCE_MARKER);
    assert!(balance_evidence
        .use_cases
        .iter()
        .any(|case| case == "private_read"));

    let open_orders = rows
        .iter()
        .find(|row| row.operation == "credential_probe:open_orders_read")
        .expect("open orders probe row");
    assert_eq!(
        open_orders
            .evidence
            .as_ref()
            .map(|evidence| evidence.path.as_str()),
        Some("/fapi/v1/openOrders")
    );
}

#[test]
fn safe_cancel_order_permission_probe_uses_trade_write_endpoint_evidence() {
    let probe = credential_probe(
        "order_permission",
        VenueCredentialProbeStatus::Unknown,
        "bitget_uta_cancel_no_match",
        "bitget.POST /api/v3/trade/cancel-order",
    );
    let rows = credential_validation_rows(
        &[],
        vec![venue_credentials::VenueCredentialValidationSnapshot {
            venue: "bitget".to_owned(),
            credential_fingerprint: None,
            evidence: VenueCredentialValidationEvidence {
                status: VenueCredentialValidationStatus::ReadOnlyOk,
                checked_at_ms: 1_000,
                probes: vec![probe],
                permission_evidence: Vec::new(),
            },
        }],
        1_500,
    );

    let row = rows
        .iter()
        .find(|row| row.operation == "credential_probe:order_permission")
        .expect("order permission row");
    let evidence = row.evidence.as_ref().expect("safe cancel evidence");

    assert_eq!(row.status, VenueOperationStatus::Unknown);
    assert_eq!(row.rows, Some(0));
    assert_eq!(evidence.path, "/api/v3/trade/cancel-order");
    assert_ne!(evidence.checked_at, UNRECORDED_EVIDENCE_MARKER);
    assert!(evidence.use_cases.iter().any(|item| item == "trade_write"));
    assert!(evidence.data_kinds.iter().any(|item| item == "order_ack"));
    assert!(evidence
        .request_context
        .iter()
        .any(|item| item == "does_not_grant_live_write=true"));
}

#[test]
fn unknown_credential_probe_is_not_reported_supported() {
    let venue = credential(true, true);

    assert!(!credential_probe_supported(&venue, "made_up"));
}

#[test]
fn missing_credentials_block_supported_operations() {
    let rows = credential_rows(&[credential(true, false)], 10);

    assert_eq!(rows[0].status, VenueOperationStatus::Blocked);
    assert!(rows[0].error.is_some());
}

#[test]
fn market_quality_maps_rate_limit_to_blocked() {
    assert_eq!(
        market_status(MarketQuality::RateLimited),
        VenueOperationStatus::Blocked
    );
    assert_eq!(
        market_status(MarketQuality::Fresh),
        VenueOperationStatus::Ok
    );
}

#[test]
fn market_row_preserves_structured_exchange_problem() {
    let row = market_row(
        MarketRuntimeHealth {
            venue: "hyperliquid:xyz".to_owned(),
            operation: "rest_orderbooks",
            quality: MarketQuality::RateLimited,
            source: MarketSource::RestBaseline,
            requested: 1,
            rows: 0,
            retry_after_ms: Some(2_000),
            last_error: Some("rate limited".to_owned()),
            problem: Some(
                ExchangeProblem::new("hyperliquid:xyz", "rest_orderbooks", "rate limited")
                    .with_symbol("SNDK")
                    .with_source("exchange-fanout"),
            ),
            observed_at_ms: 10_000,
        },
        11_000,
    );

    let problem = row.problem.expect("structured problem");

    assert_eq!(row.status, VenueOperationStatus::Blocked);
    assert_eq!(row.retry_after_ms, Some(2_000));
    assert_eq!(problem.code, "MARKET_DATA_RATE_LIMITED");
    assert_eq!(problem.retry_after_ms, Some(2_000));
    assert_eq!(problem.source.as_deref(), Some("exchange-fanout"));
    assert!(problem
        .details
        .as_ref()
        .and_then(|details| details.get("symbol"))
        .is_some_and(|symbol| symbol.as_str() == Some("SNDK")));
}
