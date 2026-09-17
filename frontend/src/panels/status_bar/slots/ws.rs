use super::*;

pub(super) const WS_SLOT_LABEL: &str = "PrivateWS";

#[component]
pub fn WsStatusSlot(
    operation_health: Memo<Option<VenueOperationHealthSnapshot>>,
    operation_problem: Memo<Option<ApiProblem>>,
    environment: Memo<Option<ExecutionEnvironment>>,
) -> impl IntoView {
    view! {
        <div
            data-testid="status-private-ws"
            class=move || {
                let operation_problem = operation_problem.get();
                ws_slot_class_for_environment(
                    operation_health.get().as_ref(),
                    operation_problem.as_ref(),
                    environment.get(),
                )
            }
            title=move || {
                let operation_problem = operation_problem.get();
                ws_title_for_environment(
                    operation_health.get().as_ref(),
                    operation_problem.as_ref(),
                    environment.get(),
                )
            }
        >
            <span class=move || {
                let operation_problem = operation_problem.get();
                dot_class(ws_degraded_for_environment(
                    operation_health.get().as_ref(),
                    operation_problem.as_ref(),
                    environment.get(),
                ))
            }></span>
            <span class="slot-label">{WS_SLOT_LABEL}</span>
            <span class="num">{move || {
                let operation_problem = operation_problem.get();
                ws_label_with_problem_for_environment(
                    operation_health.get().as_ref(),
                    operation_problem.as_ref(),
                    environment.get(),
                )
            }}</span>
        </div>
    }
}

pub(super) fn ws_slot_class_for_environment(
    operation_health: Option<&VenueOperationHealthSnapshot>,
    operation_problem: Option<&ApiProblem>,
    environment: Option<ExecutionEnvironment>,
) -> &'static str {
    if ws_degraded_for_environment(operation_health, operation_problem, environment) {
        "slot degraded"
    } else {
        "slot"
    }
}

#[cfg(test)]
pub(super) fn ws_degraded(
    operation_health: Option<&VenueOperationHealthSnapshot>,
    operation_problem: Option<&ApiProblem>,
) -> bool {
    ws_degraded_for_environment(
        operation_health,
        operation_problem,
        Some(ExecutionEnvironment::Live),
    )
}

pub(super) fn ws_degraded_for_environment(
    operation_health: Option<&VenueOperationHealthSnapshot>,
    operation_problem: Option<&ApiProblem>,
    environment: Option<ExecutionEnvironment>,
) -> bool {
    operation_problem.is_some()
        || operation_health
            .map(|snapshot| {
                let total = ws_operation_count(snapshot);
                if total > 0 {
                    let configured = ws_configured_count(snapshot);
                    if configured == 0 {
                        environment != Some(ExecutionEnvironment::Paper)
                    } else {
                        ws_attention_count(snapshot) > 0
                    }
                } else {
                    true
                }
            })
            .unwrap_or(true)
}

#[cfg(test)]
pub(super) fn ws_label(operation_health: Option<&VenueOperationHealthSnapshot>) -> String {
    ws_label_for_environment(operation_health, Some(ExecutionEnvironment::Live))
}

pub(super) fn ws_label_for_environment(
    operation_health: Option<&VenueOperationHealthSnapshot>,
    environment: Option<ExecutionEnvironment>,
) -> String {
    if let Some(snapshot) = operation_health {
        let total = ws_operation_count(snapshot);
        if total > 0 {
            let configured = ws_configured_count(snapshot);
            if configured == 0 {
                return if environment == Some(ExecutionEnvironment::Paper) {
                    "模拟无需".into()
                } else {
                    "需配置凭证".into()
                };
            }
            let usable = ws_usable_count(snapshot);
            return format!("{usable}可用/{configured}配置");
        }
    }
    "无证据".into()
}

#[cfg(test)]
pub(super) fn ws_label_with_problem(
    operation_health: Option<&VenueOperationHealthSnapshot>,
    operation_problem: Option<&ApiProblem>,
) -> String {
    ws_label_with_problem_for_environment(
        operation_health,
        operation_problem,
        Some(ExecutionEnvironment::Live),
    )
}

pub(super) fn ws_label_with_problem_for_environment(
    operation_health: Option<&VenueOperationHealthSnapshot>,
    operation_problem: Option<&ApiProblem>,
    environment: Option<ExecutionEnvironment>,
) -> String {
    if operation_problem.is_some() {
        "异常".into()
    } else {
        ws_label_for_environment(operation_health, environment)
    }
}

#[cfg(test)]
pub(super) fn ws_title(
    operation_health: Option<&VenueOperationHealthSnapshot>,
    operation_problem: Option<&ApiProblem>,
) -> String {
    ws_title_for_environment(
        operation_health,
        operation_problem,
        Some(ExecutionEnvironment::Live),
    )
}

pub(super) fn ws_title_for_environment(
    operation_health: Option<&VenueOperationHealthSnapshot>,
    operation_problem: Option<&ApiProblem>,
    environment: Option<ExecutionEnvironment>,
) -> String {
    let operation = operation_health
        .and_then(most_severe_ws_operation)
        .map(operation_summary)
        .unwrap_or_else(|| "等待交易所私有 WS 运行态".into());
    let problem = operation_problem
        .map(api_problem_summary)
        .unwrap_or_default();
    title_parts([
        operation,
        configuration_summary_for_environment(operation_health, environment),
        problem,
    ])
}

pub(super) fn ws_configured_count(snapshot: &VenueOperationHealthSnapshot) -> usize {
    snapshot
        .rows
        .iter()
        .filter(|row| is_private_ws_row(row) && row.configured == Some(true))
        .count()
}

pub(super) fn ws_usable_count(snapshot: &VenueOperationHealthSnapshot) -> usize {
    snapshot
        .rows
        .iter()
        .filter(|row| {
            is_private_ws_row(row) && row.configured == Some(true) && row.is_currently_usable()
        })
        .count()
}

pub(super) fn most_severe_ws_operation(
    snapshot: &VenueOperationHealthSnapshot,
) -> Option<&VenueOperationHealth> {
    let configured = snapshot
        .rows
        .iter()
        .filter(|row| is_private_ws_row(row) && row.configured == Some(true))
        .max_by_key(|row| (status_rank(row.status), ws_operation_priority(row)));
    configured.or_else(|| {
        snapshot
            .rows
            .iter()
            .filter(|row| is_private_ws_row(row))
            .max_by_key(|row| (status_rank(row.status), ws_operation_priority(row)))
    })
}

fn ws_operation_priority(row: &VenueOperationHealth) -> u8 {
    match VenueOperationKind::parse(&row.operation) {
        VenueOperationKind::PrivateWsOrderStream => 4,
        VenueOperationKind::PrivateWsAccountStream => 3,
        VenueOperationKind::PrivateWsSubscribe => 2,
        VenueOperationKind::PrivateWsSession => 1,
        _ => 0,
    }
}

pub(super) fn ws_operation_count(snapshot: &VenueOperationHealthSnapshot) -> usize {
    snapshot
        .rows
        .iter()
        .filter(|row| is_private_ws_row(row))
        .count()
}

pub(super) fn ws_attention_count(snapshot: &VenueOperationHealthSnapshot) -> usize {
    snapshot
        .rows
        .iter()
        .filter(|row| {
            is_private_ws_row(row)
                && row.configured == Some(true)
                && row_needs_attention(row.status)
        })
        .count()
}

pub(super) fn is_private_ws_row(row: &VenueOperationHealth) -> bool {
    row.supported != Some(false)
        && VenueOperationKind::parse(&row.operation).is_private_ws_status_row()
}
