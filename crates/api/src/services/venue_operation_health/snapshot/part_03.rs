fn order_stream_probe_row(
    credential: &VenueCredentialStatus,
    probe: &UnresolvedOrderStreamProbe,
    observed_at_ms: i64,
    ws_venues: &[ExchangeWsVenue],
) -> VenueOperationHealth {
    let freshness_ms = observed_at_ms.saturating_sub(probe.oldest_created_at_ms);
    let message = format!(
        "存在 {} 笔未决实盘订单，但尚无新鲜私有 WS 订单流样本",
        probe.count
    );
    let mut problem = ApiProblem::new(codes::PRIVATE_WS_RUNTIME_FAILED, message.clone())
        .with_source(SOURCE_PRIVATE_WS_RUNTIME)
        .with_retry_after_ms(Some(ORDER_STREAM_PROBE_RETRY_AFTER_MS));
    problem.details = Some(serde_json::json!({
        "venue": credential.venue.as_str(),
        "operation": OP_PRIVATE_WS_ORDER_STREAM,
        "status": VenueOperationStatus::Warn,
        "message": message.as_str(),
        "requested": probe.count,
        "rows": 0,
        "freshnessMs": freshness_ms,
        "freshnessWindowMs": ORDER_STREAM_PROBE_RETRY_AFTER_MS,
        "oldestUnresolvedOrderCreatedAtMs": probe.oldest_created_at_ms,
    }));
    VenueOperationHealth {
        venue: credential.venue.clone(),
        operation: OP_PRIVATE_WS_ORDER_STREAM.to_owned(),
        status: VenueOperationStatus::Warn,
        source: SOURCE_PRIVATE_WS_RUNTIME.to_owned(),
        message: message.clone(),
        supported: Some(credential.live_write),
        configured: Some(true),
        requested: Some(probe.count),
        rows: Some(0),
        freshness_ms: Some(freshness_ms),
        retry_after_ms: Some(ORDER_STREAM_PROBE_RETRY_AFTER_MS),
        latency_ms: None,
        latency_p95_ms: None,
        error: Some(message),
        evidence: private_ws_operation_evidence(
            &credential.venue,
            OP_PRIVATE_WS_ORDER_STREAM,
            None,
            ws_venues,
        ),
        problem: Some(problem),
        observed_at_ms,
    }
}

fn reconciliation_runtime_rows(
    credentials: &[VenueCredentialStatus],
    snapshots: Vec<ReconciliationRuntimeHealth>,
    observed_at_ms: i64,
) -> Vec<VenueOperationHealth> {
    let mut by_venue = snapshots
        .into_iter()
        .map(|snapshot| (normalized_venue_name(&snapshot.venue), snapshot))
        .collect::<BTreeMap<_, _>>();
    let global = by_venue.remove(&normalized_venue_name(GLOBAL_RECONCILIATION_VENUE));
    let mut rows = Vec::with_capacity(credentials.len());
    for venue in credentials {
        let key = normalized_venue_name(&venue.venue);
        let snapshot = by_venue.remove(&key).or_else(|| {
            (venue.live_write && credentials_configured(venue))
                .then(|| global.clone())
                .flatten()
                .map(|row| venue_scoped_global_reconciliation(row, &venue.venue))
        });
        let row = match snapshot {
            Some(snapshot) => reconciliation_runtime_row(snapshot, Some(venue)),
            None => reconciliation_missing_row(venue, observed_at_ms),
        };
        rows.push(row);
    }
    rows.extend(
        by_venue
            .into_values()
            .map(|snapshot| reconciliation_runtime_row(snapshot, None)),
    );
    rows
}

fn venue_scoped_global_reconciliation(
    mut row: ReconciliationRuntimeHealth,
    venue: &str,
) -> ReconciliationRuntimeHealth {
    let idle = row.status == VenueOperationStatus::Ok
        && row.requested == Some(0)
        && row.rows == Some(0)
        && row.error.is_none();
    let global_message = row.message;
    row.venue = venue.to_owned();
    if idle {
        row.message = format!("订单回查空闲：当前无未决实盘订单；{global_message}");
    } else {
        row.status = global_reconciliation_status(row.status);
        row.message = format!("全局订单回查样本：{global_message}；未证明该交易所订单回查");
        row.requested = None;
        row.rows = None;
    }
    row
}

fn global_reconciliation_status(status: VenueOperationStatus) -> VenueOperationStatus {
    match status {
        VenueOperationStatus::Blocked => VenueOperationStatus::Blocked,
        VenueOperationStatus::Warn => VenueOperationStatus::Warn,
        VenueOperationStatus::Unsupported => VenueOperationStatus::Unsupported,
        VenueOperationStatus::Ok | VenueOperationStatus::Unknown => VenueOperationStatus::Unknown,
    }
}

fn http_outcome_rows(
    snapshots: Vec<HttpOutcomeMetricSnapshot>,
    now_ms: i64,
) -> Vec<VenueOperationHealth> {
    latest_http_outcomes(snapshots)
        .into_iter()
        .map(|snapshot| http_outcome_row(snapshot, now_ms))
        .collect()
}

fn latest_http_outcomes(
    snapshots: Vec<HttpOutcomeMetricSnapshot>,
) -> Vec<HttpOutcomeMetricSnapshot> {
    let mut latest = BTreeMap::new();
    for snapshot in snapshots {
        let key = (
            normalized_venue_name(&snapshot.exchange),
            snapshot.method.clone(),
            snapshot.path.clone(),
        );
        let replace = match latest.get(&key) {
            Some(current) => newer_http_outcome(&snapshot, current),
            None => true,
        };
        if replace {
            latest.insert(key, snapshot);
        }
    }
    latest.into_values().collect()
}

fn newer_http_outcome(
    next: &HttpOutcomeMetricSnapshot,
    current: &HttpOutcomeMetricSnapshot,
) -> bool {
    next.last_observed_at_ms > current.last_observed_at_ms
        || (next.last_observed_at_ms == current.last_observed_at_ms
            && http_outcome_rank(&next.outcome) > http_outcome_rank(&current.outcome))
}

fn http_outcome_row(snapshot: HttpOutcomeMetricSnapshot, now_ms: i64) -> VenueOperationHealth {
    let freshness_ms = freshness_since(snapshot.last_observed_at_ms, now_ms);
    let retry_after_ms = http_retry_after_remaining(&snapshot, now_ms);
    let status = http_outcome_status(&snapshot, freshness_ms, retry_after_ms);
    let message = http_outcome_message(&snapshot, retry_after_ms);
    let evidence = Some(http_operation_evidence(&snapshot));
    let problem = http_outcome_problem(&snapshot, status, &message, retry_after_ms);
    VenueOperationHealth {
        venue: snapshot.exchange,
        operation: format!("http_rest:{} {}", snapshot.method, snapshot.path),
        status,
        source: SOURCE_HTTP_OUTCOME_METRICS.to_owned(),
        message: message.clone(),
        supported: Some(true),
        configured: None,
        requested: None,
        rows: Some(snapshot.request_total),
        freshness_ms: Some(freshness_ms),
        retry_after_ms,
        latency_ms: Some(snapshot.last_latency_ms),
        latency_p95_ms: snapshot.latency_p95_ms,
        error: attention_error(status, &message),
        evidence,
        problem,
        observed_at_ms: snapshot.last_observed_at_ms,
    }
}

fn host_gate_rows(snapshots: Vec<HostGateSnapshot>, _now_ms: i64) -> Vec<VenueOperationHealth> {
    snapshots.into_iter().map(host_gate_row).collect()
}

fn host_gate_row(snapshot: HostGateSnapshot) -> VenueOperationHealth {
    let retry_after_ms = max_retry_after(
        snapshot.rate_limit_retry_after_ms,
        snapshot.circuit_retry_after_ms,
    );
    let status = host_gate_status(&snapshot, retry_after_ms);
    let message = host_gate_message(&snapshot, retry_after_ms);
    let problem = host_gate_problem(&snapshot, status, &message, retry_after_ms);
    VenueOperationHealth {
        venue: snapshot.exchange,
        operation: format!("{OP_HOST_GATE_PREFIX}{}", snapshot.host),
        status,
        source: SOURCE_HOST_GATE.to_owned(),
        message: message.clone(),
        supported: Some(true),
        configured: None,
        requested: None,
        rows: Some(snapshot.inflight_keys),
        freshness_ms: Some(0),
        retry_after_ms,
        latency_ms: None,
        latency_p95_ms: None,
        error: attention_error(status, &message),
        evidence: None,
        problem,
        observed_at_ms: snapshot.observed_at_ms,
    }
}

fn rate_limiter_rows(snapshots: &[RateLimiterSnapshot], now_ms: i64) -> Vec<VenueOperationHealth> {
    snapshots
        .iter()
        .map(|snapshot| rate_limiter_row(snapshot, now_ms))
        .collect()
}

fn rate_limiter_row(snapshot: &RateLimiterSnapshot, now_ms: i64) -> VenueOperationHealth {
    let status = rate_limiter_status(snapshot, now_ms);
    let message = rate_limiter_message(snapshot, now_ms);
    let problem = rate_limiter_problem(snapshot, status, &message);
    VenueOperationHealth {
        venue: snapshot.name.clone(),
        operation: format!("{OP_RATE_LIMITER_PREFIX}{}", snapshot.name),
        status,
        source: SOURCE_RATE_LIMITER.to_owned(),
        message: message.clone(),
        supported: Some(true),
        configured: None,
        requested: Some(u64::from(snapshot.qps)),
        rows: Some(snapshot.wait_total),
        freshness_ms: latest_rate_limiter_observed_at(snapshot)
            .map(|observed_at_ms| freshness_since(observed_at_ms, now_ms)),
        retry_after_ms: None,
        latency_ms: None,
        latency_p95_ms: None,
        error: attention_error(status, &message),
        evidence: None,
        problem,
        observed_at_ms: snapshot.observed_at_ms,
    }
}

fn task_registry_rows(
    registry: &crate::task_registry::TaskRegistry,
    now_ms: i64,
) -> Vec<VenueOperationHealth> {
    let snapshots = registry.task_snapshots(now_ms);
    let issues = snapshots
        .iter()
        .filter_map(|snapshot| snapshot.issue.clone())
        .collect::<Vec<_>>();
    let total = snapshots.len();
    let enabled = snapshots.iter().filter(|snapshot| snapshot.enabled).count();
    let slow_count = snapshots
        .iter()
        .filter(|snapshot| {
            snapshot.enabled && snapshot.issue.is_none() && task_snapshot_slow(snapshot)
        })
        .count();
    let mut rows = Vec::with_capacity(snapshots.len().saturating_add(1));
    rows.push(task_registry_summary_row(
        total, enabled, &issues, slow_count, now_ms,
    ));
    rows.extend(
        snapshots
            .iter()
            .map(|snapshot| task_registry_task_row(snapshot, now_ms)),
    );
    rows
}
