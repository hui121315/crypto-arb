use super::*;

mod view;

use view::api_status_slot_body;

pub(super) const API_SLOT_LABEL: &str = "TradingAPI";

pub(in crate::panels::status_bar) fn api_status_slot(
    operation_health: Memo<Option<VenueOperationHealthSnapshot>>,
    operation_problem: Memo<Option<ApiProblem>>,
    environment: Memo<Option<ExecutionEnvironment>>,
) -> impl IntoView {
    api_status_slot_body(operation_health, operation_problem, environment)
}

#[cfg(test)]
pub(super) fn api_degraded(
    operation_health: Option<&VenueOperationHealthSnapshot>,
    operation_problem: Option<&ApiProblem>,
) -> bool {
    api_degraded_for_environment(
        operation_health,
        operation_problem,
        Some(ExecutionEnvironment::Live),
    )
}

pub(super) fn api_degraded_for_environment(
    operation_health: Option<&VenueOperationHealthSnapshot>,
    operation_problem: Option<&ApiProblem>,
    environment: Option<ExecutionEnvironment>,
) -> bool {
    operation_problem.is_some()
        || operation_health
            .map(|snapshot| {
                let total = api_operation_count(snapshot);
                if total > 0 {
                    let configured = api_configured_count(snapshot);
                    if configured == 0 {
                        environment != Some(ExecutionEnvironment::Paper)
                    } else {
                        api_attention_count_for_environment(snapshot, environment) > 0
                            || api_transport_attention_count(snapshot) > 0
                    }
                } else {
                    true
                }
            })
            .unwrap_or(true)
}

#[cfg(test)]
pub(super) fn api_slot_class(
    operation_health: Option<&VenueOperationHealthSnapshot>,
    operation_problem: Option<&ApiProblem>,
) -> &'static str {
    api_slot_class_for_environment(
        operation_health,
        operation_problem,
        Some(ExecutionEnvironment::Live),
    )
}

pub(super) fn api_slot_class_for_environment(
    operation_health: Option<&VenueOperationHealthSnapshot>,
    operation_problem: Option<&ApiProblem>,
    environment: Option<ExecutionEnvironment>,
) -> &'static str {
    if api_degraded_for_environment(operation_health, operation_problem, environment) {
        "slot degraded"
    } else {
        "slot"
    }
}

#[cfg(test)]
pub(super) fn api_title(
    operation_health: Option<&VenueOperationHealthSnapshot>,
    operation_problem: Option<&ApiProblem>,
) -> String {
    api_title_for_environment(
        operation_health,
        operation_problem,
        Some(ExecutionEnvironment::Live),
    )
}

pub(super) fn api_title_for_environment(
    operation_health: Option<&VenueOperationHealthSnapshot>,
    operation_problem: Option<&ApiProblem>,
    environment: Option<ExecutionEnvironment>,
) -> String {
    let operation = operation_health
        .and_then(most_severe_api_operation)
        .map(operation_summary)
        .unwrap_or_default();
    let transport = operation_health
        .and_then(most_severe_api_transport_operation)
        .map(|row| format!("Transport：{}", operation_summary(row)))
        .unwrap_or_default();
    let operation_problem = operation_problem
        .map(api_problem_summary)
        .unwrap_or_default();
    title_parts([
        operation,
        transport,
        missing_api_evidence_summary(operation_health),
        configuration_summary_for_environment(operation_health, environment),
        operation_problem,
    ])
}

#[cfg(test)]
pub(super) fn api_label(operation_health: Option<&VenueOperationHealthSnapshot>) -> String {
    api_label_for_environment(operation_health, Some(ExecutionEnvironment::Live))
}

pub(super) fn api_label_for_environment(
    operation_health: Option<&VenueOperationHealthSnapshot>,
    environment: Option<ExecutionEnvironment>,
) -> String {
    if let Some(snapshot) = operation_health {
        let total = api_operation_count(snapshot);
        if total > 0 {
            let configured = api_configured_count(snapshot);
            if configured == 0 {
                return if environment == Some(ExecutionEnvironment::Paper) {
                    "模拟无需".into()
                } else {
                    "需配置凭证".into()
                };
            }
            let usable = api_usable_count(snapshot);
            return format!("{usable}可用/{configured}配置");
        }
    }
    "无证据".into()
}

pub(super) fn api_configured_count(snapshot: &VenueOperationHealthSnapshot) -> usize {
    snapshot
        .rows
        .iter()
        .filter(|row| is_api_operation_row(row) && row.configured == Some(true))
        .count()
}

pub(super) fn api_usable_count(snapshot: &VenueOperationHealthSnapshot) -> usize {
    snapshot
        .rows
        .iter()
        .filter(|row| {
            is_api_operation_row(row) && row.configured == Some(true) && row.is_currently_usable()
        })
        .count()
}

#[cfg(test)]
pub(super) fn api_label_with_problem(
    operation_health: Option<&VenueOperationHealthSnapshot>,
    operation_problem: Option<&ApiProblem>,
) -> String {
    api_label_with_problem_for_environment(
        operation_health,
        operation_problem,
        Some(ExecutionEnvironment::Live),
    )
}

pub(super) fn api_label_with_problem_for_environment(
    operation_health: Option<&VenueOperationHealthSnapshot>,
    operation_problem: Option<&ApiProblem>,
    environment: Option<ExecutionEnvironment>,
) -> String {
    if operation_problem.is_some() {
        "异常".into()
    } else {
        api_label_for_environment(operation_health, environment)
    }
}

pub(super) fn missing_api_evidence_summary(
    operation_health: Option<&VenueOperationHealthSnapshot>,
) -> String {
    match operation_health {
        Some(snapshot) if api_operation_count(snapshot) == 0 => {
            "API 运行态无可用证据：等待 venue-operation-health API 行".into()
        }
        None => "API 运行态无可用证据：等待 venue-operation-health 快照".into(),
        Some(_) => String::new(),
    }
}

pub(super) fn most_severe_api_operation(
    snapshot: &VenueOperationHealthSnapshot,
) -> Option<&VenueOperationHealth> {
    let configured = snapshot
        .rows
        .iter()
        .filter(|row| is_api_operation_row(row) && row.configured == Some(true))
        .max_by_key(|row| status_rank(row.status));
    configured.or_else(|| {
        snapshot
            .rows
            .iter()
            .filter(|row| is_api_operation_row(row))
            .max_by_key(|row| status_rank(row.status))
    })
}

pub(super) fn most_severe_api_transport_operation(
    snapshot: &VenueOperationHealthSnapshot,
) -> Option<&VenueOperationHealth> {
    snapshot
        .rows
        .iter()
        .filter(|row| is_authenticated_api_transport_row(row))
        .max_by_key(|row| status_rank(row.status))
}

pub(super) fn api_operation_count(snapshot: &VenueOperationHealthSnapshot) -> usize {
    snapshot
        .rows
        .iter()
        .filter(|row| is_api_operation_row(row))
        .count()
}

pub(super) fn api_attention_count_for_environment(
    snapshot: &VenueOperationHealthSnapshot,
    environment: Option<ExecutionEnvironment>,
) -> usize {
    let configured = api_configured_count(snapshot);
    snapshot
        .rows
        .iter()
        .filter(|row| {
            is_api_operation_row(row)
                && if configured > 0 {
                    row.configured == Some(true)
                } else {
                    environment != Some(ExecutionEnvironment::Paper)
                        && row.configured != Some(false)
                }
                && row_needs_attention(row.status)
        })
        .count()
}

pub(super) fn api_transport_attention_count(snapshot: &VenueOperationHealthSnapshot) -> usize {
    snapshot
        .rows
        .iter()
        .filter(|row| is_authenticated_api_transport_row(row) && row_needs_attention(row.status))
        .count()
}

pub(super) fn is_api_operation_row(row: &VenueOperationHealth) -> bool {
    row.supported != Some(false)
        && VenueOperationKind::parse(&row.operation).is_trading_api_status_row()
}

pub(super) fn is_api_transport_row(row: &VenueOperationHealth) -> bool {
    row.supported != Some(false)
        && VenueOperationKind::parse(&row.operation).is_api_transport_status_row()
}

fn is_authenticated_api_transport_row(row: &VenueOperationHealth) -> bool {
    is_api_transport_row(row)
        && row.evidence.as_ref().is_some_and(|evidence| {
            let auth_kind = evidence.auth_kind.trim();
            !auth_kind.is_empty()
                && !auth_kind.eq_ignore_ascii_case("public")
                && auth_kind != UNRECORDED_EVIDENCE_MARKER
        })
}

pub(super) fn row_needs_attention(status: VenueOperationStatus) -> bool {
    matches!(
        status,
        VenueOperationStatus::Warn | VenueOperationStatus::Blocked | VenueOperationStatus::Unknown
    )
}
