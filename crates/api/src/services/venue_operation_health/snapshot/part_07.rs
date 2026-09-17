fn credential_validation_probe_rows(
    venue: &str,
    credential: Option<&VenueCredentialStatus>,
    evidence: VenueCredentialValidationEvidence,
    now_ms: i64,
) -> Vec<VenueOperationHealth> {
    let validation_status = evidence.status;
    evidence
        .probes
        .into_iter()
        .map(|probe| {
            credential_validation_row(venue, credential, validation_status, &probe, now_ms)
        })
        .collect()
}

fn credential_validation_row(
    venue: &str,
    credential: Option<&VenueCredentialStatus>,
    validation_status: VenueCredentialValidationStatus,
    probe: &VenueCredentialProbe,
    now_ms: i64,
) -> VenueOperationHealth {
    let status = credential_probe_operation_status(probe.status);
    let message = credential_probe_message(validation_status, probe);
    VenueOperationHealth {
        venue: venue.to_owned(),
        operation: credential_probe_operation(&probe.kind),
        status,
        source: SOURCE_CREDENTIAL_VALIDATION.to_owned(),
        message: message.clone(),
        supported: credential.map(|venue| credential_probe_supported(venue, &probe.kind)),
        configured: credential.map(credentials_configured),
        requested: Some(1),
        rows: Some(u64::from(probe.status == VenueCredentialProbeStatus::Ok)),
        freshness_ms: Some(freshness_since(probe.checked_at_ms, now_ms)),
        retry_after_ms: None,
        latency_ms: None,
        latency_p95_ms: None,
        error: attention_error(status, &message),
        evidence: Some(credential_probe_evidence(venue, validation_status, probe)),
        problem: None,
        observed_at_ms: probe.checked_at_ms,
    }
}

fn market_row(row: MarketRuntimeHealth, now_ms: i64) -> VenueOperationHealth {
    let problem = market_problem(&row);
    VenueOperationHealth {
        venue: row.venue,
        operation: row.operation.to_owned(),
        status: market_status(row.quality),
        source: format!("{SOURCE_MARKET_DATA_CACHE}:{}", row.source.as_str()),
        message: market_message(row.quality).to_owned(),
        supported: Some(row.quality != MarketQuality::Unsupported),
        configured: None,
        requested: Some(row.requested),
        rows: Some(row.rows),
        freshness_ms: Some(now_ms.saturating_sub(row.observed_at_ms)),
        retry_after_ms: row.retry_after_ms,
        latency_ms: None,
        latency_p95_ms: None,
        error: (row.quality != MarketQuality::Warming)
            .then_some(row.last_error)
            .flatten(),
        evidence: None,
        problem,
        observed_at_ms: row.observed_at_ms,
    }
}

fn task_registry_summary_status(
    total: usize,
    issues: &[TaskIssue],
    slow_count: usize,
) -> VenueOperationStatus {
    if total == 0 {
        return VenueOperationStatus::Unknown;
    }
    if issues
        .iter()
        .any(|issue| task_issue_status(issue.kind) == VenueOperationStatus::Blocked)
    {
        VenueOperationStatus::Blocked
    } else if slow_count > 0 {
        VenueOperationStatus::Warn
    } else if issues.is_empty() {
        VenueOperationStatus::Ok
    } else {
        VenueOperationStatus::Warn
    }
}

fn task_issue_status(kind: TaskIssueKind) -> VenueOperationStatus {
    match kind {
        TaskIssueKind::Dead | TaskIssueKind::Stale | TaskIssueKind::Failing => {
            VenueOperationStatus::Blocked
        }
    }
}

fn task_snapshot_status(snapshot: &TaskSnapshot) -> VenueOperationStatus {
    if !snapshot.enabled {
        return VenueOperationStatus::Unknown;
    }
    snapshot
        .issue
        .as_ref()
        .map(|issue| task_issue_status(issue.kind))
        .unwrap_or_else(|| {
            if task_snapshot_slow(snapshot) {
                VenueOperationStatus::Warn
            } else {
                VenueOperationStatus::Ok
            }
        })
}

fn task_registry_summary_message(
    total: usize,
    enabled: usize,
    healthy: usize,
    disabled: usize,
    issue_count: usize,
    slow_count: usize,
) -> String {
    if total == 0 {
        return "后台任务尚未登记".to_owned();
    }
    let mut message = format!("后台任务 healthy {healthy}/{enabled}，disabled {disabled}");
    if issue_count > 0 {
        message.push_str(&format!("，异常 {issue_count}"));
    }
    if slow_count > 0 {
        message.push_str(&format!("，慢迭代 {slow_count}"));
    }
    message
}

fn task_snapshot_message(snapshot: &TaskSnapshot, now_ms: i64) -> String {
    if !snapshot.enabled {
        return format!("后台任务 {} 未启用", snapshot.name);
    }
    if let Some(issue) = &snapshot.issue {
        return format!("后台任务 {}：{}", issue.name, issue.detail);
    }
    let last_tick_ms = freshness_since(snapshot.last_tick_ms, now_ms);
    let success = snapshot
        .last_success_ms
        .map(|last_success_ms| {
            format!(
                "，last_success {}ms 前",
                freshness_since(last_success_ms, now_ms)
            )
        })
        .unwrap_or_default();
    let duration = snapshot
        .last_duration_ms
        .map(|duration_ms| format!("，last_duration {duration_ms}ms"))
        .unwrap_or_default();
    let restarts = if snapshot.restart_count > 0 {
        format!("，restart {}", snapshot.restart_count)
    } else {
        String::new()
    };
    let slow = task_snapshot_slow_message(snapshot);
    format!(
        "后台任务 {} 正常，last_tick {}ms 前{}{}{}{}，连续失败 {}",
        snapshot.name,
        last_tick_ms,
        success,
        duration,
        slow,
        restarts,
        snapshot.consecutive_failures
    )
}

fn task_snapshot_slow_message(snapshot: &TaskSnapshot) -> String {
    if task_snapshot_slow(snapshot) {
        return format!(
            "，超过 slow_threshold {}ms，slow_tick {}",
            snapshot.slow_threshold_ms, snapshot.slow_tick_count
        );
    }
    if snapshot.slow_tick_count > 0 {
        return format!("，历史 slow_tick {}", snapshot.slow_tick_count);
    }
    String::new()
}

fn task_snapshot_slow(snapshot: &TaskSnapshot) -> bool {
    snapshot
        .last_duration_ms
        .is_some_and(|duration_ms| duration_ms > snapshot.slow_threshold_ms.max(0) as u64)
}

fn task_snapshot_observed_at(snapshot: &TaskSnapshot) -> Option<i64> {
    if !snapshot.enabled {
        return None;
    }
    snapshot
        .issue
        .as_ref()
        .and_then(|issue| issue.since_ms)
        .or(snapshot.last_success_ms)
        .or(Some(snapshot.last_tick_ms))
}

fn task_snapshot_evidence(snapshot: &TaskSnapshot) -> VenueOperationEvidence {
    let mut request_context = vec![
        format!("task={}", snapshot.name),
        format!("enabled={}", snapshot.enabled),
        format!("running={}", snapshot.running),
        format!("started_at_ms={}", snapshot.started_at_ms),
        format!("last_tick_ms={}", snapshot.last_tick_ms),
        format!("max_silence_ms={}", snapshot.max_silence_ms),
        format!("slow_threshold_ms={}", snapshot.slow_threshold_ms),
        format!("slow_tick_count={}", snapshot.slow_tick_count),
        format!("consecutive_failures={}", snapshot.consecutive_failures),
        format!("exit_count={}", snapshot.exit_count),
        format!("restart_count={}", snapshot.restart_count),
        format!("lag_ms={}", snapshot.lag_ms),
    ];
    push_optional_u64_context(
        &mut request_context,
        "retry_after_ms",
        snapshot.retry_after_ms,
    );
    push_optional_u64_context(
        &mut request_context,
        "last_duration_ms",
        snapshot.last_duration_ms,
    );
    push_optional_i64_context(
        &mut request_context,
        "last_success_ms",
        snapshot.last_success_ms,
    );
    push_optional_i64_context(
        &mut request_context,
        "last_exit_at_ms",
        snapshot.last_exit_at_ms,
    );
    push_optional_context(
        &mut request_context,
        "last_error",
        snapshot.last_error.as_deref(),
    );
    push_optional_context(
        &mut request_context,
        "last_exit_reason",
        snapshot.last_exit_reason.as_deref(),
    );
    VenueOperationEvidence {
        method: "task_registry".to_owned(),
        path: snapshot.name.to_owned(),
        checked_at: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        doc_version: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        schema_hash: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        fixture_id: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        parser_test: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        request_builder_test: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        auth_kind: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        request_id: None,
        request_context,
        doc_urls: Vec::new(),
        use_cases: vec!["background_task_health".to_owned()],
        data_kinds: vec!["task_runtime_state".to_owned()],
        rate_scopes: Vec::new(),
        weight: 0,
    }
}

fn push_optional_i64_context(items: &mut Vec<String>, key: &str, value: Option<i64>) {
    if let Some(value) = value {
        items.push(format!("{key}={value}"));
    }
}

fn push_optional_u64_context(items: &mut Vec<String>, key: &str, value: Option<u64>) {
    if let Some(value) = value {
        items.push(format!("{key}={value}"));
    }
}

fn task_issue_problem(issue: &TaskIssue, message: &str) -> ApiProblem {
    let mut problem =
        ApiProblem::new(issue.kind.code(), message.to_owned()).with_source(SOURCE_TASK_REGISTRY);
    problem.details = Some(serde_json::json!({
        "task": issue.name,
        "issue": issue.kind.code(),
        "detail": issue.detail.as_str(),
        "sinceMs": issue.since_ms,
    }));
    problem
}
