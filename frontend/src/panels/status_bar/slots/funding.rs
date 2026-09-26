use super::*;

#[component]
pub fn NextFundingSlot(
    next: Memo<Option<FundingSlot>>,
    problem: Memo<Option<ApiProblem>>,
    environment: Memo<Option<ExecutionEnvironment>>,
) -> impl IntoView {
    view! {
        <div class=move || {
            let next = next.get();
            let problem = problem.get();
            funding_slot_class_for_environment(
                next.as_ref(),
                problem.as_ref(),
                environment.get(),
            )
        } title=move || {
            let next = next.get();
            let problem = problem.get();
            funding_title_for_environment(
                next.as_ref(),
                problem.as_ref(),
                environment.get(),
            )
        }>
            <span class=move || {
                let next = next.get();
                let problem = problem.get();
                funding_dot_class_for_environment(
                    next.as_ref(),
                    problem.as_ref(),
                    environment.get(),
                )
            }></span>
            <span class="slot-label">"资金费"</span>
            <span class="num">{move || {
                let next = next.get();
                let problem = problem.get();
                funding_label_for_environment(
                    next.as_ref(),
                    problem.as_ref(),
                    environment.get(),
                )
            }}</span>
        </div>
    }
}

#[cfg(test)]
pub(super) fn funding_slot_class_with_problem(
    next: Option<&FundingSlot>,
    problem: Option<&ApiProblem>,
) -> &'static str {
    funding_slot_class_for_environment(next, problem, Some(ExecutionEnvironment::Live))
}

pub(super) fn funding_slot_class_for_environment(
    next: Option<&FundingSlot>,
    problem: Option<&ApiProblem>,
    environment: Option<ExecutionEnvironment>,
) -> &'static str {
    scalar_slot_class(
        problem.is_some()
            || (environment != Some(ExecutionEnvironment::Paper) && funding_degraded(next)),
    )
}

#[cfg(test)]
pub(super) fn funding_dot_class_with_problem(
    next: Option<&FundingSlot>,
    problem: Option<&ApiProblem>,
) -> &'static str {
    funding_dot_class_for_environment(next, problem, Some(ExecutionEnvironment::Live))
}

pub(super) fn funding_dot_class_for_environment(
    next: Option<&FundingSlot>,
    problem: Option<&ApiProblem>,
    environment: Option<ExecutionEnvironment>,
) -> &'static str {
    dot_class(
        problem.is_some()
            || (environment != Some(ExecutionEnvironment::Paper) && funding_degraded(next)),
    )
}

#[cfg(test)]
pub(super) fn funding_slot_class(next: Option<&FundingSlot>) -> &'static str {
    funding_slot_class_with_problem(next, None)
}

#[cfg(test)]
pub(super) fn funding_dot_class(next: Option<&FundingSlot>) -> &'static str {
    funding_dot_class_with_problem(next, None)
}

pub(super) fn funding_degraded(next: Option<&FundingSlot>) -> bool {
    next.map(|slot| slot.minutes_to_settle < 5).unwrap_or(true)
}

pub(super) fn funding_label(next: Option<&FundingSlot>) -> String {
    next.map(|slot| format!("{}m", slot.minutes_to_settle))
        .unwrap_or_else(|| "未知".into())
}

#[cfg(test)]
pub(super) fn funding_label_with_problem(
    next: Option<&FundingSlot>,
    problem: Option<&ApiProblem>,
) -> String {
    funding_label_for_environment(next, problem, Some(ExecutionEnvironment::Live))
}

pub(super) fn funding_label_for_environment(
    next: Option<&FundingSlot>,
    problem: Option<&ApiProblem>,
    environment: Option<ExecutionEnvironment>,
) -> String {
    if next.is_none() && problem.is_some() {
        "错误".into()
    } else if next.is_none() && environment == Some(ExecutionEnvironment::Paper) {
        "无模拟持仓".into()
    } else {
        funding_label(next)
    }
}

pub(super) fn funding_title(next: Option<&FundingSlot>) -> String {
    next.map(|slot| {
        format!(
            "{} @ {} 下一次 资金费 {}m；预估流出 ${:.0}；<5m 标红；来源 SystemHealth.nextFunding",
            slot.symbol, slot.venue, slot.minutes_to_settle, slot.estimated_outflow_usd
        )
    })
    .unwrap_or_else(|| "资金费 未知：等待 SystemHealth.nextFunding".into())
}

#[cfg(test)]
pub(super) fn funding_title_with_problem(
    next: Option<&FundingSlot>,
    problem: Option<&ApiProblem>,
) -> String {
    funding_title_for_environment(next, problem, Some(ExecutionEnvironment::Live))
}

pub(super) fn funding_title_for_environment(
    next: Option<&FundingSlot>,
    problem: Option<&ApiProblem>,
    environment: Option<ExecutionEnvironment>,
) -> String {
    let title = match (next, environment) {
        (Some(slot), Some(ExecutionEnvironment::Paper)) => format!(
            "{}；模拟账本只展示市场结算窗口和预估资金费，不产生真实账户扣款",
            funding_title(Some(slot))
        ),
        (None, Some(ExecutionEnvironment::Paper)) => {
            "当前没有模拟持仓；开仓后显示对应市场的下一 资金费 结算窗口，模拟账本不产生真实账户扣款"
                .to_owned()
        }
        _ => funding_title(next),
    };
    title_with_api_problem(title, problem)
}
