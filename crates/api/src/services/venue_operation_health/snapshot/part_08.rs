fn push_optional_context(items: &mut Vec<String>, key: &str, value: Option<&str>) {
    if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
        items.push(format!("{key}={value}"));
    }
}

fn run_finality_runtime_problem(
    snapshot: &RunFinalityRuntimeHealth,
    message: &str,
) -> Option<ApiProblem> {
    if !matches!(
        snapshot.status,
        VenueOperationStatus::Warn | VenueOperationStatus::Blocked
    ) {
        return None;
    }
    let mut problem = ApiProblem::new(codes::HEDGE_ORDER_FINALITY_FAILED, message.to_owned())
        .with_source(SOURCE_RUN_FINALITY_RUNTIME)
        .with_retry_after_ms(snapshot.retry_after_ms);
    if snapshot.refresh_failure_count > 0 || snapshot.publish_failure_count > 0 {
        problem = problem.with_status(502);
    } else if snapshot.remote_missing_count > 0 {
        problem = problem.with_status(409);
    }
    problem.details = Some(run_finality_details(snapshot));
    Some(problem)
}

fn run_finality_details(snapshot: &RunFinalityRuntimeHealth) -> serde_json::Value {
    serde_json::json!({
        "venue": snapshot.venue.as_str(),
        "operation": OP_ORDER_FINALITY,
        "status": snapshot.status,
        "scannedOrderCount": snapshot.scanned_order_count,
        "refreshedOrderCount": snapshot.refreshed_order_count,
        "remoteMissingCount": snapshot.remote_missing_count,
        "skippedTerminalCount": snapshot.skipped_terminal_count,
        "refreshFailureCount": snapshot.refresh_failure_count,
        "publishFailureCount": snapshot.publish_failure_count,
        "freshnessMs": snapshot.freshness_ms,
        "sampleProblem": snapshot.sample_problem.as_ref().map(run_finality_sample_problem_details),
    })
}

fn run_finality_sample_problem_details(
    sample: &crate::services::run_finality::RunFinalitySampleProblem,
) -> serde_json::Value {
    serde_json::json!({
        "rawOrderId": sample.raw_order_id.as_str(),
        "internalOrderId": sample.internal_order_id.as_str(),
        "venue": sample.venue.as_str(),
        "source": sample.source.as_str(),
        "orderState": sample.order_state,
        "code": sample.code.as_str(),
        "message": sample.message.as_str(),
        "status": sample.status,
        "checkedAtMs": sample.checked_at_ms,
        "error": sample.error.as_deref(),
    })
}

fn run_finality_runtime_evidence(snapshot: &RunFinalityRuntimeHealth) -> VenueOperationEvidence {
    VenueOperationEvidence {
        method: "internal".to_owned(),
        path: "run_finality.refresh_pending_runs".to_owned(),
        checked_at: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        doc_version: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        schema_hash: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        fixture_id: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        parser_test: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        request_builder_test: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        auth_kind: "internal_order_query".to_owned(),
        request_id: None,
        request_context: run_finality_context(snapshot),
        doc_urls: Vec::new(),
        use_cases: vec![
            "order_finality".to_owned(),
            "execution_run_finality".to_owned(),
            "close_run_finality".to_owned(),
        ],
        data_kinds: vec![
            "order_state".to_owned(),
            "execution_run_finality".to_owned(),
            "close_run_finality".to_owned(),
        ],
        rate_scopes: Vec::new(),
        weight: 0,
    }
}

fn run_finality_context(snapshot: &RunFinalityRuntimeHealth) -> Vec<String> {
    let mut context = vec![
        format!("operation={OP_ORDER_FINALITY}"),
        format!("scanned_order_count={}", snapshot.scanned_order_count),
        format!("refreshed_order_count={}", snapshot.refreshed_order_count),
        format!("remote_missing_count={}", snapshot.remote_missing_count),
        format!("skipped_terminal_count={}", snapshot.skipped_terminal_count),
        format!("refresh_failure_count={}", snapshot.refresh_failure_count),
        format!("publish_failure_count={}", snapshot.publish_failure_count),
    ];
    if let Some(sample) = snapshot.sample_problem.as_ref() {
        push_run_finality_sample_context(&mut context, sample);
    }
    context
}

fn push_run_finality_sample_context(
    context: &mut Vec<String>,
    sample: &crate::services::run_finality::RunFinalitySampleProblem,
) {
    context.push(format!("sample_raw_order_id={}", sample.raw_order_id));
    context.push(format!(
        "sample_internal_order_id={}",
        sample.internal_order_id
    ));
    context.push(format!("sample_source={}", sample.source));
    context.push(format!("sample_order_state={:?}", sample.order_state));
    context.push(format!("sample_code={}", sample.code));
    context.push(format!("sample_checked_at_ms={}", sample.checked_at_ms));
    if let Some(status) = sample.status {
        context.push(format!("sample_status={status}"));
    }
    if let Some(error) = sample.error.as_deref() {
        context.push(format!("sample_error={error}"));
    }
}

fn live_order_proof_problem(
    snapshot: &LiveOrderProofRuntimeHealth,
    message: &str,
) -> Option<ApiProblem> {
    if !matches!(
        snapshot.status,
        VenueOperationStatus::Warn | VenueOperationStatus::Blocked
    ) {
        return None;
    }
    let mut problem = ApiProblem::new(codes::HEDGE_PRE_TRADE_REJECTED, message.to_owned())
        .with_source(SOURCE_LIVE_ORDER_PROOF_RUNTIME)
        .with_request_id(snapshot.request_id.clone())
        .with_retry_after_ms(snapshot.retry_after_ms);
    if let Some(status) = snapshot
        .last_problem
        .as_ref()
        .and_then(|problem| problem.status)
    {
        problem = problem.with_status(status);
    } else if snapshot.status == VenueOperationStatus::Blocked {
        problem = problem.with_status(502);
    }
    problem.details = Some(live_order_proof_details(snapshot));
    Some(problem)
}

fn live_order_proof_details(snapshot: &LiveOrderProofRuntimeHealth) -> serde_json::Value {
    serde_json::json!({
        "venue": snapshot.venue.as_str(),
        "operation": OP_ORDER_WRITE,
        "status": snapshot.status,
        "livePlaceRemoteProof": live_place_remote_proof(snapshot),
        "liveCancelRemoteProof": live_cancel_remote_proof(snapshot),
        "requested": snapshot.requested,
        "rows": snapshot.rows,
        "freshnessMs": snapshot.freshness_ms,
        "placeAckCount": snapshot.place_ack_count,
        "cancelRequestedCount": snapshot.cancel_requested_count,
        "cancelFinalityCount": snapshot.cancel_finality_count,
        "placeSample": snapshot.place_proof.as_ref().map(live_order_proof_sample_details),
        "cancelRequestSample": snapshot.cancel_request.as_ref().map(live_order_proof_sample_details),
        "cancelFinalitySample": snapshot.cancel_finality.as_ref().map(live_order_proof_sample_details),
        "lastProblem": snapshot.last_problem.as_ref().map(|problem| serde_json::json!({
            "message": problem.message.as_str(),
            "source": problem.source.as_str(),
            "requestId": problem.request_id.as_deref(),
            "retryAfterMs": problem.retry_after_ms,
            "status": problem.status,
            "observedAtMs": problem.observed_at_ms,
        })),
    })
}

fn live_order_proof_sample_details(sample: &LiveOrderProofSample) -> serde_json::Value {
    serde_json::json!({
        "venue": sample.venue.as_str(),
        "symbol": sample.symbol.as_str(),
        "internalOrderId": sample.internal_order_id.as_str(),
        "exchangeOrderId": sample.exchange_order_id.as_deref(),
        "clientOrderId": sample.client_order_id.as_deref(),
        "source": sample.source.as_str(),
        "checkedAtMs": sample.checked_at_ms,
        "requestId": sample.request_id.as_deref(),
        "nativeTransport": sample.native_transport.as_deref(),
        "nativeRequestId": sample.native_request_id.as_deref(),
        "nativeResponseId": sample.native_response_id.as_deref(),
    })
}

fn live_order_proof_evidence(snapshot: &LiveOrderProofRuntimeHealth) -> VenueOperationEvidence {
    VenueOperationEvidence {
        method: "internal".to_owned(),
        path: "live_order_proof.runtime".to_owned(),
        checked_at: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        doc_version: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        schema_hash: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        fixture_id: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        parser_test: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        request_builder_test: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        auth_kind: "live_order_remote_proof".to_owned(),
        request_id: snapshot.request_id.clone(),
        request_context: live_order_proof_context(snapshot),
        doc_urls: Vec::new(),
        use_cases: vec![
            "order_write".to_owned(),
            "live_place_cancel_remote_proof".to_owned(),
        ],
        data_kinds: vec![
            "order_ack".to_owned(),
            "cancel_request_ack".to_owned(),
            "cancel_finality".to_owned(),
        ],
        rate_scopes: Vec::new(),
        weight: 0,
    }
}

fn live_order_proof_context(snapshot: &LiveOrderProofRuntimeHealth) -> Vec<String> {
    let mut context = vec![
        format!(
            "probe_scope={}.order_write.live_place_cancel",
            snapshot.venue
        ),
        format!("probe_source={SOURCE_LIVE_ORDER_PROOF_RUNTIME}"),
        format!(
            "live_place_remote_proof={}",
            live_place_remote_proof(snapshot)
        ),
        format!(
            "live_cancel_remote_proof={}",
            live_cancel_remote_proof(snapshot)
        ),
        format!("place_ack_count={}", snapshot.place_ack_count),
        format!("cancel_requested_count={}", snapshot.cancel_requested_count),
        format!("cancel_finality_count={}", snapshot.cancel_finality_count),
    ];
    push_live_order_sample_context(&mut context, "place", snapshot.place_proof.as_ref());
    let cancel_sample = snapshot
        .cancel_finality
        .as_ref()
        .or(snapshot.cancel_request.as_ref());
    push_live_order_sample_context(&mut context, "cancel", cancel_sample);
    context
}
