use super::*;

pub(super) fn operation_summary(row: &VenueOperationHealth) -> String {
    let latency = operation_latency_summary(row);
    let retry = row
        .retry_after_ms
        .map(|ms| format!(" · retry {ms}ms"))
        .unwrap_or_default();
    let problem = row
        .problem
        .as_ref()
        .map(|problem| format!(" · {}", api_problem_summary(problem)))
        .unwrap_or_default();
    let evidence = row
        .evidence
        .as_ref()
        .map(|evidence| format!(" · {}", operation_evidence_summary(evidence)))
        .unwrap_or_default();
    let error = if problem.is_empty() {
        row.error
            .as_ref()
            .map(|value| format!(" · {value}"))
            .unwrap_or_default()
    } else {
        String::new()
    };
    format!(
        "{} / {} / {}：{}{}{}{}{}{}",
        row.venue,
        row.operation,
        operation_status_label(row.status),
        row.message,
        latency,
        retry,
        error,
        problem,
        evidence
    )
}

pub(super) fn operation_latency_summary(row: &VenueOperationHealth) -> String {
    let label = operation_latency_label(row);
    match (row.latency_ms, row.latency_p95_ms) {
        (Some(latency), Some(p95)) => format!(" · {label} {latency}ms / p95≤{p95}ms"),
        (Some(latency), None) => format!(" · {label} {latency}ms"),
        (None, Some(p95)) => format!(" · {label} p95≤{p95}ms"),
        (None, None) => String::new(),
    }
}

fn operation_latency_label(row: &VenueOperationHealth) -> &'static str {
    if VenueOperationKind::parse(&row.operation) == VenueOperationKind::HttpRest {
        "HTTP RTT"
    } else {
        "HTTP延迟"
    }
}

pub(super) fn operation_evidence_summary(evidence: &VenueOperationEvidence) -> String {
    let data_kind = evidence
        .data_kinds
        .first()
        .map(String::as_str)
        .unwrap_or("-");
    let doc_count = evidence.doc_urls.len();
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
        "官方证据 {data_kind} / docs {doc_count} / checked {} / auth {} / schema {} / fixture {}{request}{context}",
        evidence.checked_at, evidence.auth_kind, evidence.schema_hash, evidence.fixture_id
    )
}

pub(super) fn status_rank(status: VenueOperationStatus) -> u8 {
    match status {
        VenueOperationStatus::Blocked => 5,
        VenueOperationStatus::Warn => 4,
        VenueOperationStatus::Unknown => 3,
        VenueOperationStatus::Unsupported => 2,
        VenueOperationStatus::Ok => 1,
    }
}

pub(super) fn operation_status_label(status: VenueOperationStatus) -> &'static str {
    match status {
        VenueOperationStatus::Ok => "正常",
        VenueOperationStatus::Warn => "观察",
        VenueOperationStatus::Blocked => "阻断",
        VenueOperationStatus::Unknown => "待验证",
        VenueOperationStatus::Unsupported => "不支持",
    }
}

pub(super) fn problem_summary(problem: &RuntimeProblem) -> String {
    let venue = problem.venue.as_deref().unwrap_or("-");
    let retry = problem
        .retry_after_ms
        .map(|retry_after_ms| format!(" · retry {retry_after_ms}ms"))
        .unwrap_or_default();
    format!(
        "{} / {} / {} / {}：{}{}",
        venue, problem.scope, problem.operation, problem.code, problem.message, retry
    )
}

pub(super) fn api_problem_summary(problem: &ApiProblem) -> String {
    let mut parts = Vec::with_capacity(5);
    parts.push(format!("code {}", problem.code));
    if let Some(status) = problem.status {
        parts.push(format!("HTTP {status}"));
    }
    if let Some(request_id) = problem.request_id.as_deref() {
        parts.push(format!("request_id {request_id}"));
    }
    if let Some(retry_after_ms) = problem.retry_after_ms {
        parts.push(format!("retry {retry_after_ms}ms"));
    }
    if let Some(source) = problem.source.as_deref() {
        parts.push(format!("source {source}"));
    }
    format!("请求失败：{} · {}", problem.message, parts.join(" · "))
}

pub(super) fn title_parts(parts: impl IntoIterator<Item = String>) -> String {
    parts
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("；")
}

pub(super) fn title_with_api_problem(
    base: impl Into<String>,
    problem: Option<&ApiProblem>,
) -> String {
    title_parts([
        base.into(),
        problem.map(api_problem_summary).unwrap_or_default(),
    ])
}
