use super::*;

pub(crate) async fn positions(state: &AppState) -> Result<Vec<PositionRow>, common::AppError> {
    let positions = state.trading_service().list_positions_low_latency().await?;
    let orders = state.trading_service().list_orders();
    let include_dry_run = state.trading_service().adapter_name() == "mock";
    let rates = funding_rows(state);
    let pair_evidence = execution_pair_evidence(state);
    let cfg = state.trading_service().risk_config();
    let now_ms = common::time::now_ms();
    let dry_run_marks = dry_run_mark_prices(state, &orders, now_ms);
    Ok(rows_from_sources_with_dry_run_marks(
        positions,
        &orders,
        &rates,
        &pair_evidence,
        DryRunRows {
            enabled: include_dry_run,
            marks: &dry_run_marks,
        },
        RiskAnnotation {
            now_ms,
            warn_pct: cfg.liquidation_warn_pct,
            danger_pct: cfg.liquidation_danger_pct,
        },
    ))
}

pub(crate) async fn nav_history(
    state: &AppState,
    requested_limit: Option<usize>,
) -> HistoryResponse<PortfolioNavHistoryRow> {
    let now_ms = common::time::now_ms();
    let limit = nav_history_limit(requested_limit);
    let (total_count, rows) = nav_history_rows(state, limit).await;
    let storage_health = venue_operation_health::portfolio_nav_storage_health_row(state, now_ms);
    let backend_status = nav_history_backend_status(state, now_ms);
    let latest_at_ms = rows.last().map(|row| row.occurred_at_ms);
    let problems = nav_history_problems(&storage_health);
    HistoryResponse {
        count: rows.len(),
        page: Some(nav_history_page(limit, rows.len(), total_count)),
        row_cap: Some(shared_types::RowCapEvidence::exact(
            limit,
            rows.len(),
            total_count,
            NAV_HISTORY_SOURCE,
        )),
        backend_status,
        storage_health: Some(storage_health.clone()),
        source: NAV_HISTORY_SOURCE.to_owned(),
        observed_at_ms: now_ms,
        latest_at_ms,
        freshness_ms: latest_at_ms.map(|ts| now_ms.saturating_sub(ts)),
        problem: problems.first().cloned(),
        retry_after_ms: storage_health.retry_after_ms,
        problems,
        rows,
    }
}

pub(super) fn nav_history_limit(requested_limit: Option<usize>) -> usize {
    requested_limit
        .unwrap_or(NAV_HISTORY_DEFAULT_LIMIT)
        .clamp(1, NAV_HISTORY_MAX_LIMIT)
}

pub(super) async fn nav_history_rows(
    state: &AppState,
    limit: usize,
) -> (usize, Vec<PortfolioNavHistoryRow>) {
    let history = state.portfolio_nav_history().read().await;
    let start = history.len().saturating_sub(limit);
    let rows = history[start..]
        .iter()
        .filter_map(|(occurred_at_ms, nav_usd)| nav_history_row(*occurred_at_ms, *nav_usd))
        .collect::<Vec<_>>();
    (history.len(), rows)
}

pub(super) fn nav_history_row(occurred_at_ms: i64, nav_usd: f64) -> Option<PortfolioNavHistoryRow> {
    nav_usd.is_finite().then_some(PortfolioNavHistoryRow {
        occurred_at_ms,
        nav_usd,
    })
}

pub(super) fn nav_history_page(
    limit: usize,
    returned_count: usize,
    total_count: usize,
) -> HistoryPage {
    HistoryPage {
        limit,
        max_limit: NAV_HISTORY_MAX_LIMIT,
        returned_count,
        has_more: total_count > returned_count,
        next_cursor: None,
    }
}

pub(super) fn nav_history_backend_status(
    state: &AppState,
    observed_at_ms: i64,
) -> HistoryBackendStatus {
    let health = state
        .portfolio_nav_storage_health()
        .snapshot(observed_at_ms);
    let migration_authority = nav_migration_authority(&health);
    HistoryBackendStatus {
        backend: nav_history_backend_name(health.enabled).to_owned(),
        storage_contract: nav_storage_contract(&health, migration_authority.clone()),
        enabled: health.enabled,
        durable: health.enabled,
        fallback: !health.enabled,
        ephemeral: !health.enabled,
        schema_version: health.schema_version,
        migration_checksum: health.migration_checksum.clone(),
        migration_status: Some(migration_authority),
        startup_problem: None,
        timescale_status: Some(shared_types::HistoryTimescaleStatus::NotApplicable),
        timescale_problem: None,
        append_success_total: health.append_success_total,
        append_error_total: health.append_error_total,
        query_success_total: health.load_success_total,
        query_error_total: health.load_error_total,
        last_success_at_ms: health.last_success_at_ms,
        last_append_at_ms: health.latest_sample_at_ms,
        last_query_at_ms: health.last_success_at_ms,
        last_error_at_ms: health.last_error_at_ms,
        last_error: health.last_error.clone(),
        last_error_code: nav_history_error_code(&health),
        observed_at_ms,
    }
}

pub(crate) fn nav_storage_contract(
    health: &nav_persist::NavStorageHealth,
    migration_authority: shared_types::StorageMigrationAuthority,
) -> shared_types::StorageRuntimeContract {
    let mut degraded_reasons = Vec::with_capacity(4);
    if !health.enabled {
        degraded_reasons.extend([
            shared_types::StorageDegradedReason::Disabled,
            shared_types::StorageDegradedReason::Ephemeral,
        ]);
    }
    if health.load_error_total > 0 {
        degraded_reasons.push(shared_types::StorageDegradedReason::ReadFailed);
    }
    if health.append_error_total > 0 {
        degraded_reasons.push(shared_types::StorageDegradedReason::WriteFailed);
    }
    if health.latest_sample_problem.is_some() {
        degraded_reasons.push(shared_types::StorageDegradedReason::Unavailable);
    }
    if health.enabled && !migration_authority.applied {
        degraded_reasons.push(shared_types::StorageDegradedReason::MigrationUnapplied);
    }
    shared_types::StorageRuntimeContract {
        backend_kind: if health.enabled {
            shared_types::StorageBackendKind::Sqlite
        } else {
            shared_types::StorageBackendKind::Memory
        },
        degraded_reasons,
        migration_authority: Some(migration_authority),
    }
}

pub(crate) fn nav_migration_authority(
    health: &nav_persist::NavStorageHealth,
) -> shared_types::StorageMigrationAuthority {
    let checksum = nav_persist::nav_schema_hash();
    shared_types::StorageMigrationAuthority {
        migration_id: nav_persist::NAV_SCHEMA_MIGRATION_ID.to_owned(),
        schema_name: "portfolio_nav".to_owned(),
        migration_path: nav_persist::NAV_SCHEMA_MIGRATION_PATH.to_owned(),
        schema_version: health.schema_version,
        migration_checksum: Some(checksum.clone()),
        applied: health.enabled
            && health.schema_version == Some(nav_persist::NAV_SCHEMA_VERSION)
            && health.migration_checksum.as_deref() == Some(checksum.as_str()),
        applied_at_ms: health.last_success_at_ms,
    }
}

pub(super) fn nav_history_backend_name(enabled: bool) -> &'static str {
    if enabled {
        "sqlite"
    } else {
        "memory"
    }
}

pub(super) fn nav_history_error_code(health: &nav_persist::NavStorageHealth) -> Option<String> {
    if !health.enabled {
        return Some(shared_types::problem::codes::NAV_STORAGE_UNAVAILABLE.to_owned());
    }
    health
        .last_error
        .is_some()
        .then_some(shared_types::problem::codes::NAV_STORAGE_IO_FAILED.to_owned())
}

pub(super) fn nav_history_problems(storage_health: &VenueOperationHealth) -> Vec<ApiProblem> {
    storage_health.problem.clone().into_iter().collect()
}
