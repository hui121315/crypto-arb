use super::*;

pub(super) const ORDER_ELAPSED_SLOT_LABEL: &str = "订单终态";

#[component]
pub fn OrderElapsedSlot(
    elapsed_ms: Memo<Option<u32>>,
    problem: Memo<Option<ApiProblem>>,
    environment: Memo<Option<ExecutionEnvironment>>,
) -> impl IntoView {
    view! {
        <div
            data-testid="status-order-elapsed"
            class=move || {
                let problem = problem.get();
                order_elapsed_slot_class_for_environment(
                    elapsed_ms.get(),
                    problem.as_ref(),
                    environment.get(),
                )
            }
            title=move || {
                let problem = problem.get();
                order_elapsed_title_for_environment(
                    elapsed_ms.get(),
                    problem.as_ref(),
                    environment.get(),
                )
            }
        >
            <span class=move || {
                let problem = problem.get();
                order_elapsed_dot_class_for_environment(
                    elapsed_ms.get(),
                    problem.as_ref(),
                    environment.get(),
                )
            }></span>
            <span class="slot-label">{ORDER_ELAPSED_SLOT_LABEL}</span>
            <span class="num">{move || {
                let problem = problem.get();
                order_elapsed_label_for_environment(
                    elapsed_ms.get(),
                    problem.as_ref(),
                    environment.get(),
                )
            }}</span>
        </div>
    }
}

#[cfg(test)]
pub(super) fn order_elapsed_slot_class_with_problem(
    elapsed_ms: Option<u32>,
    problem: Option<&ApiProblem>,
) -> &'static str {
    order_elapsed_slot_class_for_environment(elapsed_ms, problem, Some(ExecutionEnvironment::Live))
}

pub(super) fn order_elapsed_slot_class_for_environment(
    elapsed_ms: Option<u32>,
    problem: Option<&ApiProblem>,
    environment: Option<ExecutionEnvironment>,
) -> &'static str {
    scalar_slot_class(
        problem.is_some()
            || (environment != Some(ExecutionEnvironment::Paper)
                && order_elapsed_degraded(elapsed_ms)),
    )
}

#[cfg(test)]
pub(super) fn order_elapsed_dot_class_with_problem(
    elapsed_ms: Option<u32>,
    problem: Option<&ApiProblem>,
) -> &'static str {
    order_elapsed_dot_class_for_environment(elapsed_ms, problem, Some(ExecutionEnvironment::Live))
}

pub(super) fn order_elapsed_dot_class_for_environment(
    elapsed_ms: Option<u32>,
    problem: Option<&ApiProblem>,
    environment: Option<ExecutionEnvironment>,
) -> &'static str {
    dot_class(
        problem.is_some()
            || (environment != Some(ExecutionEnvironment::Paper)
                && order_elapsed_degraded(elapsed_ms)),
    )
}

#[cfg(test)]
pub(super) fn order_elapsed_slot_class(elapsed_ms: Option<u32>) -> &'static str {
    order_elapsed_slot_class_with_problem(elapsed_ms, None)
}

#[cfg(test)]
pub(super) fn order_elapsed_dot_class(elapsed_ms: Option<u32>) -> &'static str {
    order_elapsed_dot_class_with_problem(elapsed_ms, None)
}

pub(super) fn order_elapsed_degraded(elapsed_ms: Option<u32>) -> bool {
    elapsed_ms.map(|value| value > 200).unwrap_or(true)
}

pub(super) fn order_elapsed_label(elapsed_ms: Option<u32>) -> String {
    elapsed_ms
        .map(|value| {
            if value < 1_000 {
                format!("{value}ms")
            } else {
                format!("{:.1}s", value as f64 / 1_000.0)
            }
        })
        .unwrap_or_else(|| "未知".into())
}

#[cfg(test)]
pub(super) fn order_elapsed_label_with_problem(
    elapsed_ms: Option<u32>,
    problem: Option<&ApiProblem>,
) -> String {
    order_elapsed_label_for_environment(elapsed_ms, problem, Some(ExecutionEnvironment::Live))
}

pub(super) fn order_elapsed_label_for_environment(
    elapsed_ms: Option<u32>,
    problem: Option<&ApiProblem>,
    environment: Option<ExecutionEnvironment>,
) -> String {
    if elapsed_ms.is_none() && problem.is_some() {
        "错误".into()
    } else if elapsed_ms.is_none() && environment == Some(ExecutionEnvironment::Paper) {
        "无实盘订单".into()
    } else {
        order_elapsed_label(elapsed_ms)
    }
}

pub(super) fn order_elapsed_title(elapsed_ms: Option<u32>) -> &'static str {
    if elapsed_ms.is_some() {
        "订单终态耗时：从创建到 Filled/Cancelled/Rejected/Failed 的平均耗时（OrderRecord updated_at - created_at）；包含交易所处理、重试与本地状态推进，不代表网络 RTT"
    } else {
        "订单终态耗时未知：等待 SystemHealth 快照"
    }
}

#[cfg(test)]
pub(super) fn order_elapsed_title_with_problem(
    elapsed_ms: Option<u32>,
    problem: Option<&ApiProblem>,
) -> String {
    order_elapsed_title_for_environment(elapsed_ms, problem, Some(ExecutionEnvironment::Live))
}

pub(super) fn order_elapsed_title_for_environment(
    elapsed_ms: Option<u32>,
    problem: Option<&ApiProblem>,
    environment: Option<ExecutionEnvironment>,
) -> String {
    let title = if elapsed_ms.is_none() && environment == Some(ExecutionEnvironment::Paper) {
        "模拟模式当前没有实盘订单状态样本"
    } else {
        order_elapsed_title(elapsed_ms)
    };
    title_with_api_problem(title, problem)
}
