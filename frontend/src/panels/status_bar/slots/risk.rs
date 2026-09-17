use super::*;

#[component]
pub fn RiskSlot(
    status: Memo<Option<RiskStatusSlot>>,
    problem: Memo<Option<ApiProblem>>,
    on_click: Callback<()>,
) -> impl IntoView {
    view! {
        <button
            class=move || {
                let problem = problem.get();
                risk_slot_class_with_problem(status.get(), problem.as_ref())
            }
            title=move || {
                let problem = problem.get();
                risk_title_with_problem(status.get(), problem.as_ref())
            }
            on:click=move |_| on_click.run(())
        >
            <span class=move || {
                let problem = problem.get();
                risk_dot_class_with_problem(status.get(), problem.as_ref())
            }></span>
            <span class="slot-label">"Risk"</span>
            <span class="num">{move || {
                let problem = problem.get();
                risk_status_label_with_problem(status.get(), problem.as_ref())
            }}</span>
        </button>
    }
}

pub(super) fn risk_slot_class_with_problem(
    status: Option<RiskStatusSlot>,
    problem: Option<&ApiProblem>,
) -> &'static str {
    if problem.is_some() || risk_degraded(status) {
        "slot clickable degraded"
    } else {
        "slot clickable"
    }
}

pub(super) fn risk_dot_class_with_problem(
    status: Option<RiskStatusSlot>,
    problem: Option<&ApiProblem>,
) -> &'static str {
    dot_class(problem.is_some() || risk_degraded(status))
}

#[cfg(test)]
pub(super) fn risk_slot_class(status: Option<RiskStatusSlot>) -> &'static str {
    risk_slot_class_with_problem(status, None)
}

#[cfg(test)]
pub(super) fn risk_dot_class(status: Option<RiskStatusSlot>) -> &'static str {
    risk_dot_class_with_problem(status, None)
}

pub(super) fn risk_degraded(status: Option<RiskStatusSlot>) -> bool {
    !matches!(status, Some(RiskStatusSlot::Ok))
}

pub(super) fn risk_status_label(status: Option<RiskStatusSlot>) -> &'static str {
    match status {
        Some(RiskStatusSlot::Ok) => "OK",
        Some(RiskStatusSlot::Warn) => "WARN",
        Some(RiskStatusSlot::Block) => "BLOCK",
        None => "未知",
    }
}

pub(super) fn risk_status_label_with_problem(
    status: Option<RiskStatusSlot>,
    problem: Option<&ApiProblem>,
) -> &'static str {
    if status.is_none() && problem.is_some() {
        "错误"
    } else {
        risk_status_label(status)
    }
}

pub(super) fn risk_title(status: Option<RiskStatusSlot>) -> &'static str {
    match status {
        Some(RiskStatusSlot::Ok) => "风险状态：OK；来源 SystemHealth.risk",
        Some(RiskStatusSlot::Warn) => {
            "风险状态：WARN；来源 SystemHealth.risk；请进入持仓/风控查看约束"
        }
        Some(RiskStatusSlot::Block) => {
            "风险状态：BLOCK；来源 SystemHealth.risk；高风险动作应被阻断"
        }
        None => "风险状态未知：等待 SystemHealth 快照",
    }
}

pub(super) fn risk_title_with_problem(
    status: Option<RiskStatusSlot>,
    problem: Option<&ApiProblem>,
) -> String {
    title_with_api_problem(risk_title(status), problem)
}
