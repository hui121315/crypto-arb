fn account_cache_missing_row(
    venue: &VenueCredentialStatus,
    operation: &str,
    observed_at_ms: i64,
) -> VenueOperationHealth {
    let supported = venue.private_read;
    let configured = credentials_configured(venue);
    let status = missing_cache_status(supported, configured);
    let message = missing_cache_message(supported, configured).to_owned();
    VenueOperationHealth {
        venue: venue.venue.clone(),
        operation: operation.to_owned(),
        status,
        source: SOURCE_ACCOUNT_CACHE.to_owned(),
        message: message.clone(),
        supported: Some(supported),
        configured: Some(configured),
        requested: None,
        rows: None,
        freshness_ms: None,
        retry_after_ms: None,
        latency_ms: None,
        latency_p95_ms: None,
        error: account_cache_error(status, &message),
        evidence: None,
        problem: None,
        observed_at_ms,
    }
}

fn private_ws_runtime_row(
    snapshot: PrivateWsRuntimeHealth,
    credential: Option<&VenueCredentialStatus>,
    ws_venues: &[ExchangeWsVenue],
) -> VenueOperationHealth {
    let problem = private_ws_runtime_problem(&snapshot);
    let mut evidence = private_ws_operation_evidence(
        &snapshot.venue,
        snapshot.operation,
        snapshot.request_id.as_deref(),
        ws_venues,
    );
    if let Some(evidence) = evidence.as_mut() {
        evidence
            .request_context
            .extend(private_ws_channel_context(&snapshot));
    }
    VenueOperationHealth {
        venue: snapshot.venue,
        operation: snapshot.operation.to_owned(),
        status: snapshot.status,
        source: SOURCE_PRIVATE_WS_RUNTIME.to_owned(),
        message: snapshot.message,
        supported: credential
            .map(|venue| private_ws_operation_supported(venue, snapshot.operation)),
        configured: credential.map(credentials_configured),
        requested: snapshot.requested,
        rows: snapshot.rows,
        freshness_ms: snapshot.freshness_ms,
        retry_after_ms: snapshot.retry_after_ms,
        latency_ms: None,
        latency_p95_ms: None,
        error: snapshot.error,
        evidence,
        problem,
        observed_at_ms: snapshot.observed_at_ms,
    }
}

fn private_ws_runtime_problem(snapshot: &PrivateWsRuntimeHealth) -> Option<ApiProblem> {
    let error = snapshot.error.as_ref()?;
    let mut problem = ApiProblem::new(codes::PRIVATE_WS_RUNTIME_FAILED, error.clone())
        .with_source(SOURCE_PRIVATE_WS_RUNTIME)
        .with_request_id(snapshot.request_id.clone())
        .with_retry_after_ms(snapshot.retry_after_ms);
    problem.details = Some(serde_json::json!({
        "venue": snapshot.venue.as_str(),
        "operation": snapshot.operation,
        "status": snapshot.status,
        "message": snapshot.message.as_str(),
        "requestId": snapshot.request_id.as_deref(),
        "requested": snapshot.requested,
        "rows": snapshot.rows,
        "freshnessMs": snapshot.freshness_ms,
        "okCount": snapshot.ok_count,
        "warnCount": snapshot.warn_count,
        "blockedCount": snapshot.blocked_count,
        "lastProblem": snapshot.last_problem.as_deref(),
        "lastProblemAtMs": snapshot.last_problem_at_ms,
        "accountDirty": snapshot.account_dirty.as_ref().map(|dirty| serde_json::json!({
            "venue": dirty.venue.as_str(),
            "scope": dirty.scope.as_str(),
            "reason": dirty.reason.as_str(),
            "refetch": "bounded_rest_on_next_read",
        })),
    }));
    Some(problem)
}

fn private_ws_channel_context(snapshot: &PrivateWsRuntimeHealth) -> Vec<String> {
    let mut context = vec![
        format!("channel_ok_count={}", snapshot.ok_count),
        format!("channel_warn_count={}", snapshot.warn_count),
        format!("channel_blocked_count={}", snapshot.blocked_count),
    ];
    if let Some(last_problem) = snapshot.last_problem.as_deref() {
        context.push(format!("channel_last_problem={last_problem}"));
    }
    if let Some(last_problem_at_ms) = snapshot.last_problem_at_ms {
        context.push(format!("channel_last_problem_at_ms={last_problem_at_ms}"));
    }
    if let Some(dirty) = snapshot.account_dirty.as_ref() {
        context.push(format!("account_dirty_venue={}", dirty.venue));
        context.push(format!("account_dirty_scope={}", dirty.scope.as_str()));
        context.push(format!("account_dirty_reason={}", dirty.reason));
        context.push("account_dirty_refetch=bounded_rest_on_next_read".to_owned());
    }
    context.extend(ws_ingest_context(&snapshot.venue));
    context
}

fn ws_ingest_context(venue: &str) -> Vec<String> {
    let venue_key = normalized_venue_name(venue);
    let family_key = normalized_venue_name(venue_family(venue));
    exchange::ws::ingest_snapshots()
        .into_iter()
        .filter(|row| {
            let row_key = normalized_venue_name(&row.exchange);
            row_key == venue_key || row_key == family_key
        })
        .map(|row| {
            let mut line = format!(
                "ws_ingest instance={} callsite={} url={} text={} text_bytes={} binary={} binary_bytes={} decode_failures={} ping_probe_failures={} send_failures={}",
                row.instance_id,
                row.callsite,
                row.url,
                row.stats.text_messages,
                row.stats.text_bytes,
                row.stats.binary_messages,
                row.stats.binary_bytes,
                row.stats.decode_failures,
                row.stats.ping_probe_failures,
                row.stats.send_failures
            );
            if let Some(last_failure) = row.stats.last_failure {
                line.push_str(&format!(" last_failure={last_failure}"));
            }
            line
        })
        .collect()
}

fn private_ws_missing_row(
    venue: &VenueCredentialStatus,
    operation: &str,
    observed_at_ms: i64,
    ws_venues: &[ExchangeWsVenue],
) -> VenueOperationHealth {
    let supported = private_ws_operation_supported(venue, operation);
    let configured = credentials_configured(venue);
    let status = missing_cache_status(supported, configured);
    let message = private_ws_missing_message(supported, configured).to_owned();
    VenueOperationHealth {
        venue: venue.venue.clone(),
        operation: operation.to_owned(),
        status,
        source: SOURCE_PRIVATE_WS_RUNTIME.to_owned(),
        message: message.clone(),
        supported: Some(supported),
        configured: Some(configured),
        requested: None,
        rows: None,
        freshness_ms: None,
        retry_after_ms: None,
        latency_ms: None,
        latency_p95_ms: None,
        error: account_cache_error(status, &message),
        evidence: private_ws_operation_evidence(&venue.venue, operation, None, ws_venues),
        problem: None,
        observed_at_ms,
    }
}

fn private_ws_operation_supported(venue: &VenueCredentialStatus, operation: &str) -> bool {
    let kind = VenueOperationKind::parse(operation);
    if kind.private_ws_requires_order_write() {
        venue.live_write
    } else if kind.private_ws_requires_private_read() {
        venue.private_read
    } else {
        false
    }
}

fn private_ws_operation_evidence(
    venue: &str,
    operation: &str,
    request_id: Option<&str>,
    ws_venues: &[ExchangeWsVenue],
) -> Option<VenueOperationEvidence> {
    let venue_key = normalized_venue_name(venue);
    let family_key = normalized_venue_name(venue_family(venue));
    let ws_venue = ws_venues.iter().find(|candidate| {
        let candidate_key = normalized_venue_name(&candidate.venue);
        candidate_key == venue_key || candidate_key == family_key
    })?;
    let capability = private_ws_capability_operation(ws_venue, operation)?;
    let evidence = capability.evidence.as_ref()?;
    let mut request_context = vec![
        format!("runtime_operation={operation}"),
        format!(
            "support_status={}",
            private_ws_support_status_name(capability)
        ),
        format!(
            "release_status={}",
            private_ws_release_status_name(evidence.release_status)
        ),
        format!("capability_supported={}", capability.supported),
        format!("product={}", capability.product),
    ];
    push_optional_context(
        &mut request_context,
        "ws_operation",
        capability.operation.as_deref(),
    );
    push_optional_context(
        &mut request_context,
        "private_endpoint",
        ws_venue.private_endpoint.as_deref(),
    );
    push_optional_context(
        &mut request_context,
        "trade_endpoint",
        ws_venue.trade_endpoint.as_deref(),
    );
    let schema_hash = evidence
        .fixture_hash
        .clone()
        .unwrap_or_else(|| UNRECORDED_EVIDENCE_MARKER.to_owned());
    let fixture_id = evidence
        .fixture_id
        .clone()
        .unwrap_or_else(|| UNRECORDED_EVIDENCE_MARKER.to_owned());
    push_context_once(&mut request_context, &format!("schema_hash={schema_hash}"));
    push_context_once(&mut request_context, &format!("fixture_id={fixture_id}"));
    Some(VenueOperationEvidence {
        method: "WS".to_owned(),
        path: private_ws_evidence_path(ws_venue, capability),
        checked_at: evidence.checked_at.clone(),
        doc_version: evidence.doc_version.clone(),
        schema_hash,
        fixture_id,
        parser_test: evidence
            .parser_test
            .clone()
            .unwrap_or_else(|| UNRECORDED_EVIDENCE_MARKER.to_owned()),
        request_builder_test: evidence
            .subscription_test
            .clone()
            .unwrap_or_else(|| UNRECORDED_EVIDENCE_MARKER.to_owned()),
        auth_kind: evidence.auth_kind.clone(),
        request_id: request_id.map(str::to_owned),
        request_context,
        doc_urls: vec![evidence.doc_url.clone()],
        use_cases: private_ws_operation_use_cases(operation),
        data_kinds: private_ws_operation_data_kinds(operation),
        rate_scopes: Vec::new(),
        weight: 0,
    })
}

fn private_ws_capability_operation<'a>(
    venue: &'a ExchangeWsVenue,
    operation: &str,
) -> Option<&'a ExchangeWsOperation> {
    match operation {
        OP_PRIVATE_WS_ACCOUNT_STREAM => Some(&venue.account_stream),
        OP_PRIVATE_WS_ORDER_STREAM => Some(&venue.order_stream),
        OP_PRIVATE_WS_SESSION => Some(&venue.account_stream),
        OP_PRIVATE_WS_SUBSCRIBE => Some(&venue.order_stream),
        _ => None,
    }
}
