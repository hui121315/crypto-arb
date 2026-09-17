use super::super::{
    credential_validation_rows, order_write_runtime_rows, run_finality_runtime_rows,
    SOURCE_LIVE_ORDER_PROOF_RUNTIME, SOURCE_RUN_FINALITY_RUNTIME, UNRECORDED_EVIDENCE_MARKER,
};
use super::{credential, credential_probe, live_order_proof_health};
use crate::services::live_order_proof_health::LiveOrderProofProblem;
use crate::services::venue_credentials;
use shared_types::{
    VenueCredentialProbeStatus, VenueCredentialValidationEvidence, VenueCredentialValidationStatus,
    VenueOperationStatus,
};

#[test]
fn safe_order_permission_probe_matrix_uses_registered_endpoint_evidence_without_live_write() {
    for case in [
        (
            "binance",
            "binance_usdm_futures_order_test_cancel_no_match",
            "binance.POST /fapi/v1/order/test + DELETE /fapi/v1/order",
            "/fapi/v1/order/test",
        ),
        (
            "okx",
            "okx_order_precheck",
            "okx.POST /api/v5/trade/order-precheck",
            "/api/v5/trade/order-precheck",
        ),
        (
            "bybit",
            "bybit_linear_order_pre_check_no_match",
            "bybit.POST /v5/order/pre-check",
            "/v5/order/pre-check",
        ),
        (
            "bitget",
            "bitget_uta_cancel_no_match",
            "bitget.POST /api/v3/trade/cancel-order",
            "/api/v3/trade/cancel-order",
        ),
        (
            "gate",
            "gate_futures_cancel_no_match",
            "gate.DELETE /api/v4/futures/usdt/orders/{order_id}",
            "/api/v4/futures/usdt/orders/{order_id}",
        ),
        (
            "kucoin",
            "kucoin_classic_futures_order_test_cancel_no_match",
            "kucoin.POST /api/v1/orders/test + DELETE /api/v1/orders/client-order/{clientOid}",
            "/api/v1/orders/test",
        ),
    ] {
        assert_safe_probe_evidence(case);
    }
}

fn assert_safe_probe_evidence((venue, scope, source, expected_path): (&str, &str, &str, &str)) {
    let probe = credential_probe(
        "order_permission",
        VenueCredentialProbeStatus::Unknown,
        scope,
        source,
    );
    let rows = rows_for_probe(venue, probe);
    let row = rows
        .iter()
        .find(|row| row.operation == "credential_probe:order_permission")
        .expect("order permission row");
    let evidence = row.evidence.as_ref().expect("safe probe evidence");

    assert_eq!(row.status, VenueOperationStatus::Unknown);
    assert_eq!(row.rows, Some(0));
    assert_eq!(evidence.path, expected_path);
    assert_ne!(evidence.checked_at, UNRECORDED_EVIDENCE_MARKER);
    assert!(evidence.use_cases.iter().any(|item| item == "trade_write"));
    assert!(evidence.data_kinds.iter().any(|item| item == "order_ack"));
    assert!(evidence
        .request_context
        .iter()
        .any(|item| item == "does_not_grant_live_write=true"));
    assert_safe_probe_context_does_not_claim_live_write(row, evidence);
}

#[test]
fn hyperliquid_noop_order_permission_probe_does_not_reuse_place_order_evidence() {
    let probe = credential_probe(
        "order_permission",
        VenueCredentialProbeStatus::Unknown,
        "hyperliquid_noop",
        "hyperliquid.POST /exchange action=noop",
    );
    let rows = rows_for_probe("hyperliquid", probe);
    let row = rows
        .iter()
        .find(|row| row.operation == "credential_probe:order_permission")
        .expect("order permission row");
    let evidence = row.evidence.as_ref().expect("noop fallback evidence");

    assert_eq!(row.status, VenueOperationStatus::Unknown);
    assert_eq!(evidence.path, "hyperliquid_noop");
    assert_eq!(evidence.checked_at, UNRECORDED_EVIDENCE_MARKER);
    assert_eq!(evidence.doc_version, UNRECORDED_EVIDENCE_MARKER);
    assert_eq!(evidence.schema_hash, UNRECORDED_EVIDENCE_MARKER);
    assert_eq!(evidence.fixture_id, UNRECORDED_EVIDENCE_MARKER);
    assert_eq!(evidence.parser_test, UNRECORDED_EVIDENCE_MARKER);
    assert_eq!(evidence.request_builder_test, UNRECORDED_EVIDENCE_MARKER);
    assert_eq!(evidence.auth_kind, UNRECORDED_EVIDENCE_MARKER);
    assert!(evidence.doc_urls.is_empty());
    assert_eq!(evidence.use_cases, vec!["credential_validation"]);
    assert_eq!(evidence.data_kinds, vec!["order_permission"]);
    assert!(evidence
        .request_context
        .iter()
        .any(|item| item == "does_not_grant_live_write=true"));
    assert_safe_probe_context_does_not_claim_live_write(row, evidence);
}

#[test]
fn safe_order_permission_probe_does_not_satisfy_live_runtime_rows() {
    let permission_row = safe_binance_order_permission_row();
    let permission_evidence = permission_row
        .evidence
        .as_ref()
        .expect("safe order permission evidence");

    assert_eq!(permission_row.status, VenueOperationStatus::Unknown);
    assert_eq!(permission_row.source, "credential_validation");
    assert_safe_probe_context_does_not_claim_live_write(&permission_row, permission_evidence);
    assert_safe_probe_does_not_satisfy_order_write_runtime();
    assert_safe_probe_does_not_satisfy_order_finality_runtime();
}

fn safe_binance_order_permission_row() -> shared_types::VenueOperationHealth {
    let probe = credential_probe(
        "order_permission",
        VenueCredentialProbeStatus::Unknown,
        "binance_usdm_futures_order_test_cancel_no_match",
        "binance.POST /fapi/v1/order/test + DELETE /fapi/v1/order",
    );
    rows_for_probe("binance", probe)
        .into_iter()
        .find(|row| row.operation == "credential_probe:order_permission")
        .expect("order permission row")
}

fn assert_safe_probe_does_not_satisfy_order_write_runtime() {
    let credential = credential(true, true);
    let order_write_rows =
        order_write_runtime_rows(std::slice::from_ref(&credential), Vec::new(), 2_000);
    let order_write_row = &order_write_rows[0];
    let order_write_evidence = order_write_row
        .evidence
        .as_ref()
        .expect("missing live proof evidence");

    assert_eq!(order_write_row.operation, "order_write");
    assert_eq!(order_write_row.status, VenueOperationStatus::Unknown);
    assert_eq!(order_write_row.source, SOURCE_LIVE_ORDER_PROOF_RUNTIME);
    assert_eq!(order_write_row.rows, Some(0));
    assert!(order_write_evidence
        .request_context
        .iter()
        .any(|item| item == "live_place_remote_proof=missing"));
    assert!(order_write_evidence
        .request_context
        .iter()
        .any(|item| item == "live_cancel_remote_proof=missing"));
    assert!(!order_write_evidence
        .request_context
        .iter()
        .any(|item| item == "does_not_grant_live_write=true"));
}

fn assert_safe_probe_does_not_satisfy_order_finality_runtime() {
    let credential = credential(true, true);
    let finality_rows =
        run_finality_runtime_rows(std::slice::from_ref(&credential), Vec::new(), 2_000);
    let finality_row = &finality_rows[0];

    assert_eq!(finality_row.operation, "order_finality");
    assert_eq!(finality_row.status, VenueOperationStatus::Unknown);
    assert_eq!(finality_row.source, SOURCE_RUN_FINALITY_RUNTIME);
    assert_eq!(finality_row.requested, None);
    assert_eq!(finality_row.rows, None);
    assert!(finality_row.evidence.is_none());
}

#[test]
fn order_write_ok_context_excludes_stale_last_problem() {
    let credential = credential(true, true);
    let mut snapshot = live_order_proof_health("binance", VenueOperationStatus::Ok);
    snapshot.last_problem = Some(LiveOrderProofProblem {
        message: "previous submit failed".to_owned(),
        source: "adapter.submit_order".to_owned(),
        request_id: Some("req-old-problem".to_owned()),
        retry_after_ms: Some(60_000),
        status: Some(502),
        observed_at_ms: 9,
    });

    let rows = order_write_runtime_rows(std::slice::from_ref(&credential), vec![snapshot], 20);
    let row = &rows[0];
    let evidence = row.evidence.as_ref().expect("order write evidence");

    assert_eq!(row.status, VenueOperationStatus::Ok);
    assert!(row.problem.is_none());
    assert!(!evidence
        .request_context
        .iter()
        .any(|item| item.starts_with("last_problem_")));
}

fn assert_safe_probe_context_does_not_claim_live_write(
    row: &shared_types::VenueOperationHealth,
    evidence: &shared_types::VenueOperationEvidence,
) {
    assert_eq!(row.operation, "credential_probe:order_permission");
    assert_eq!(row.source, "credential_validation");
    assert_eq!(row.status, VenueOperationStatus::Unknown);
    assert_eq!(row.rows, Some(0));
    assert!(!evidence
        .request_context
        .iter()
        .any(|item| item == "probe_source=live_order_proof_runtime"));
    assert!(!evidence
        .request_context
        .iter()
        .any(|item| item.contains(".order_write.live_place_cancel")));
    assert!(!evidence
        .request_context
        .iter()
        .any(|item| item.contains("live_place_remote_proof")));
    assert!(!evidence
        .request_context
        .iter()
        .any(|item| item.contains("live_cancel_remote_proof")));
}

fn rows_for_probe(
    venue: &str,
    probe: shared_types::VenueCredentialProbe,
) -> Vec<shared_types::VenueOperationHealth> {
    credential_validation_rows(
        &[],
        vec![venue_credentials::VenueCredentialValidationSnapshot {
            venue: venue.to_owned(),
            credential_fingerprint: None,
            evidence: VenueCredentialValidationEvidence {
                status: VenueCredentialValidationStatus::ReadOnlyOk,
                checked_at_ms: 1_000,
                probes: vec![probe],
                permission_evidence: Vec::new(),
            },
        }],
        1_500,
    )
}
