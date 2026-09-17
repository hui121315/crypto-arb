fn trading_sql_ledger_latest_at(snapshot: &SqlLedgerStorageSnapshot) -> Option<i64> {
    [
        snapshot.last_append_at_ms,
        snapshot.last_error_at_ms,
        snapshot.last_replay_at_ms,
        snapshot.last_replay_error_at_ms,
    ]
    .into_iter()
    .flatten()
    .max()
}

fn trading_sql_ledger_observed_at(snapshot: &SqlLedgerStorageSnapshot) -> i64 {
    trading_sql_ledger_latest_at(snapshot)
        .or_else(|| trading_sql_migration_latest_at(&snapshot.migration))
        .unwrap_or(snapshot.migration.observed_at_ms)
}

fn nav_storage_recent_unrecovered_error(health: &NavStorageHealth, now_ms: i64) -> bool {
    let Some(error_at_ms) = health.last_error_at_ms else {
        return false;
    };
    let success_after_error = health
        .last_success_at_ms
        .is_some_and(|success_at_ms| success_at_ms >= error_at_ms);
    !success_after_error && freshness_since(error_at_ms, now_ms) <= NAV_STORAGE_ERROR_RECENT_MS
}

fn nav_storage_message(
    health: &NavStorageHealth,
    status: VenueOperationStatus,
    now_ms: i64,
) -> String {
    let path = health.path.as_deref().unwrap_or("未配置");
    let io = format!(
        "load {}/{}，append {}/{}",
        health.load_success_total,
        health
            .load_success_total
            .saturating_add(health.load_error_total),
        health.append_success_total,
        health
            .append_success_total
            .saturating_add(health.append_error_total)
    );
    if !health.enabled {
        return format!("NAV 存储未配置，path={path}，{io}");
    }
    if let Some(error) = &health.last_error {
        if nav_storage_recent_unrecovered_error(health, now_ms) {
            return format!("NAV 存储最近 IO 失败：{error}，path={path}，{io}");
        }
    }
    if let Some(problem) = health.latest_sample_problem.as_deref() {
        return format!("NAV 样本来源未知：{problem}，path={path}，{io}");
    }
    if status == VenueOperationStatus::Warn {
        return format!("NAV 存储存在历史错误样本，path={path}，{io}");
    }
    format!("NAV 存储可用，path={path}，{io}")
}

fn nav_storage_problem(
    health: &NavStorageHealth,
    status: VenueOperationStatus,
    message: &str,
) -> Option<ApiProblem> {
    if status == VenueOperationStatus::Ok {
        return None;
    }
    let sample_unknown = health
        .latest_sample_status
        .as_deref()
        .is_some_and(|status| status == crate::lifecycle::nav_persist::NAV_SAMPLE_STATUS_UNKNOWN);
    let code = if !health.enabled || sample_unknown {
        codes::NAV_STORAGE_UNAVAILABLE
    } else {
        codes::NAV_STORAGE_IO_FAILED
    };
    let authority = crate::services::portfolio::nav_migration_authority(health);
    let storage_contract =
        crate::services::portfolio::nav_storage_contract(health, authority.clone());
    let mut problem = ApiProblem::new(code, message.to_owned()).with_source(SOURCE_NAV_STORAGE);
    problem.details = Some(serde_json::json!({
        "storageContract": storage_contract,
        "migrationAuthority": authority,
        "path": health.path.as_deref(),
        "enabled": health.enabled,
        "loadSuccessTotal": health.load_success_total,
        "loadErrorTotal": health.load_error_total,
        "appendSuccessTotal": health.append_success_total,
        "appendErrorTotal": health.append_error_total,
        "schemaVersion": health.schema_version,
        "migrationChecksum": health.migration_checksum.as_deref(),
        "sampleCount": health.sample_count,
        "latestSampleAtMs": health.latest_sample_at_ms,
        "latestSampleStatus": health.latest_sample_status.as_deref(),
        "latestSampleSource": health.latest_sample_source.as_deref(),
        "latestSampleProblem": health.latest_sample_problem.as_deref(),
        "lastSuccessAtMs": health.last_success_at_ms,
        "lastErrorAtMs": health.last_error_at_ms,
        "lastError": health.last_error.as_deref(),
    }));
    Some(problem)
}

fn nav_storage_evidence(health: &NavStorageHealth) -> VenueOperationEvidence {
    let authority = crate::services::portfolio::nav_migration_authority(health);
    let storage_contract =
        crate::services::portfolio::nav_storage_contract(health, authority.clone());
    let schema_hash = authority
        .migration_checksum
        .clone()
        .unwrap_or_else(nav_persist::nav_schema_hash);
    let mut request_context = vec![
        "backend=sqlite".to_owned(),
        format!("backend_kind={}", storage_contract.backend_kind),
        format!("enabled={}", health.enabled),
        format!(
            "schema_version={}",
            optional_u32_label(health.schema_version)
        ),
        format!("schema_hash={schema_hash}"),
        format!("migration_checksum={schema_hash}"),
        format!("migration_id={}", authority.migration_id),
        format!("migration_path={}", authority.migration_path),
        format!("schema_name={}", authority.schema_name),
        format!("migration_applied={}", authority.applied),
        format!("sample_count={}", health.sample_count),
    ];
    request_context.extend(
        storage_contract
            .degraded_reasons
            .iter()
            .map(|reason| format!("degraded_reason={reason}")),
    );
    if let Some(path) = health.path.as_deref() {
        request_context.push(format!("path={path}"));
    }
    if let Some(status) = health.latest_sample_status.as_deref() {
        request_context.push(format!("latest_sample_status={status}"));
    }
    if let Some(source) = health.latest_sample_source.as_deref() {
        request_context.push(format!("latest_sample_source={source}"));
    }
    if let Some(problem) = health.latest_sample_problem.as_deref() {
        request_context.push(format!("latest_sample_problem={problem}"));
    }
    VenueOperationEvidence {
        method: "sqlite".to_owned(),
        path: "portfolio_nav_samples".to_owned(),
        checked_at: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        doc_version: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        schema_hash,
        fixture_id: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        parser_test: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        request_builder_test: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        auth_kind: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        request_id: None,
        request_context,
        doc_urls: Vec::new(),
        use_cases: vec!["portfolio_nav_history".to_owned()],
        data_kinds: vec!["portfolio_nav_sample".to_owned()],
        rate_scopes: Vec::new(),
        weight: 0,
    }
}

fn optional_u32_label(value: Option<u32>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "unknown".to_owned())
}

fn nav_storage_io_total(health: &NavStorageHealth) -> u64 {
    health.success_total().saturating_add(health.error_total())
}

fn nav_storage_observed_at(health: &NavStorageHealth) -> Option<i64> {
    [
        health.last_success_at_ms,
        health.last_error_at_ms,
        health.latest_sample_at_ms,
    ]
    .into_iter()
    .flatten()
    .max()
}

fn market_problem(row: &MarketRuntimeHealth) -> Option<ApiProblem> {
    row.problem
        .as_ref()
        .map(|problem| problem.to_api_problem(row.quality.problem_code()))
        .map(|problem| problem.with_retry_after_ms(row.retry_after_ms))
        .or_else(|| market_fallback_problem(row))
}

fn market_fallback_problem(row: &MarketRuntimeHealth) -> Option<ApiProblem> {
    if row.quality == MarketQuality::Fresh {
        return None;
    }
    let message = row
        .last_error
        .clone()
        .unwrap_or_else(|| market_message(row.quality).to_owned());
    let source = format!("{SOURCE_MARKET_DATA_CACHE}:{}", row.source.as_str());
    let mut problem = ApiProblem::new(row.quality.problem_code(), message)
        .with_source(source)
        .with_retry_after_ms(row.retry_after_ms);
    problem.details = Some(serde_json::json!({
        "venue": row.venue.as_str(),
        "operation": row.operation,
        "quality": row.quality.as_str(),
        "source": row.source.as_str(),
        "requested": row.requested,
        "rows": row.rows,
    }));
    Some(problem)
}
