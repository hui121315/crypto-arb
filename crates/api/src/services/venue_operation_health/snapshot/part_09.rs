fn push_live_order_sample_context(
    context: &mut Vec<String>,
    prefix: &str,
    sample: Option<&LiveOrderProofSample>,
) {
    let Some(sample) = sample else {
        return;
    };
    context.push(format!(
        "sample_{prefix}_internal_order_id={}",
        sample.internal_order_id
    ));
    context.push(format!("sample_{prefix}_symbol={}", sample.symbol));
    context.push(format!("sample_{prefix}_source={}", sample.source));
    context.push(format!(
        "sample_{prefix}_checked_at_ms={}",
        sample.checked_at_ms
    ));
    push_optional_context(
        context,
        &format!("sample_{prefix}_exchange_order_id"),
        sample.exchange_order_id.as_deref(),
    );
    push_optional_context(
        context,
        &format!("sample_{prefix}_client_order_id"),
        sample.client_order_id.as_deref(),
    );
    push_optional_context(
        context,
        &format!("sample_{prefix}_request_id"),
        sample.request_id.as_deref(),
    );
    push_optional_context(
        context,
        &format!("sample_{prefix}_native_transport"),
        sample.native_transport.as_deref(),
    );
    push_optional_context(
        context,
        &format!("sample_{prefix}_native_request_id"),
        sample.native_request_id.as_deref(),
    );
    push_optional_context(
        context,
        &format!("sample_{prefix}_native_response_id"),
        sample.native_response_id.as_deref(),
    );
}

fn live_place_remote_proof(snapshot: &LiveOrderProofRuntimeHealth) -> &'static str {
    if snapshot.place_proof.is_some() {
        "ok"
    } else if snapshot.status == VenueOperationStatus::Blocked {
        "failed"
    } else {
        "missing"
    }
}

fn live_cancel_remote_proof(snapshot: &LiveOrderProofRuntimeHealth) -> &'static str {
    if snapshot.cancel_finality.is_some() {
        "ok"
    } else if snapshot.status == VenueOperationStatus::Blocked {
        "failed"
    } else {
        "missing"
    }
}

fn audit_log_storage_status(snapshot: &AuditSinkHealthSnapshot) -> VenueOperationStatus {
    if !snapshot.initialized {
        return VenueOperationStatus::Blocked;
    }
    if !snapshot.configured {
        return VenueOperationStatus::Warn;
    }
    if !snapshot.opened || !snapshot.writer_alive || snapshot.last_error.is_some() {
        return VenueOperationStatus::Blocked;
    }
    if snapshot.write_failures > 0 {
        return VenueOperationStatus::Warn;
    }
    VenueOperationStatus::Ok
}

fn audit_log_storage_message(
    snapshot: &AuditSinkHealthSnapshot,
    status: VenueOperationStatus,
) -> String {
    let path = snapshot.path.as_deref().unwrap_or("未配置");
    let io = format!(
        "attempts={}，success={}，failed={}，pending={}，capacity={}，writer_alive={}",
        snapshot.write_attempts,
        snapshot.write_successes,
        snapshot.write_failures,
        snapshot.pending_writes,
        snapshot.queue_capacity,
        snapshot.writer_alive
    );
    if !snapshot.initialized {
        return format!("Audit JSONL sink 未初始化，path={path}，{io}");
    }
    if !snapshot.configured {
        return format!("Audit JSONL 未配置，path={path}，高风险动作只有进程内 ActionRun，{io}");
    }
    if !snapshot.opened {
        return format!(
            "Audit JSONL 无法打开：{}，path={path}，{io}",
            snapshot.last_error.as_deref().unwrap_or("unknown")
        );
    }
    if let Some(error) = snapshot.last_error.as_deref() {
        return format!("Audit JSONL 最近写入异常：{error}，path={path}，{io}");
    }
    if status == VenueOperationStatus::Warn {
        return format!("Audit JSONL 可用但存在历史写入失败，path={path}，{io}");
    }
    format!("Audit JSONL 可用，path={path}，{io}")
}

fn audit_log_storage_problem(
    snapshot: &AuditSinkHealthSnapshot,
    status: VenueOperationStatus,
    message: &str,
) -> Option<ApiProblem> {
    if status == VenueOperationStatus::Ok {
        return None;
    }
    let code = if !snapshot.configured || !snapshot.opened || !snapshot.writer_alive {
        codes::AUDIT_STORAGE_UNAVAILABLE
    } else {
        codes::AUDIT_STORAGE_WRITE_FAILED
    };
    let mut problem =
        ApiProblem::new(code, message.to_owned()).with_source(SOURCE_AUDIT_LOG_STORAGE);
    problem.details = Some(serde_json::json!({
        "initialized": snapshot.initialized,
        "configured": snapshot.configured,
        "opened": snapshot.opened,
        "queueCapacity": snapshot.queue_capacity,
        "pendingWrites": snapshot.pending_writes,
        "writerAlive": snapshot.writer_alive,
        "path": snapshot.path.as_deref(),
        "writeAttempts": snapshot.write_attempts,
        "writeSuccesses": snapshot.write_successes,
        "writeFailures": snapshot.write_failures,
        "lastWriteAtMs": snapshot.last_write_at_ms,
        "lastErrorAtMs": snapshot.last_error_at_ms,
        "lastError": snapshot.last_error.as_deref(),
    }));
    Some(problem)
}

fn audit_log_storage_evidence(snapshot: &AuditSinkHealthSnapshot) -> VenueOperationEvidence {
    let mut request_context = vec![
        format!("initialized={}", snapshot.initialized),
        format!("configured={}", snapshot.configured),
        format!("opened={}", snapshot.opened),
        format!("queue_capacity={}", snapshot.queue_capacity),
        format!("pending_writes={}", snapshot.pending_writes),
        format!("writer_alive={}", snapshot.writer_alive),
        format!("write_attempts={}", snapshot.write_attempts),
        format!("write_successes={}", snapshot.write_successes),
        format!("write_failures={}", snapshot.write_failures),
    ];
    push_optional_i64_context(
        &mut request_context,
        "last_write_at_ms",
        snapshot.last_write_at_ms,
    );
    push_optional_i64_context(
        &mut request_context,
        "last_error_at_ms",
        snapshot.last_error_at_ms,
    );
    push_optional_context(
        &mut request_context,
        "last_error",
        snapshot.last_error.as_deref(),
    );
    push_optional_context(&mut request_context, "path", snapshot.path.as_deref());
    VenueOperationEvidence {
        method: "jsonl".to_owned(),
        path: "audit_log".to_owned(),
        checked_at: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        doc_version: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        schema_hash: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        fixture_id: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        parser_test: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        request_builder_test: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        auth_kind: "local_file_append".to_owned(),
        request_id: None,
        request_context,
        doc_urls: Vec::new(),
        use_cases: vec![
            "high_risk_mutation_audit".to_owned(),
            "action_run_audit".to_owned(),
        ],
        data_kinds: vec!["audit_event".to_owned(), "action_run".to_owned()],
        rate_scopes: Vec::new(),
        weight: 0,
    }
}

fn audit_log_observed_at(snapshot: &AuditSinkHealthSnapshot) -> Option<i64> {
    [
        snapshot.last_write_at_ms,
        snapshot.last_error_at_ms,
        Some(snapshot.observed_at_ms),
    ]
    .into_iter()
    .flatten()
    .max()
}

fn history_storage_status(health: &HistoryStoreHealth, now_ms: i64) -> VenueOperationStatus {
    if !health.enabled {
        return VenueOperationStatus::Blocked;
    }
    if history_storage_recent_unrecovered_error(health, now_ms) {
        return VenueOperationStatus::Blocked;
    }
    if history_migration_unapplied(health) {
        return VenueOperationStatus::Blocked;
    }
    if health.error_total() > 0
        || health.fallback
        || !health.durable
        || history_timescale_degraded(health)
    {
        return VenueOperationStatus::Warn;
    }
    VenueOperationStatus::Ok
}

fn history_storage_recent_unrecovered_error(health: &HistoryStoreHealth, now_ms: i64) -> bool {
    let Some(error_at_ms) = health.last_error_at_ms else {
        return false;
    };
    let success_after_error = health
        .last_success_at_ms
        .is_some_and(|success_at_ms| success_at_ms >= error_at_ms);
    !success_after_error && freshness_since(error_at_ms, now_ms) <= HISTORY_STORAGE_ERROR_RECENT_MS
}
