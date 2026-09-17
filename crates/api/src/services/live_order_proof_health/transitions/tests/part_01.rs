use super::*;

#[test]
fn place_ack_alone_warns_without_cancel_grant() {
    let store = LiveOrderProofHealthStore::default();
    store.record_place_ack(sample("binance", "internal-1", 1_000, Some("req-place")));

    let rows = store.snapshot(1_500);
    assert_eq!(rows.len(), 1);
    let row = &rows[0];

    assert_eq!(row.status, VenueOperationStatus::Warn);
    assert_eq!(row.requested, Some(2));
    assert_eq!(row.rows, Some(1));
    assert_eq!(row.freshness_ms, Some(500));
    assert_eq!(row.request_id.as_deref(), Some("req-place"));
    assert!(row.message.contains("仍缺撤单请求"));
}

#[test]
fn cancel_finality_completes_remote_proof() {
    let store = LiveOrderProofHealthStore::default();
    store.record_place_ack(sample("okx", "internal-1", 1_000, Some("req-place")));
    store.record_cancel_requested(sample("okx", "internal-1", 1_200, Some("req-cancel")));
    store.record_cancel_finality(sample("okx", "internal-1", 1_400, Some("req-final")));

    let rows = store.snapshot(1_500);
    assert_eq!(rows.len(), 1);
    let row = &rows[0];

    assert_eq!(row.status, VenueOperationStatus::Ok);
    assert_eq!(row.rows, Some(2));
    assert_eq!(row.freshness_ms, Some(100));
    assert_eq!(row.request_id.as_deref(), Some("req-final"));
    assert!(row.error.is_none());
}

#[test]
fn live_order_proof_runtime_acceptance_requires_matching_place_and_cancel_samples() {
    let store = LiveOrderProofHealthStore::default();
    store.record_place_ack(sample("okx", "internal-place", 1_000, Some("req-place")));
    store.record_cancel_finality(sample("okx", "internal-cancel", 1_200, Some("req-final")));

    let mismatched_rows = store.snapshot(1_300);

    assert_eq!(mismatched_rows.len(), 1);
    assert_eq!(mismatched_rows[0].status, VenueOperationStatus::Warn);
    assert_eq!(mismatched_rows[0].rows, Some(1));
    assert!(mismatched_rows[0].message.contains("身份不一致"));

    store.record_cancel_finality(sample(
        "okx",
        "internal-place",
        1_400,
        Some("req-final-matched"),
    ));
    let accepted_rows = store.snapshot(1_500);

    assert_eq!(accepted_rows[0].status, VenueOperationStatus::Ok);
    assert_eq!(accepted_rows[0].rows, Some(2));
    assert_eq!(
        accepted_rows[0].request_id.as_deref(),
        Some("req-final-matched")
    );
}

#[test]
fn live_order_proof_runtime_rejects_conflicting_retained_identity_family() {
    let store = LiveOrderProofHealthStore::default();
    let mut place = sample("okx", "internal-1", 1_000, Some("req-place"));
    place.exchange_order_id = Some("exchange-place".to_owned());
    let mut cancel = sample("okx", "internal-1", 1_200, Some("req-final"));
    cancel.exchange_order_id = Some("exchange-cancel".to_owned());

    store.record_place_ack(place);
    store.record_cancel_finality(cancel);
    let rows = store.snapshot(1_300);

    assert_eq!(rows[0].status, VenueOperationStatus::Warn);
    assert_eq!(rows[0].rows, Some(1));
    assert!(rows[0].message.contains("身份不一致"));
}

#[test]
fn newer_problem_blocks_until_complete_proof_is_newer() {
    let store = LiveOrderProofHealthStore::default();
    store.record_place_ack(sample("bitget", "internal-1", 1_000, Some("req-place")));
    store.record_cancel_finality(sample("bitget", "internal-1", 1_100, Some("req-final")));
    store.record_problem(LiveOrderProofProblemInput {
        venue: "bitget",
        source: "submit_order",
        message: "exchange rejected live order",
        request_id: Some("req-error".to_owned()),
        retry_after_ms: Some(5_000),
        status: Some(502),
    });

    let blocked_rows = store.snapshot(common::time::now_ms());
    assert_eq!(blocked_rows[0].status, VenueOperationStatus::Blocked);
    assert_eq!(blocked_rows[0].retry_after_ms, Some(5_000));

    let now_ms = common::time::now_ms();
    store.record_place_ack(sample(
        "bitget",
        "internal-2",
        now_ms + 1,
        Some("req-place-2"),
    ));
    store.record_cancel_finality(sample(
        "bitget",
        "internal-2",
        now_ms + 2,
        Some("req-final-2"),
    ));
    let recovered_rows = store.snapshot(now_ms + 3);

    assert_eq!(recovered_rows[0].status, VenueOperationStatus::Ok);
    assert_eq!(recovered_rows[0].request_id.as_deref(), Some("req-final-2"));
}

#[test]
fn newer_place_ack_downgrades_an_older_failure_to_incomplete_warning() {
    let store = LiveOrderProofHealthStore::default();
    store.record_problem(LiveOrderProofProblemInput {
        venue: "kucoin",
        source: "submit_order",
        message: "local position compatibility rejected close",
        request_id: Some("req-error".to_owned()),
        retry_after_ms: None,
        status: Some(502),
    });
    let after_problem = common::time::now_ms() + 1;

    store.record_place_ack(sample(
        "kucoin",
        "internal-recovery-close",
        after_problem,
        Some("req-recovery"),
    ));
    let rows = store.snapshot(after_problem + 1);

    assert_eq!(rows[0].status, VenueOperationStatus::Warn);
    assert_eq!(rows[0].request_id.as_deref(), Some("req-recovery"));
    assert!(rows[0].message.contains("仍缺撤单请求"));
    assert!(!rows[0].message.contains("证明失败"));
}

#[test]
fn recovered_ok_row_does_not_inherit_stale_problem_request_id() {
    let store = LiveOrderProofHealthStore::default();
    store.record_problem(LiveOrderProofProblemInput {
        venue: "gate",
        source: "submit_order",
        message: "exchange rejected live order",
        request_id: Some("req-error".to_owned()),
        retry_after_ms: Some(5_000),
        status: Some(502),
    });

    let now_ms = common::time::now_ms();
    store.record_place_ack(sample("gate", "internal-1", now_ms + 1, None));
    store.record_cancel_finality(sample("gate", "internal-1", now_ms + 2, None));
    let recovered_rows = store.snapshot(now_ms + 3);

    assert_eq!(recovered_rows[0].status, VenueOperationStatus::Ok);
    assert_eq!(recovered_rows[0].request_id, None);
}

#[test]
fn complete_proof_remains_ok_for_current_credential_epoch() {
    let store = LiveOrderProofHealthStore::default();
    store.record_place_ack(sample("hyperliquid", "internal-1", 1_000, None));
    store.record_cancel_finality(sample("hyperliquid", "internal-1", 1_100, None));

    let rows = store.snapshot(86_401_100);

    assert_eq!(rows[0].status, VenueOperationStatus::Ok);
    assert_eq!(rows[0].freshness_ms, Some(86_400_000));
    assert!(rows[0].error.is_none());
}

#[test]
fn unrelated_new_place_ack_does_not_replace_completed_epoch_proof() {
    let store = LiveOrderProofHealthStore::default();
    store.record_place_ack(sample("bitget", "proof-order", 1_000, None));
    store.record_cancel_finality(sample("bitget", "proof-order", 1_100, None));

    store.record_place_ack(sample("bitget", "filled-order", 1_200, None));
    let rows = store.snapshot(1_300);

    assert_eq!(rows[0].status, VenueOperationStatus::Ok);
    assert_eq!(rows[0].place_ack_count, 2);
    assert_eq!(
        rows[0]
            .place_proof
            .as_ref()
            .map(|sample| sample.internal_order_id.as_str()),
        Some("proof-order")
    );
    assert_eq!(
        rows[0]
            .cancel_finality
            .as_ref()
            .map(|sample| sample.internal_order_id.as_str()),
        Some("proof-order")
    );
}

#[test]
fn credential_update_invalidation_removes_exact_and_family_proofs() {
    let store = LiveOrderProofHealthStore::default();
    store.record_place_ack(sample("okx", "internal-okx", 1_000, Some("req-okx-place")));
    store.record_cancel_finality(sample("okx", "internal-okx", 1_100, Some("req-okx-final")));
    store.record_place_ack(sample(
        "binance",
        "internal-binance",
        1_200,
        Some("req-binance-place"),
    ));
    store.record_cancel_finality(sample(
        "binance",
        "internal-binance",
        1_300,
        Some("req-binance-final"),
    ));
    store.record_place_ack(sample(
        "hyperliquid:xyz",
        "internal-hl",
        1_400,
        Some("req-hl-place"),
    ));
    store.record_cancel_finality(sample(
        "hyperliquid:xyz",
        "internal-hl",
        1_500,
        Some("req-hl-final"),
    ));

    store.invalidate_credentials_update(" OKX ");
    let after_okx = venue_keys(store.snapshot(1_600));
    assert!(!after_okx.iter().any(|venue| venue == "okx"));
    assert!(after_okx.iter().any(|venue| venue == "binance"));
    assert!(after_okx.iter().any(|venue| venue == "hyperliquid:xyz"));

    store.invalidate_credentials_update("hyperliquid");
    let after_hyperliquid = venue_keys(store.snapshot(1_700));
    assert_eq!(after_hyperliquid, vec!["binance"]);
}

#[test]
fn record_methods_only_accept_live_adapter_ack_and_order_query_finality() {
    let store = LiveOrderProofHealthStore::default();
    store.record_submit_ack_from_record(&record(
        ExecutionMode::DryRun,
        LiveOrderState::Accepted,
        OrderUpdateSource::AdapterAck,
        1_000,
    ));
    store.record_submit_ack_from_record(&record(
        ExecutionMode::Live,
        LiveOrderState::Accepted,
        OrderUpdateSource::AdapterAck,
        1_100,
    ));
    store.record_cancel_ack_from_record(&record(
        ExecutionMode::Live,
        LiveOrderState::CancelRequested,
        OrderUpdateSource::AdapterAck,
        1_200,
    ));
    store.record_order_query_cancel_finality_from_record(&record(
        ExecutionMode::Live,
        LiveOrderState::Cancelled,
        OrderUpdateSource::OrderQuery,
        1_300,
    ));

    let rows = store.snapshot(1_400);

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, VenueOperationStatus::Ok);
    assert_eq!(rows[0].place_ack_count, 1);
    assert_eq!(rows[0].cancel_requested_count, 1);
    assert_eq!(rows[0].cancel_finality_count, 1);
    assert_eq!(
        rows[0]
            .cancel_finality
            .as_ref()
            .map(|sample| sample.source.as_str()),
        Some("order_query")
    );
}
