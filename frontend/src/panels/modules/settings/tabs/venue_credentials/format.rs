use super::*;

#[path = "format/status.rs"]
mod status;
#[path = "format/ws.rs"]
mod ws;

pub(super) use status::{
    credential_field_label, credential_field_source_label, credential_link_label,
    operation_status_class, operation_status_label, secret_storage_health_class,
    secret_storage_health_label, selected_summary, trading_runtime_status_class,
    trading_runtime_status_label, trading_runtime_summary, validation_status_label,
};
pub(super) use ws::ws_chip;
#[cfg(test)]
pub(super) use ws::{
    authenticated_runtime_evidence_label, status_label, ws_status_label, ws_submit_gate_label,
    ws_transport_label,
};

pub(super) fn secret_storage_mode_label(mode: SecretStorageMode) -> &'static str {
    match mode {
        SecretStorageMode::EnvFileAtomic => ".env",
        SecretStorageMode::Keychain => "Keychain",
        SecretStorageMode::RuntimeOnly => "Runtime",
    }
}

pub(super) fn runtime_operation_label(operation: &str) -> String {
    match VenueOperationKind::parse(operation) {
        VenueOperationKind::Unknown => operation.to_owned(),
        kind => kind.label_zh().to_owned(),
    }
}

pub(super) fn freshness_label(freshness_ms: Option<i64>) -> String {
    match freshness_ms.filter(|value| *value >= 0) {
        Some(value) => format!("freshness {value}ms"),
        None => "freshness -".to_owned(),
    }
}

pub(super) fn runtime_sample(row: &VenueOperationHealth) -> String {
    let base = match (row.requested, row.rows) {
        (Some(requested), Some(rows)) => format!("{rows}/{requested}"),
        _ => credential_sample(row.configured, row.supported),
    };
    let latency = runtime_latency_sample(row);
    if latency.is_empty() {
        base
    } else if base == "-" {
        latency
    } else {
        format!("{base} · {latency}")
    }
}
pub(super) fn credential_sample(configured: Option<bool>, supported: Option<bool>) -> String {
    match (configured, supported) {
        (_, Some(false)) => "不支持".to_owned(),
        (Some(true), _) => "静态字段完整".to_owned(),
        (Some(false), _) => "静态缺字段".to_owned(),
        _ => "-".to_owned(),
    }
}
pub(super) fn runtime_latency_sample(row: &VenueOperationHealth) -> String {
    let mut parts = Vec::with_capacity(2);
    if let Some(latency_ms) = row.latency_ms {
        let label = match VenueOperationKind::parse(&row.operation) {
            VenueOperationKind::HttpRest => "HTTP RTT",
            _ => "延迟",
        };
        parts.push(format!("{label} {latency_ms}ms"));
    }
    if let Some(latency_p95_ms) = row.latency_p95_ms {
        parts.push(format!("p95 {latency_p95_ms}ms"));
    }
    parts.join(" · ")
}
pub(super) fn runtime_health_message(row: &VenueOperationHealth) -> String {
    let base = row
        .problem
        .as_ref()
        .map(runtime_problem_label)
        .unwrap_or_else(|| {
            row.error
                .as_deref()
                .filter(|error| !error.is_empty())
                .unwrap_or(&row.message)
                .to_owned()
        });
    format!("{base}{}", runtime_retry_suffix(row))
}
pub(super) fn runtime_problem_label(problem: &ApiProblem) -> String {
    let request = problem
        .request_id
        .as_deref()
        .map(|request_id| format!(" · request_id {request_id}"))
        .unwrap_or_default();
    let source = problem
        .source
        .as_deref()
        .map(|source| format!(" · source {source}"))
        .unwrap_or_default();
    format!("{}：{}{}{}", problem.code, problem.message, request, source)
}

pub(super) fn runtime_retry_suffix(row: &VenueOperationHealth) -> String {
    let problem_retry = row
        .problem
        .as_ref()
        .and_then(|problem| problem.retry_after_ms);
    let runtime_retry = row
        .retry_after_ms
        .filter(|value| Some(*value) != problem_retry);
    let mut parts = Vec::with_capacity(2);
    if let Some(retry_after_ms) = problem_retry {
        parts.push(format!("retry {retry_after_ms}ms"));
    }
    if let Some(retry_after_ms) = runtime_retry {
        parts.push(format!("runtime retry {retry_after_ms}ms"));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!(" · {}", parts.join(" · "))
    }
}

pub(super) fn runtime_evidence_summary(row: &VenueOperationHealth) -> String {
    if let Some(evidence) = row.evidence.as_ref() {
        return runtime_evidence_summary_from(evidence);
    }
    row.problem
        .as_ref()
        .and_then(|problem| problem.request_id.as_deref())
        .map(|request_id| format!("request_id {request_id}"))
        .unwrap_or_else(|| "-".to_owned())
}

pub(super) fn runtime_request_id_label(row: &VenueOperationHealth) -> String {
    row.evidence
        .as_ref()
        .and_then(|evidence| evidence.request_id.as_deref())
        .map(|request_id| format!("request_id {request_id}"))
        .or_else(|| {
            row.problem
                .as_ref()
                .and_then(|problem| problem.request_id.as_deref())
                .map(|request_id| format!("problem_request_id {request_id}"))
        })
        .unwrap_or_else(|| "request_id -".to_owned())
}

pub(super) fn runtime_evidence_summary_from(evidence: &VenueOperationEvidence) -> String {
    let request = evidence
        .request_id
        .as_deref()
        .map(|request_id| format!(" · request_id {request_id}"))
        .unwrap_or_default();
    let live_proof = live_write_context_summary(evidence)
        .map(|summary| format!(" · live proof {summary}"))
        .unwrap_or_default();
    format!(
        "{} {}{}{}",
        evidence.method, evidence.path, request, live_proof
    )
}

pub(super) fn runtime_evidence_detail(row: &VenueOperationHealth) -> String {
    let Some(evidence) = row.evidence.as_ref() else {
        return runtime_health_message(row);
    };
    let docs = join_or_dash(&evidence.doc_urls);
    let context = join_or_dash(&evidence.request_context);
    let request_id = evidence.request_id.as_deref().unwrap_or("-");
    let live_proof = live_write_context_summary(evidence)
        .map(|summary| format!(" · live proof {summary}"))
        .unwrap_or_default();
    format!(
        "{} {}{} · request_id {} · checked {} · doc {} · schema {} · fixture {} · parser {} · builder {} · auth {} · docs {} · context {}",
        evidence.method,
        evidence.path,
        live_proof,
        request_id,
        evidence.checked_at,
        evidence.doc_version,
        evidence.schema_hash,
        evidence.fixture_id,
        evidence.parser_test,
        evidence.request_builder_test,
        evidence.auth_kind,
        docs,
        context
    )
}

fn live_write_context_summary(evidence: &VenueOperationEvidence) -> Option<String> {
    let mut values = evidence
        .request_context
        .iter()
        .filter(|item| is_live_write_context_key(item))
        .cloned()
        .collect::<Vec<_>>();
    values.dedup();
    (!values.is_empty()).then(|| values.join(" · "))
}

fn is_live_write_context_key(item: &str) -> bool {
    item.starts_with("live_place_remote_proof=")
        || item.starts_with("live_cancel_remote_proof=")
        || item.starts_with("does_not_grant_live_write=")
        || item.starts_with("probe_scope=")
        || item.starts_with("probe_source=")
        || item.starts_with("safe_order")
}

pub(super) fn join_or_dash(values: &[String]) -> String {
    if values.is_empty() {
        "-".to_owned()
    } else {
        values.join("/")
    }
}
