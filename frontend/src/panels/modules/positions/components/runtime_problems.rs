use leptos::prelude::*;
use shared_types::{
    ApiProblem, RuntimeProblem, VenueOperationHealth, VenueOperationStatus,
    OP_PRIVATE_WS_ACCOUNT_STREAM, OP_PRIVATE_WS_ORDER_STREAM,
};

use super::super::data::PortfolioAccountAccess;

pub(in crate::panels::modules::positions) fn runtime_problems_banner(
    problems: Memo<Vec<RuntimeProblem>>,
    operation_health: Memo<Vec<VenueOperationHealth>>,
    api_problem: Memo<Option<ApiProblem>>,
    account_access: Memo<PortfolioAccountAccess>,
) -> impl IntoView {
    view! {
        {move || {
            let access = account_access.get();
            let coverage_incomplete = access.coverage_incomplete();
            let rows: Vec<RuntimeProblem> = problems.get().into_iter()
                .filter(|row| !coverage_incomplete || !setup_problem_code(&row.code))
                .collect();
            let health_rows = attention_health(operation_health.get().into_iter()
                .filter(|row| !coverage_incomplete || row.configured != Some(false))
                .filter(|row| !coverage_incomplete || row.source != "portfolio_nav_store")
                .collect());
            let api_problem = api_problem.get()
                .filter(|problem| !coverage_incomplete || !setup_problem_code(&problem.code));
            match (rows.is_empty(), health_rows.is_empty(), api_problem.is_none()) {
                (true, true, true) => ().into_any(),
                _ => render_banner(rows, health_rows, api_problem),
            }
        }}
    }
}

fn setup_problem_code(code: &str) -> bool {
    use shared_types::problem::codes;
    [
        codes::ACCOUNT_FIELD_UNKNOWN,
        codes::BALANCE_EVIDENCE_MISSING,
        codes::POSITION_EVIDENCE_MISSING,
        codes::OPEN_ORDER_EVIDENCE_MISSING,
        codes::NAV_STORAGE_UNAVAILABLE,
    ]
    .contains(&code)
}

fn render_banner(
    rows: Vec<RuntimeProblem>,
    health_rows: Vec<VenueOperationHealth>,
    api_problem: Option<ApiProblem>,
) -> AnyView {
    let api_count = usize::from(api_problem.is_some());
    let shown = rows
        .len()
        .min(3)
        .saturating_add(health_rows.len().min(3))
        .saturating_add(api_count);
    let total = rows
        .len()
        .saturating_add(health_rows.len())
        .saturating_add(api_count);
    let title = problem_title(&rows, &health_rows, api_problem.as_ref());
    let detail_label = detail_label(total, shown);
    view! {
        <details class="runtime-problems">
            <summary>
                <span class="runtime-problems-state">
                    <span class="runtime-problems-dot"></span>
                    <strong>"数据降级"</strong>
                    <em>{format!("{total} 项需处理")}</em>
                </span>
                <span class="runtime-problems-toggle">{detail_label}</span>
            </summary>
            <div class="runtime-problems-body">
                <div class="runtime-problems-chips">
                    {rows.into_iter().take(3).map(problem_chip).collect_view()}
                    {health_rows.into_iter().take(3).map(health_chip).collect_view()}
                    {api_problem.map(api_problem_chip)}
                </div>
                <p>{title}</p>
            </div>
        </details>
    }
    .into_any()
}

fn attention_health(rows: Vec<VenueOperationHealth>) -> Vec<VenueOperationHealth> {
    rows.into_iter()
        .filter(|row| {
            !matches!(
                row.status,
                VenueOperationStatus::Ok | VenueOperationStatus::Unsupported
            ) && !expected_idle_private_stream(row)
        })
        .collect()
}

fn expected_idle_private_stream(row: &VenueOperationHealth) -> bool {
    let expected_message = match row.operation.as_str() {
        OP_PRIVATE_WS_ACCOUNT_STREAM => "等待账户流事件样本",
        OP_PRIVATE_WS_ORDER_STREAM => "等待订单流事件样本",
        _ => return false,
    };
    row.source == "private_ws_runtime"
        && row.status == VenueOperationStatus::Unknown
        && row.supported != Some(false)
        && row.configured == Some(true)
        && row.rows == Some(0)
        && row.error.is_none()
        && row.problem.is_none()
        && row.message.contains(expected_message)
}

fn problem_chip(problem: RuntimeProblem) -> impl IntoView {
    let venue = problem.venue.unwrap_or_else(|| "-".into());
    view! {
        <span class="runtime-problem-chip">
            {venue} " · " {problem.scope} "/" {problem.operation}
        </span>
    }
}

fn health_chip(row: VenueOperationHealth) -> impl IntoView {
    view! {
        <span class="runtime-problem-chip">
            {row.venue} " · " {row.operation} " · " {health_status_label(row.status)}
        </span>
    }
}

fn health_status_label(status: VenueOperationStatus) -> &'static str {
    match status {
        VenueOperationStatus::Ok => "OK",
        VenueOperationStatus::Warn => "WARN",
        VenueOperationStatus::Blocked => "BLOCK",
        VenueOperationStatus::Unknown => "UNKNOWN",
        VenueOperationStatus::Unsupported => "UNSUPPORTED",
    }
}

fn api_problem_chip(problem: ApiProblem) -> impl IntoView {
    view! {
        <span class="runtime-problem-chip">
            "请求失败 · " {problem.code}
        </span>
    }
}

fn problem_title(
    rows: &[RuntimeProblem],
    health_rows: &[VenueOperationHealth],
    api_problem: Option<&ApiProblem>,
) -> String {
    let mut parts = rows
        .iter()
        .map(|problem| {
            let venue = problem.venue.as_deref().unwrap_or("-");
            let mut title = format!(
                "{} · {}/{} · {}",
                venue, problem.scope, problem.operation, problem.message
            );
            if problem.problem.is_some() {
                title.push_str(" · ");
                title.push_str(&api_problem_title(&problem.to_api_problem()));
            }
            title
        })
        .collect::<Vec<_>>();
    parts.extend(health_rows.iter().map(health_title));
    if let Some(problem) = api_problem {
        parts.push(api_problem_title(problem));
    }
    parts.join("；")
}

fn health_title(row: &VenueOperationHealth) -> String {
    let problem = row.error.as_deref().unwrap_or(&row.message);
    let mut title = format!(
        "{} · {} · {} · {}",
        row.venue,
        row.operation,
        health_status_label(row.status),
        problem
    );
    if let Some(problem) = row.problem.as_ref() {
        title.push_str(" · ");
        title.push_str(&api_problem_title(problem));
    }
    title
}

fn api_problem_title(problem: &ApiProblem) -> String {
    let mut parts = vec![format!("请求失败 · {} · {}", problem.code, problem.message)];
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
    if let Some(details) = problem.details.as_ref() {
        parts.push(format!("details {details}"));
    }
    parts.join(" · ")
}

fn more_label(total: usize, shown: usize) -> String {
    if total > shown {
        format!("还有 {} 条", total - shown)
    } else {
        String::new()
    }
}

fn detail_label(total: usize, shown: usize) -> String {
    let hidden = more_label(total, shown);
    if hidden.is_empty() {
        "明细".to_owned()
    } else {
        format!("明细 · {hidden}")
    }
}

#[cfg(test)]
#[path = "runtime_problems_tests.rs"]
mod tests;
