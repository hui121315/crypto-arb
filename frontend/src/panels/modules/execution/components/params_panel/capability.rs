//! 执行参数面板的纯能力派生：双腿能力交集 → 禁用项、首个可用项、文案提示。
//! 组件见父模块 `params_panel.rs`。

use super::super::super::data::ExecutionPreview;
use super::super::super::problem::execution_problem_text;
use super::{MARGIN_MODE_OPTIONS, ORDER_TYPE_OPTIONS, TIME_IN_FORCE_OPTIONS};
use crate::state::load_state::LoadState;
use shared_types::{venue_family, MarginMode, OrderCompilePlan, OrderType, TimeInForce};

#[path = "capability/labels.rs"]
mod labels;
use labels::capability_hint;

pub(super) fn disabled_time_in_force_for_state(state: &LoadState<ExecutionPreview>) -> Vec<String> {
    ready_preview(state)
        .map(disabled_time_in_force)
        .unwrap_or_default()
}

pub(super) fn disabled_order_types_for_state(state: &LoadState<ExecutionPreview>) -> Vec<String> {
    ready_preview(state)
        .map(disabled_order_types)
        .unwrap_or_default()
}

pub(super) fn disabled_margin_modes_for_state(state: &LoadState<ExecutionPreview>) -> Vec<String> {
    ready_preview(state)
        .map(disabled_margin_modes)
        .unwrap_or_default()
}

pub(super) fn capability_hint_for_state(state: &LoadState<ExecutionPreview>) -> String {
    match state {
        LoadState::Ready(preview) if preview.is_ready() => capability_hint(preview),
        LoadState::Ready(_) | LoadState::Loading => "等待后端预览".into(),
        LoadState::Stale { problem, .. } => execution_problem_text("预览已失效", problem),
        LoadState::Error(problem) => execution_problem_text("预览失败", problem),
    }
}

fn ready_preview(state: &LoadState<ExecutionPreview>) -> Option<&ExecutionPreview> {
    match state {
        LoadState::Ready(preview) if preview.is_ready() => Some(preview),
        LoadState::Ready(_)
        | LoadState::Stale { .. }
        | LoadState::Error(_)
        | LoadState::Loading => None,
    }
}

pub(super) fn disabled_time_in_force(preview: &ExecutionPreview) -> Vec<String> {
    common_time_in_force_labels(&preview.order_plans)
        .map(|allowed| disabled_by_allowed(TIME_IN_FORCE_OPTIONS, &allowed))
        .unwrap_or_default()
}

pub(super) fn disabled_order_types(preview: &ExecutionPreview) -> Vec<String> {
    common_order_type_labels(&preview.order_plans)
        .map(|allowed| disabled_by_allowed(ORDER_TYPE_OPTIONS, &allowed))
        .unwrap_or_default()
}

pub(super) fn disabled_margin_modes(preview: &ExecutionPreview) -> Vec<String> {
    common_margin_mode_labels(&preview.order_plans)
        .map(|allowed| disabled_by_allowed(MARGIN_MODE_OPTIONS, &allowed))
        .unwrap_or_default()
}

/// 只有 write path 会消费保证金模式的腿参与交集；两腿都不消费时不做约束。
fn common_margin_mode_labels(plans: &[OrderCompilePlan]) -> Option<Vec<&'static str>> {
    let constrained: Vec<&OrderCompilePlan> = plans
        .iter()
        .filter(|plan| !plan.available_margin_modes.is_empty())
        .collect();
    (!constrained.is_empty()).then(|| {
        MARGIN_MODE_OPTIONS
            .iter()
            .copied()
            .filter(|label| {
                constrained
                    .iter()
                    .all(|plan| margin_mode_allowed(plan, label))
            })
            .collect()
    })
}

fn margin_mode_allowed(plan: &OrderCompilePlan, label: &str) -> bool {
    plan.available_margin_modes
        .iter()
        .any(|mode| margin_mode_label(*mode) == label)
}

fn margin_mode_label(mode: MarginMode) -> &'static str {
    match mode {
        MarginMode::Cross => "Cross",
        MarginMode::Isolated => "Isolated",
    }
}

fn common_order_type_labels(plans: &[OrderCompilePlan]) -> Option<Vec<&'static str>> {
    (!plans.is_empty()).then(|| {
        ORDER_TYPE_OPTIONS
            .iter()
            .copied()
            .filter(|label| plans.iter().all(|plan| order_type_allowed(plan, label)))
            .collect()
    })
}

fn common_time_in_force_labels(plans: &[OrderCompilePlan]) -> Option<Vec<&'static str>> {
    (!plans.is_empty()).then(|| {
        TIME_IN_FORCE_OPTIONS
            .iter()
            .copied()
            .filter(|label| plans.iter().all(|plan| time_in_force_allowed(plan, label)))
            .collect()
    })
}

fn order_type_allowed(plan: &OrderCompilePlan, label: &str) -> bool {
    plan.available_order_types
        .iter()
        .any(|kind| order_type_label(*kind) == label)
}

fn time_in_force_allowed(plan: &OrderCompilePlan, label: &str) -> bool {
    plan.available_time_in_force
        .iter()
        .any(|tif| time_in_force_label(*tif) == label)
}

fn disabled_by_allowed(options: &[&'static str], allowed: &[&'static str]) -> Vec<String> {
    options
        .iter()
        .filter(|option| !allowed.contains(option))
        .map(|option| (*option).to_owned())
        .collect()
}

pub(super) fn first_enabled_option(
    options: &[&'static str],
    disabled_options: &[String],
) -> String {
    options
        .iter()
        .find(|option| !option_disabled(option, disabled_options))
        .copied()
        .unwrap_or_else(|| options.first().copied().unwrap_or_default())
        .to_owned()
}

fn order_type_label(kind: OrderType) -> &'static str {
    match kind {
        OrderType::Limit => "Limit",
        OrderType::Market => "Market",
        OrderType::PostOnly => "Post-only",
    }
}

fn time_in_force_label(tif: TimeInForce) -> &'static str {
    match tif {
        TimeInForce::Ioc => "IOC",
        TimeInForce::Fok => "FOK",
        TimeInForce::Gtc => "GTC",
        TimeInForce::Gtx => "GTX",
    }
}

pub(super) fn option_disabled(option: &str, disabled_options: &[String]) -> bool {
    disabled_options.iter().any(|disabled| disabled == option)
}

#[cfg(test)]
#[path = "capability/tests.rs"]
mod tests;
