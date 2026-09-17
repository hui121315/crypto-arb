use super::*;

pub(super) fn capability_hint(preview: &ExecutionPreview) -> String {
    let blocker_count = preview
        .order_plans
        .iter()
        .map(|plan| plan.blockers.len())
        .sum::<usize>();
    if preview.order_plans.is_empty() {
        "等待后端预览".into()
    } else if blocker_count == 0 {
        format!("能力矩阵 {}", capability_matrix(&preview.order_plans))
    } else {
        format!(
            "能力阻断 {blocker_count} 条 · {}",
            capability_matrix(&preview.order_plans)
        )
    }
}

fn capability_matrix(plans: &[OrderCompilePlan]) -> String {
    plans
        .iter()
        .map(plan_capability_label)
        .collect::<Vec<_>>()
        .join(" / ")
}

fn plan_capability_label(plan: &OrderCompilePlan) -> String {
    let capability = &plan.venue_capability;
    let order_types = order_type_labels(capability_or_legacy(
        &capability.available_order_types,
        &plan.available_order_types,
    ));
    let time_in_force = time_in_force_labels(capability_or_legacy(
        &capability.available_time_in_force,
        &plan.available_time_in_force,
    ));
    let margin_modes = margin_mode_labels(capability_or_legacy(
        &capability.available_margin_modes,
        &plan.available_margin_modes,
    ));
    let account_mode_note = account_mode_note(plan);
    format!(
        "{} {}: {} · {} · {}{}",
        venue_family(&plan.exchange),
        plan.symbol,
        order_types,
        time_in_force,
        margin_modes,
        account_mode_note,
    )
}

fn capability_or_legacy<'a, T>(capability: &'a [T], legacy: &'a [T]) -> &'a [T] {
    if capability.is_empty() {
        legacy
    } else {
        capability
    }
}

fn margin_mode_labels(values: &[MarginMode]) -> String {
    if values.is_empty() {
        "保证金模式不进下单载荷".into()
    } else {
        values
            .iter()
            .map(|mode| margin_mode_label(*mode))
            .collect::<Vec<_>>()
            .join("/")
    }
}

fn order_type_labels(values: &[OrderType]) -> String {
    if values.is_empty() {
        "订单类型未返回".into()
    } else {
        values
            .iter()
            .map(|kind| order_type_label(*kind))
            .collect::<Vec<_>>()
            .join("/")
    }
}

fn time_in_force_labels(values: &[TimeInForce]) -> String {
    if values.is_empty() {
        "TIF 未返回".into()
    } else {
        values
            .iter()
            .map(|tif| time_in_force_label(*tif))
            .collect::<Vec<_>>()
            .join("/")
    }
}

fn account_mode_note(plan: &OrderCompilePlan) -> String {
    match &plan.venue_capability.account_mode {
        Some(mode) => format!(" · {}", mode.mode),
        None => plan
            .venue_capability
            .account_mode_error
            .as_ref()
            .map(|_| " · 账户模式未确认".to_owned())
            .unwrap_or_default(),
    }
}
