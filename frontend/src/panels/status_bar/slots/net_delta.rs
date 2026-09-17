use super::*;

#[component]
pub fn NetDeltaSlot(
    delta: Memo<Option<(f64, f64)>>,
    problem: Memo<Option<ApiProblem>>,
) -> impl IntoView {
    view! {
        <div
            class=move || {
                let problem = problem.get();
                net_delta_slot_class_with_problem(delta.get(), problem.as_ref())
            }
            title=move || {
                let problem = problem.get();
                net_delta_title_with_problem(delta.get(), problem.as_ref())
            }
        >
            <span class=move || {
                let problem = problem.get();
                net_delta_dot_class_with_problem(delta.get(), problem.as_ref())
            }></span>
            <span class="slot-label">"Delta"</span>
            <span class="num">{move || {
                let problem = problem.get();
                net_delta_label_with_problem(delta.get(), problem.as_ref())
            }}</span>
        </div>
    }
}

pub(super) fn net_delta_slot_class_with_problem(
    delta: Option<(f64, f64)>,
    problem: Option<&ApiProblem>,
) -> &'static str {
    scalar_slot_class(problem.is_some() || net_delta_degraded(delta))
}

pub(super) fn net_delta_dot_class_with_problem(
    delta: Option<(f64, f64)>,
    problem: Option<&ApiProblem>,
) -> &'static str {
    dot_class(problem.is_some() || net_delta_degraded(delta))
}

#[cfg(test)]
pub(super) fn net_delta_slot_class(delta: Option<(f64, f64)>) -> &'static str {
    net_delta_slot_class_with_problem(delta, None)
}

#[cfg(test)]
pub(super) fn net_delta_dot_class(delta: Option<(f64, f64)>) -> &'static str {
    net_delta_dot_class_with_problem(delta, None)
}

pub(super) fn net_delta_degraded(delta: Option<(f64, f64)>) -> bool {
    delta.map(|(_, pct)| pct.abs() > 5.0).unwrap_or(true)
}

pub(super) fn net_delta_label(delta: Option<(f64, f64)>) -> String {
    delta
        .map(|(value, pct)| {
            let value = display_zero(value);
            let pct = display_zero(pct);
            format!("${value:.0} ({pct:+.1}%)")
        })
        .unwrap_or_else(|| "未知".into())
}

pub(super) fn net_delta_label_with_problem(
    delta: Option<(f64, f64)>,
    problem: Option<&ApiProblem>,
) -> String {
    if delta.is_none() && problem.is_some() {
        "错误".into()
    } else {
        net_delta_label(delta)
    }
}

pub(super) fn net_delta_title(delta: Option<(f64, f64)>) -> String {
    delta
        .map(|(value, pct)| {
            let value = display_zero(value);
            let pct = display_zero(pct);
            format!(
                "净 Delta：${value:.0}，占 NAV {pct:+.1}%；阈值 ±5%；来源 SystemHealth.netDeltaUsd/netDeltaPctOfNav"
            )
        })
        .unwrap_or_else(|| "净 Delta 未知：等待 SystemHealth 快照".into())
}

pub(super) fn net_delta_title_with_problem(
    delta: Option<(f64, f64)>,
    problem: Option<&ApiProblem>,
) -> String {
    title_with_api_problem(net_delta_title(delta), problem)
}

fn display_zero(value: f64) -> f64 {
    if value.abs() < 0.05 {
        0.0
    } else {
        value
    }
}
