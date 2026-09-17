fn private_ws_evidence_path(venue: &ExchangeWsVenue, capability: &ExchangeWsOperation) -> String {
    let endpoint = venue
        .private_endpoint
        .as_deref()
        .or(venue.trade_endpoint.as_deref())
        .unwrap_or(&venue.public_endpoint);
    capability
        .operation
        .as_deref()
        .map(|operation| format!("{endpoint}#{operation}"))
        .unwrap_or_else(|| endpoint.to_owned())
}

fn private_ws_support_status_name(capability: &ExchangeWsOperation) -> &'static str {
    match capability.status {
        shared_types::ExchangeWsSupportStatus::Ready => "ready",
        shared_types::ExchangeWsSupportStatus::RequiresPermission => "requires_permission",
        shared_types::ExchangeWsSupportStatus::SchemaPending => "schema_pending",
        shared_types::ExchangeWsSupportStatus::RestOnly => "rest_only",
    }
}

fn private_ws_release_status_name(
    status: shared_types::ExchangeWsReleaseStatus,
) -> &'static str {
    match status {
        shared_types::ExchangeWsReleaseStatus::Unknown => "unknown",
        shared_types::ExchangeWsReleaseStatus::ProductionReady => "production_ready",
        shared_types::ExchangeWsReleaseStatus::BetaUnavailable => "beta_unavailable",
    }
}

fn private_ws_operation_use_cases(operation: &str) -> Vec<String> {
    let use_case = match operation {
        OP_PRIVATE_WS_SESSION => "private_ws_auth",
        OP_PRIVATE_WS_SUBSCRIBE => "private_ws_subscription",
        OP_PRIVATE_WS_ACCOUNT_STREAM => "account_stream",
        OP_PRIVATE_WS_ORDER_STREAM => "order_stream",
        _ => "private_ws_runtime",
    };
    vec!["private_ws_runtime".to_owned(), use_case.to_owned()]
}

fn private_ws_operation_data_kinds(operation: &str) -> Vec<String> {
    let data_kind = match operation {
        OP_PRIVATE_WS_SESSION => "private_ws_session",
        OP_PRIVATE_WS_SUBSCRIBE => "private_ws_subscription",
        OP_PRIVATE_WS_ACCOUNT_STREAM => "account_state_stream",
        OP_PRIVATE_WS_ORDER_STREAM => "order_state_stream",
        _ => "private_ws_runtime",
    };
    vec![data_kind.to_owned()]
}

fn reconciliation_runtime_row(
    snapshot: ReconciliationRuntimeHealth,
    credential: Option<&VenueCredentialStatus>,
) -> VenueOperationHealth {
    VenueOperationHealth {
        venue: snapshot.venue,
        operation: OP_ORDER_RECONCILIATION.to_owned(),
        status: snapshot.status,
        source: SOURCE_RECONCILIATION_RUNTIME.to_owned(),
        message: snapshot.message,
        supported: credential.map(|venue| venue.live_write),
        configured: credential.map(credentials_configured),
        requested: snapshot.requested,
        rows: snapshot.rows,
        freshness_ms: snapshot.freshness_ms,
        retry_after_ms: snapshot.retry_after_ms,
        latency_ms: None,
        latency_p95_ms: None,
        error: snapshot.error,
        evidence: None,
        problem: None,
        observed_at_ms: snapshot.observed_at_ms,
    }
}

fn reconciliation_missing_row(
    venue: &VenueCredentialStatus,
    observed_at_ms: i64,
) -> VenueOperationHealth {
    let supported = venue.live_write;
    let configured = credentials_configured(venue);
    let status = missing_cache_status(supported, configured);
    let message = reconciliation_missing_message(supported, configured).to_owned();
    VenueOperationHealth {
        venue: venue.venue.clone(),
        operation: OP_ORDER_RECONCILIATION.to_owned(),
        status,
        source: SOURCE_RECONCILIATION_RUNTIME.to_owned(),
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

fn run_finality_runtime_rows(
    credentials: &[VenueCredentialStatus],
    snapshots: Vec<RunFinalityRuntimeHealth>,
    observed_at_ms: i64,
) -> Vec<VenueOperationHealth> {
    let mut by_venue = snapshots
        .into_iter()
        .map(|snapshot| (normalized_venue_name(&snapshot.venue), snapshot))
        .collect::<BTreeMap<_, _>>();
    let global = by_venue.remove(&normalized_venue_name(GLOBAL_RUN_FINALITY_VENUE));
    let mut rows = Vec::with_capacity(credentials.len());
    for venue in credentials {
        let key = normalized_venue_name(&venue.venue);
        let snapshot = by_venue.remove(&key).or_else(|| {
            (venue.live_write && credentials_configured(venue))
                .then_some(global.as_ref())
                .flatten()
                .map(|row| venue_scoped_global_run_finality(row, &venue.venue))
        });
        let row = match snapshot {
            Some(snapshot) => run_finality_runtime_row(&snapshot, Some(venue)),
            None => run_finality_missing_row(venue, observed_at_ms),
        };
        rows.push(row);
    }
    rows.extend(
        by_venue
            .into_values()
            .map(|snapshot| run_finality_runtime_row(&snapshot, None)),
    );
    rows
}

fn venue_scoped_global_run_finality(
    row: &RunFinalityRuntimeHealth,
    venue: &str,
) -> RunFinalityRuntimeHealth {
    let mut scoped = row.clone();
    scoped.venue = venue.to_owned();
    if row.status == VenueOperationStatus::Ok
        && row.requested == Some(0)
        && row.rows == Some(0)
        && row.error.is_none()
    {
        scoped.message = format!("订单终态回查空闲：当前无待确认执行订单；{}", row.message);
    } else {
        scoped.status = global_run_finality_status(row.status);
        scoped.message = format!(
            "全局订单终态回查样本：{}；未证明该交易所订单终态回查",
            row.message
        );
        scoped.requested = None;
        scoped.rows = None;
    }
    scoped
}

fn global_run_finality_status(status: VenueOperationStatus) -> VenueOperationStatus {
    match status {
        VenueOperationStatus::Blocked => VenueOperationStatus::Blocked,
        VenueOperationStatus::Warn => VenueOperationStatus::Warn,
        VenueOperationStatus::Unsupported => VenueOperationStatus::Unsupported,
        VenueOperationStatus::Ok | VenueOperationStatus::Unknown => VenueOperationStatus::Unknown,
    }
}

fn run_finality_runtime_row(
    snapshot: &RunFinalityRuntimeHealth,
    credential: Option<&VenueCredentialStatus>,
) -> VenueOperationHealth {
    let message = snapshot.message.clone();
    let status = snapshot.status;
    let problem = run_finality_runtime_problem(snapshot, &message);
    VenueOperationHealth {
        venue: snapshot.venue.clone(),
        operation: OP_ORDER_FINALITY.to_owned(),
        status,
        source: SOURCE_RUN_FINALITY_RUNTIME.to_owned(),
        message,
        supported: credential.map(|venue| venue.live_write),
        configured: credential.map(credentials_configured),
        requested: snapshot.requested,
        rows: snapshot.rows,
        freshness_ms: snapshot.freshness_ms,
        retry_after_ms: snapshot.retry_after_ms,
        latency_ms: None,
        latency_p95_ms: None,
        error: snapshot.error.clone(),
        evidence: Some(run_finality_runtime_evidence(snapshot)),
        problem,
        observed_at_ms: snapshot.observed_at_ms,
    }
}

fn run_finality_missing_row(
    venue: &VenueCredentialStatus,
    observed_at_ms: i64,
) -> VenueOperationHealth {
    let supported = venue.live_write;
    let configured = credentials_configured(venue);
    let status = missing_cache_status(supported, configured);
    let message = run_finality_missing_message(supported, configured).to_owned();
    VenueOperationHealth {
        venue: venue.venue.clone(),
        operation: OP_ORDER_FINALITY.to_owned(),
        status,
        source: SOURCE_RUN_FINALITY_RUNTIME.to_owned(),
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

fn credential_row(
    venue: &VenueCredentialStatus,
    operation: &str,
    supported: bool,
    observed_at_ms: i64,
) -> VenueOperationHealth {
    let configured = credentials_configured(venue);
    let status = credential_status(supported, configured);
    let message = credential_message(venue, supported, configured);
    VenueOperationHealth {
        venue: venue.venue.clone(),
        operation: operation.to_owned(),
        status,
        source: SOURCE_CREDENTIAL_CONFIG.to_owned(),
        message: message.clone(),
        supported: Some(supported),
        configured: Some(configured),
        requested: None,
        rows: None,
        freshness_ms: None,
        retry_after_ms: None,
        latency_ms: None,
        latency_p95_ms: None,
        error: (!supported || !configured).then_some(message),
        evidence: None,
        problem: None,
        observed_at_ms,
    }
}

fn credential_validation_rows(
    credentials: &[VenueCredentialStatus],
    snapshots: Vec<venue_credentials::VenueCredentialValidationSnapshot>,
    now_ms: i64,
) -> Vec<VenueOperationHealth> {
    let credentials_by_venue = credentials
        .iter()
        .map(|venue| (normalized_venue_name(&venue.venue), venue))
        .collect::<BTreeMap<_, _>>();
    let mut rows = Vec::new();
    for snapshot in snapshots {
        let credential = credentials_by_venue
            .get(&normalized_venue_name(&snapshot.venue))
            .copied();
        rows.extend(credential_validation_probe_rows(
            &snapshot.venue,
            credential,
            snapshot.evidence,
            now_ms,
        ));
    }
    rows
}
