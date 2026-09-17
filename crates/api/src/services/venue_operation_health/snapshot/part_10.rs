fn history_storage_message(
    health: &HistoryStoreHealth,
    status: VenueOperationStatus,
    now_ms: i64,
) -> String {
    let io = format!(
        "append {}/{}，query {}/{}",
        health.append_success_total,
        health
            .append_success_total
            .saturating_add(health.append_error_total),
        health.query_success_total,
        health
            .query_success_total
            .saturating_add(health.query_error_total)
    );
    if !health.enabled {
        return format!("历史存储已禁用，backend={}，{io}", health.backend);
    }
    if let Some(error) = &health.last_error {
        if history_storage_recent_unrecovered_error(health, now_ms) {
            return format!(
                "历史存储 backend={} 最近 IO 失败：{}，{}",
                health.backend, error, io
            );
        }
    }
    if let Some(problem) = &health.startup_problem {
        return format!(
            "历史存储 backend={} fallback，初始化问题：{}，{}",
            health.backend, problem, io
        );
    }
    if let Some(problem) = history_migration_unapplied_problem(health) {
        return format!(
            "历史存储 backend={} migration 未应用：{}，{}",
            health.backend, problem, io
        );
    }
    if !health.durable {
        return format!(
            "历史存储 backend={} 非持久化，仅适合本地/临时历史，{}",
            health.backend, io
        );
    }
    if let Some(problem) = history_timescale_problem(health) {
        return format!(
            "历史存储 backend={} Timescale 降级：{}，{}",
            health.backend, problem, io
        );
    }
    if status == VenueOperationStatus::Warn {
        return format!("历史存储 backend={} 有历史错误样本，{}", health.backend, io);
    }
    format!("历史存储 backend={} 可用，{}", health.backend, io)
}

fn history_storage_problem(
    health: &HistoryStoreHealth,
    status: VenueOperationStatus,
    message: &str,
) -> Option<ApiProblem> {
    if status == VenueOperationStatus::Ok
        || (health.enabled
            && !health.fallback
            && health.last_error.is_none()
            && !history_migration_unapplied(health)
            && !history_timescale_degraded(health))
    {
        return None;
    }
    let mut problem = ApiProblem::new(history_storage_problem_code(health), message.to_owned())
        .with_source(SOURCE_HISTORY_STORAGE);
    problem.details = Some(serde_json::json!({
        "backend": health.backend,
        "storageContract": &health.storage_contract,
        "enabled": health.enabled,
        "durable": health.durable,
        "fallback": health.fallback,
        "schemaVersion": health.schema_version,
        "migrationChecksum": health.migration_checksum.as_deref(),
        "migrationStatus": health.migration_status.as_ref(),
        "startupProblem": health.startup_problem.as_deref(),
        "timescaleStatus": health.timescale_status,
        "timescaleProblem": health.timescale_problem.as_deref(),
        "appendSuccessTotal": health.append_success_total,
        "appendErrorTotal": health.append_error_total,
        "querySuccessTotal": health.query_success_total,
        "queryErrorTotal": health.query_error_total,
        "lastSuccessAtMs": health.last_success_at_ms,
        "lastErrorAtMs": health.last_error_at_ms,
        "lastError": health.last_error.as_deref(),
        "lastErrorCode": health.last_error_code.as_deref(),
    }));
    Some(problem)
}

fn history_storage_problem_code(health: &HistoryStoreHealth) -> &'static str {
    if history_migration_unapplied(health)
        || health.has_degraded_reason(StorageDegradedReason::SchemaDrift)
    {
        return codes::HISTORY_SCHEMA_DRIFT;
    }
    if history_timescale_degraded(health)
        && health.last_error.is_none()
        && health.startup_problem.is_none()
    {
        return codes::HISTORY_STORE_DEGRADED;
    }
    if let Some(code) = history_known_problem_code(health.last_error_code.as_deref()) {
        return code;
    }
    codes::HISTORY_STORE_UNAVAILABLE
}

fn history_known_problem_code(code: Option<&str>) -> Option<&'static str> {
    match code {
        Some(codes::HISTORY_STORE_UNAVAILABLE) => Some(codes::HISTORY_STORE_UNAVAILABLE),
        Some(codes::HISTORY_STORE_DEGRADED) => Some(codes::HISTORY_STORE_DEGRADED),
        Some(codes::HISTORY_STORE_RATE_LIMITED) => Some(codes::HISTORY_STORE_RATE_LIMITED),
        Some(codes::HISTORY_QUERY_CANCELED) => Some(codes::HISTORY_QUERY_CANCELED),
        Some(codes::HISTORY_SCHEMA_DRIFT) => Some(codes::HISTORY_SCHEMA_DRIFT),
        Some(codes::HISTORY_ENCODE_FAILED) => Some(codes::HISTORY_ENCODE_FAILED),
        Some(codes::HISTORY_DECODE_FAILED) => Some(codes::HISTORY_DECODE_FAILED),
        _ => None,
    }
}

fn history_storage_evidence(health: &HistoryStoreHealth) -> VenueOperationEvidence {
    let mut request_context = vec![
        format!("backend={}", health.backend),
        format!("backend_kind={}", health.storage_contract.backend_kind),
        format!("enabled={}", health.enabled),
        format!("durable={}", health.durable),
        format!(
            "schema_version={}",
            optional_u32_label(health.schema_version)
        ),
        format!(
            "migration_checksum={}",
            health.migration_checksum.as_deref().unwrap_or("unknown")
        ),
    ];
    request_context.extend(
        health
            .storage_contract
            .degraded_reasons
            .iter()
            .map(|reason| format!("degraded_reason={reason}")),
    );
    if let Some(migration) = health.migration_status.as_ref() {
        request_context.push(format!("migration_id={}", migration.migration_id));
        request_context.push(format!("migration_path={}", migration.migration_path));
        request_context.push(format!("migration_applied={}", migration.applied));
        request_context.push(format!("schema_name={}", migration.schema_name));
        push_optional_i64_context(
            &mut request_context,
            "migration_applied_at_ms",
            migration.applied_at_ms,
        );
    }
    VenueOperationEvidence {
        method: "storage".to_owned(),
        path: "history_store".to_owned(),
        checked_at: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        doc_version: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        schema_hash: health
            .migration_checksum
            .clone()
            .unwrap_or_else(|| UNRECORDED_EVIDENCE_MARKER.to_owned()),
        fixture_id: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        parser_test: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        request_builder_test: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        auth_kind: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        request_id: None,
        request_context,
        doc_urls: history_storage_doc_urls(health),
        use_cases: vec!["history_storage_health".to_owned()],
        data_kinds: vec![
            "funding_history".to_owned(),
            "opportunity_history".to_owned(),
        ],
        rate_scopes: Vec::new(),
        weight: 0,
    }
}

fn history_migration_unapplied(health: &HistoryStoreHealth) -> bool {
    health.enabled
        && health.durable
        && !health
            .migration_status
            .as_ref()
            .is_some_and(|migration| migration.applied)
}

fn history_migration_unapplied_problem(health: &HistoryStoreHealth) -> Option<String> {
    if !history_migration_unapplied(health) {
        return None;
    }
    Some(
        health
            .migration_status
            .as_ref()
            .map(|migration| format!("{} applied=false", migration.migration_id))
            .unwrap_or_else(|| "schema_migrations applied row missing".to_owned()),
    )
}

fn history_storage_doc_urls(health: &HistoryStoreHealth) -> Vec<String> {
    if health.durable {
        vec![
            POSTGRES_CREATE_TABLE_DOC_URL.to_owned(),
            POSTGRES_INSERT_DOC_URL.to_owned(),
        ]
    } else {
        Vec::new()
    }
}

fn history_timescale_degraded(health: &HistoryStoreHealth) -> bool {
    matches!(
        health.timescale_status,
        Some(HistoryTimescaleStatus::PlainPostgres) | Some(HistoryTimescaleStatus::Partial)
    )
}

fn history_timescale_problem(health: &HistoryStoreHealth) -> Option<&str> {
    history_timescale_degraded(health).then(|| {
        health
            .timescale_problem
            .as_deref()
            .unwrap_or("timescaledb setup degraded")
    })
}

fn history_storage_io_total(health: &HistoryStoreHealth) -> u64 {
    health.success_total().saturating_add(health.error_total())
}

fn history_storage_observed_at(health: &HistoryStoreHealth) -> Option<i64> {
    match (health.last_success_at_ms, health.last_error_at_ms) {
        (Some(success), Some(error)) => Some(success.max(error)),
        (Some(success), None) => Some(success),
        (None, Some(error)) => Some(error),
        (None, None) => None,
    }
}
