use super::*;

pub(super) fn searchable_operation_health_text(row: &VenueOperationHealth) -> String {
    let mut parts = vec![
        row.venue.as_str(),
        row.operation.as_str(),
        row.source.as_str(),
        status_label(row.status),
        row.message.as_str(),
    ];
    let kind = VenueOperationKind::parse(&row.operation);
    parts.push(kind.class().label_zh());
    parts.push(kind.label_zh());
    parts.push(operation_configured_label(row));
    parts.push(operation_capability_label(row));
    parts.push(operation_usable_label(row));
    if let Some(error) = row.error.as_deref() {
        parts.push(error);
    }
    if let Some(problem) = row.problem.as_ref() {
        parts.push(problem.code.as_str());
        parts.push(problem.message.as_str());
    }
    if let Some(evidence) = row.evidence.as_ref() {
        parts.push(evidence.method.as_str());
        parts.push(evidence.path.as_str());
        parts.push("checked_at");
        parts.push(evidence.checked_at.as_str());
        parts.push("doc_version");
        parts.push(evidence.doc_version.as_str());
        parts.push("schema_hash");
        parts.push(evidence.schema_hash.as_str());
        parts.push("fixture_id");
        parts.push(evidence.fixture_id.as_str());
        parts.push("parser_test");
        parts.push(evidence.parser_test.as_str());
        parts.push("request_builder_test");
        parts.push(evidence.request_builder_test.as_str());
        parts.push("auth_kind");
        parts.push(evidence.auth_kind.as_str());
        parts.extend(evidence.doc_urls.iter().map(String::as_str));
        parts.extend(evidence.use_cases.iter().map(String::as_str));
        parts.extend(evidence.data_kinds.iter().map(String::as_str));
        parts.extend(evidence.rate_scopes.iter().map(String::as_str));
        parts.extend(evidence.request_context.iter().map(String::as_str));
        if let Some(request_id) = evidence.request_id.as_deref() {
            parts.push(request_id);
        }
    }
    let mut text = parts.join(" ");
    if let Some(latency_ms) = row.latency_ms {
        text.push_str(&format!(" latency_ms={latency_ms}ms"));
        if VenueOperationKind::parse(&row.operation) == VenueOperationKind::HttpRest {
            text.push_str(&format!(" http_rtt_ms={latency_ms}ms HTTP_RTT"));
        }
    }
    if let Some(latency_p95_ms) = row.latency_p95_ms {
        text.push_str(&format!(" latency_p95_ms={latency_p95_ms}ms"));
        if VenueOperationKind::parse(&row.operation) == VenueOperationKind::HttpRest {
            text.push_str(&format!(" http_rtt_p95_ms={latency_p95_ms}ms"));
        }
    }
    if let Some(retry_after_ms) = row.retry_after_ms {
        text.push_str(&format!(" retry_after_ms={retry_after_ms}ms"));
    }
    if let Some(problem) = row.problem.as_ref() {
        append_problem_search_text(&mut text, problem);
    }
    text
}

fn append_problem_search_text(text: &mut String, problem: &shared_types::ApiProblem) {
    if let Some(status) = problem.status {
        text.push_str(&format!(" problem_status={status}"));
    }
    if let Some(request_id) = problem.request_id.as_deref() {
        text.push_str(&format!(" problem_request_id={request_id}"));
    }
    if let Some(source) = problem.source.as_deref() {
        text.push_str(&format!(" problem_source={source}"));
    }
    if let Some(retry_after_ms) = problem.retry_after_ms {
        text.push_str(&format!(" problem_retry_after_ms={retry_after_ms}ms"));
    }
}

pub(super) fn operation_health_message(row: &VenueOperationHealth) -> String {
    let evidence = operation_evidence_suffix(row.evidence.as_ref());
    let retry = operation_retry_suffix(row);
    if let Some(problem) = row.problem.as_ref() {
        return format!("{}{}{}", operation_problem_label(problem), retry, evidence);
    }
    format!(
        "{}{}{}",
        row.error
            .as_deref()
            .filter(|error| !error.is_empty())
            .unwrap_or(&row.message),
        retry,
        evidence
    )
}

pub(super) fn operation_problem_label(problem: &shared_types::ApiProblem) -> String {
    let status = problem
        .status
        .map(|status| format!("HTTP {status}"))
        .unwrap_or_default();
    let request = problem
        .request_id
        .as_deref()
        .map(|request_id| format!("request_id {request_id}"))
        .unwrap_or_default();
    let source = problem
        .source
        .as_deref()
        .map(|source| format!("source {source}"))
        .unwrap_or_default();
    let suffix = [status, request, source]
        .into_iter()
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
    if suffix.is_empty() {
        format!("{}：{}", problem.code, problem.message)
    } else {
        format!("{}：{} · {}", problem.code, problem.message, suffix)
    }
}

pub(super) fn diagnostics_stale_problem_message(
    label: &str,
    problem: &shared_types::ApiProblem,
) -> String {
    let retry = problem
        .retry_after_ms
        .map(|retry_after_ms| format!(" · retry {retry_after_ms}ms"))
        .unwrap_or_default();
    format!(
        "{label}，保留上次数据 · {}{}",
        operation_problem_label(problem),
        retry
    )
}

pub(super) fn operation_snapshot_problem_message(problem: &shared_types::ApiProblem) -> String {
    diagnostics_stale_problem_message("运行态矩阵刷新失败", problem)
}

pub(super) fn operation_retry_suffix(row: &VenueOperationHealth) -> String {
    let problem_retry = row
        .problem
        .as_ref()
        .and_then(|problem| problem.retry_after_ms);
    let mut parts = Vec::with_capacity(2);
    if let Some(retry_after_ms) = problem_retry {
        parts.push(format!("retry {retry_after_ms}ms"));
    }
    if let Some(retry_after_ms) = row
        .retry_after_ms
        .filter(|value| Some(*value) != problem_retry)
    {
        parts.push(format!("runtime retry {retry_after_ms}ms"));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!(" · {}", parts.join(" · "))
    }
}

pub(super) fn operation_health_title(row: &VenueOperationHealth) -> String {
    let base = operation_health_message(row);
    let evidence = row
        .evidence
        .as_ref()
        .map(operation_evidence_detail)
        .unwrap_or_default();
    [base, evidence]
        .into_iter()
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

pub(super) fn operation_evidence_suffix(evidence: Option<&VenueOperationEvidence>) -> String {
    let Some(evidence) = evidence else {
        return String::new();
    };
    let kinds = join_or_dash(&evidence.data_kinds);
    let use_cases = join_or_dash(&evidence.use_cases);
    let request = evidence
        .request_id
        .as_deref()
        .map(|request_id| format!(" / request_id {request_id}"))
        .unwrap_or_default();
    let context = evidence
        .request_context
        .first()
        .map(|value| format!(" / context {value}"))
        .unwrap_or_default();
    format!(
        " · 官方 {} / {} / weight {}{}{}",
        kinds, use_cases, evidence.weight, request, context
    )
}

pub(super) fn operation_evidence_detail(evidence: &VenueOperationEvidence) -> String {
    let docs = join_or_dash(&evidence.doc_urls);
    let scopes = join_or_dash(&evidence.rate_scopes);
    let request = evidence
        .request_id
        .as_deref()
        .map(|request_id| format!(" · request_id {request_id}"))
        .unwrap_or_default();
    let context = if evidence.request_context.is_empty() {
        String::new()
    } else {
        format!(" · context {}", join_or_dash(&evidence.request_context))
    };
    format!(
        "{} {} · checked {} · doc {} · schema {} · fixture {} · parser {} · builder {} · auth {} · scope {} · docs {}{}{}",
        evidence.method,
        evidence.path,
        evidence.checked_at,
        evidence.doc_version,
        evidence.schema_hash,
        evidence.fixture_id,
        evidence.parser_test,
        evidence.request_builder_test,
        evidence.auth_kind,
        scopes,
        docs,
        request,
        context
    )
}

pub(super) fn join_or_dash(values: &[String]) -> String {
    if values.is_empty() {
        "-".to_owned()
    } else {
        values.join("/")
    }
}

pub(super) fn status_label(status: VenueOperationStatus) -> &'static str {
    match status {
        VenueOperationStatus::Ok => "正常",
        VenueOperationStatus::Warn => "观察",
        VenueOperationStatus::Blocked => "阻断",
        VenueOperationStatus::Unknown => "待验证",
        VenueOperationStatus::Unsupported => "不支持",
    }
}

pub(super) fn status_pill_class(status: VenueOperationStatus) -> &'static str {
    match status {
        VenueOperationStatus::Ok => "status-pill ready",
        VenueOperationStatus::Warn | VenueOperationStatus::Unknown => "status-pill pending",
        VenueOperationStatus::Blocked => "status-pill blocked",
        VenueOperationStatus::Unsupported => "status-pill",
    }
}
