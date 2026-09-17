use super::*;
use crate::panels::shared::execution_environment_label;

#[component]
pub fn ExecutionModeSlot(
    status: Memo<Option<TradingStatusResponse>>,
    problem: Memo<Option<ApiProblem>>,
) -> impl IntoView {
    view! {
        <div
            class=move || execution_mode_class(status.get().as_ref(), problem.get().as_ref())
            title=move || execution_mode_title(status.get().as_ref(), problem.get().as_ref())
        >
            <span class=move || dot_class(problem.get().is_some())></span>
            <span class="slot-label">"环境"</span>
            <span class="num">{move || {
                status
                    .get()
                    .map(|value| environment_label(value.environment))
                    .unwrap_or("-")
            }}</span>
        </div>
    }
}

pub(super) fn execution_mode_class(
    status: Option<&TradingStatusResponse>,
    problem: Option<&ApiProblem>,
) -> &'static str {
    if problem.is_some() {
        "slot degraded"
    } else {
        match status.map(|value| value.environment) {
            Some(ExecutionEnvironment::Live) => "slot live",
            Some(ExecutionEnvironment::Paper) => "slot paper",
            None => "slot unknown",
        }
    }
}

pub(super) fn execution_mode_title(
    status: Option<&TradingStatusResponse>,
    problem: Option<&ApiProblem>,
) -> String {
    match (status, problem) {
        (_, Some(problem)) => api_problem_summary(problem),
        (Some(status), None) => format!(
            "后端执行环境：{}；adapter：{}；挂单：{}",
            environment_label(status.environment),
            status.adapter,
            status.open_order_count
        ),
        (None, None) => "正在读取后端执行环境".into(),
    }
}

pub(super) fn environment_label(environment: ExecutionEnvironment) -> &'static str {
    execution_environment_label(environment)
}
