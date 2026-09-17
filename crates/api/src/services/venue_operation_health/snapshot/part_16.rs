fn endpoint_snapshot_operation_evidence(
    evidence: &EndpointEvidenceSnapshot,
    request_id: Option<String>,
    mut request_context: Vec<String>,
) -> VenueOperationEvidence {
    append_unrecorded_schema_fixture_context(&mut request_context, evidence);
    VenueOperationEvidence {
        method: evidence.method.clone(),
        path: evidence.path.clone(),
        checked_at: evidence.checked_at.clone(),
        doc_version: evidence.doc_version.clone(),
        schema_hash: evidence.schema_hash.clone(),
        fixture_id: evidence.fixture_id.clone(),
        parser_test: evidence.parser_test.clone(),
        request_builder_test: evidence.request_builder_test.clone(),
        auth_kind: evidence.auth_kind.clone(),
        request_id,
        request_context,
        doc_urls: evidence.doc_urls.clone(),
        use_cases: evidence.use_cases.clone(),
        data_kinds: evidence.data_kinds.clone(),
        rate_scopes: evidence.rate_scopes.clone(),
        weight: evidence.weight,
    }
}

fn http_fallback_operation_evidence(
    snapshot: &HttpOutcomeMetricSnapshot,
) -> VenueOperationEvidence {
    let mut request_context = snapshot.last_request_context.clone();
    push_context_once(&mut request_context, "endpoint_evidence=not_recorded");
    VenueOperationEvidence {
        method: snapshot.method.clone(),
        path: snapshot.path.clone(),
        checked_at: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        doc_version: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        schema_hash: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        fixture_id: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        parser_test: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        request_builder_test: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        auth_kind: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        request_id: snapshot.last_request_id.clone(),
        request_context,
        doc_urls: Vec::new(),
        use_cases: vec!["http_outcome_metrics".to_owned()],
        data_kinds: vec!["http_outcome".to_owned()],
        rate_scopes: Vec::new(),
        weight: 0,
    }
}

fn append_unrecorded_schema_fixture_context(
    request_context: &mut Vec<String>,
    evidence: &EndpointEvidenceSnapshot,
) {
    push_unrecorded_context(request_context, "checked_at", &evidence.checked_at);
    push_unrecorded_context(request_context, "doc_version", &evidence.doc_version);
    push_unrecorded_context(request_context, "schema_hash", &evidence.schema_hash);
    push_unrecorded_context(request_context, "fixture_id", &evidence.fixture_id);
    push_unrecorded_context(request_context, "parser_test", &evidence.parser_test);
    push_unrecorded_context(
        request_context,
        "request_builder_test",
        &evidence.request_builder_test,
    );
    push_unrecorded_context(request_context, "auth_kind", &evidence.auth_kind);
}

fn push_unrecorded_context(request_context: &mut Vec<String>, key: &str, value: &str) {
    if value == UNRECORDED_EVIDENCE_MARKER {
        push_context_once(request_context, &format!("{key}=not_recorded"));
    }
}

fn push_context_once(request_context: &mut Vec<String>, value: &str) {
    if !request_context.iter().any(|item| item == value) {
        request_context.push(value.to_owned());
    }
}

fn http_outcome_problem(
    snapshot: &HttpOutcomeMetricSnapshot,
    status: VenueOperationStatus,
    message: &str,
    retry_after_ms: Option<u64>,
) -> Option<ApiProblem> {
    if status == VenueOperationStatus::Ok {
        return None;
    }
    let mut problem = ApiProblem::new(http_problem_code(&snapshot.outcome), message.to_owned())
        .with_retry_after_ms(retry_after_ms)
        .with_request_id(snapshot.last_request_id.clone())
        .with_source(SOURCE_HTTP_OUTCOME_METRICS);
    if let Some(status_code) = snapshot.status_code {
        problem = problem.with_status(status_code);
    }
    problem.details = Some(http_problem_details(snapshot));
    Some(problem)
}

fn http_problem_code(outcome: &str) -> &'static str {
    match outcome {
        "timeout" | "network" => codes::UPSTREAM_NETWORK,
        "circuit_open" | "gate_circuit_open" => codes::CIRCUIT_BREAKER_OPEN,
        _ => codes::UPSTREAM_HTTP,
    }
}

fn http_problem_details(snapshot: &HttpOutcomeMetricSnapshot) -> serde_json::Value {
    serde_json::json!({
        "venue": snapshot.exchange.as_str(),
        "operation": format!("http_rest:{} {}", snapshot.method, snapshot.path),
        "method": snapshot.method.as_str(),
        "path": snapshot.path.as_str(),
        "symbol": request_context_value(&snapshot.last_request_context, "symbol"),
        "outcome": snapshot.outcome.as_str(),
        "status": snapshot.status_code,
        "requestTotal": snapshot.request_total,
        "retryTotal": snapshot.retry_total,
        "requestId": snapshot.last_request_id.as_deref(),
        "requestContext": &snapshot.last_request_context,
        "lastLatencyMs": snapshot.last_latency_ms,
        "latencyP95Ms": snapshot.latency_p95_ms,
        "endpointEvidence": snapshot.endpoint_evidence.as_ref().map(endpoint_details),
    })
}

fn request_context_value<'a>(request_context: &'a [String], key: &str) -> Option<&'a str> {
    request_context.iter().find_map(|item| {
        item.split_once('=')
            .filter(|(candidate, _)| *candidate == key)
            .map(|(_, value)| value)
    })
}

fn endpoint_details(evidence: &EndpointEvidenceSnapshot) -> serde_json::Value {
    serde_json::json!({
        "method": evidence.method.as_str(),
        "path": evidence.path.as_str(),
        "checkedAt": evidence.checked_at.as_str(),
        "docVersion": evidence.doc_version.as_str(),
        "schemaHash": evidence.schema_hash.as_str(),
        "fixtureId": evidence.fixture_id.as_str(),
        "parserTest": evidence.parser_test.as_str(),
        "requestBuilderTest": evidence.request_builder_test.as_str(),
        "authKind": evidence.auth_kind.as_str(),
        "docUrls": &evidence.doc_urls,
        "useCases": &evidence.use_cases,
        "dataKinds": &evidence.data_kinds,
        "rateScopes": &evidence.rate_scopes,
        "weight": evidence.weight,
    })
}

fn http_retry_after_remaining(snapshot: &HttpOutcomeMetricSnapshot, now_ms: i64) -> Option<u64> {
    let retry_after_ms = snapshot.last_retry_after_ms?;
    let elapsed_ms = freshness_since(snapshot.last_observed_at_ms, now_ms) as u64;
    retry_after_ms
        .checked_sub(elapsed_ms)
        .filter(|value| *value > 0)
}

fn freshness_since(observed_at_ms: i64, now_ms: i64) -> i64 {
    now_ms.saturating_sub(observed_at_ms).max(0)
}

fn http_outcome_status(
    snapshot: &HttpOutcomeMetricSnapshot,
    freshness_ms: i64,
    retry_after_ms: Option<u64>,
) -> VenueOperationStatus {
    match snapshot.outcome.as_str() {
        "success" => {
            if http_outcome_is_slow(snapshot) {
                VenueOperationStatus::Warn
            } else {
                VenueOperationStatus::Ok
            }
        }
        "rate_limited" | "gate_rate_limited" | "circuit_open" | "gate_circuit_open" => {
            if retry_after_ms.is_some() {
                VenueOperationStatus::Blocked
            } else {
                VenueOperationStatus::Warn
            }
        }
        "timeout" | "network" | "http_error" | "other_error" => {
            if freshness_ms <= HTTP_ERROR_RECENT_MS {
                VenueOperationStatus::Blocked
            } else {
                VenueOperationStatus::Warn
            }
        }
        _ => VenueOperationStatus::Unknown,
    }
}

fn http_outcome_is_slow(snapshot: &HttpOutcomeMetricSnapshot) -> bool {
    snapshot
        .latency_p95_ms
        .is_some_and(|p95_ms| p95_ms > HTTP_SLOW_P95_WARN_MS)
}

fn http_outcome_message(
    snapshot: &HttpOutcomeMetricSnapshot,
    retry_after_ms: Option<u64>,
) -> String {
    let status = snapshot
        .status_code
        .map_or_else(|| "none".to_owned(), |status| status.to_string());
    let p95 = snapshot
        .latency_p95_ms
        .map(|p95_ms| {
            if http_outcome_is_slow(snapshot) {
                format!("，慢 endpoint：p95 bucket <= {p95_ms}ms 超过 {HTTP_SLOW_P95_WARN_MS}ms")
            } else {
                format!("，p95 bucket <= {p95_ms}ms")
            }
        })
        .unwrap_or_default();
    let retries = if snapshot.retry_total > 0 {
        format!("，retry {}", snapshot.retry_total)
    } else {
        String::new()
    };
    match retry_after_ms {
        Some(wait_ms) => format!(
            "HTTP {} {} 最近 {}，status={}，延迟 {}ms{}{}，退避 {}ms",
            snapshot.method,
            snapshot.path,
            snapshot.outcome,
            status,
            snapshot.last_latency_ms,
            p95,
            retries,
            wait_ms
        ),
        None => format!(
            "HTTP {} {} 最近 {}，status={}，延迟 {}ms{}{}",
            snapshot.method,
            snapshot.path,
            snapshot.outcome,
            status,
            snapshot.last_latency_ms,
            p95,
            retries
        ),
    }
}

fn http_outcome_rank(outcome: &str) -> u8 {
    match outcome {
        "success" => 0,
        "timeout" | "network" | "other_error" => 1,
        "http_error" => 2,
        "rate_limited" | "gate_rate_limited" => 3,
        "circuit_open" | "gate_circuit_open" => 4,
        _ => 5,
    }
}

fn max_retry_after(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    }
}
