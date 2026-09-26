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
            <span class="slot-label">"风险"</span>
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
        Some(RiskStatusSlot::Ok) => "正常",
        Some(RiskStatusSlot::Warn) => "需留意",
        Some(RiskStatusSlot::Block) => "交易受限",
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
        Some(RiskStatusSlot::Ok) => "风险检查正常；不代表没有亏损风险",
        Some(RiskStatusSlot::Warn) => {
            "风险检查有提醒；请到持仓/风控查看原因"
        }
        Some(RiskStatusSlot::Block) => {
            "风控已限制交易；请到持仓/风控查看限制原因"
        }
        None => "风险状态待确认：尚未收到后台风险数据",
    }
}

pub(super) fn risk_title_with_problem(
    status: Option<RiskStatusSlot>,
    problem: Option<&ApiProblem>,
) -> String {
    title_with_api_problem(risk_title(status), problem)
}
