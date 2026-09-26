use super::*;

pub(super) fn decision_text(preview: &ExecutionPreview) -> &'static str {
    match preview.readiness {
        PreviewReadiness::Pending => "待交易检查",
        PreviewReadiness::Stale => "失效",
        PreviewReadiness::Error => "错误",
        PreviewReadiness::Ready if preview.can_submit() => "通过",
        PreviewReadiness::Ready => "阻断",
    }
}

pub(super) fn positive_net_edge_state(preview: &ExecutionPreview) -> CheckItemState {
    if let Some(state) = readiness_state(preview.readiness) {
        return state;
    }
    if preview.one_cycle_cost.is_none() {
        return CheckItemState::Missing;
    }
    if preview.net_edge_usd() > 0.0 {
        CheckItemState::Ok
    } else {
        CheckItemState::Warn
    }
}

pub(super) fn cost_edge_state(preview: &ExecutionPreview) -> CheckItemState {
    if let Some(state) = readiness_state(preview.readiness) {
        return state;
    }
    if preview.one_cycle_cost.is_none() {
        return CheckItemState::Missing;
    }
    if preview.total_cost_usd() < preview.estimated_funding_usd.abs() {
        CheckItemState::Ok
    } else {
        CheckItemState::Warn
    }
}

pub(super) fn ready_nonnegative_state(preview: &ExecutionPreview, value: f64) -> CheckItemState {
    if let Some(state) = readiness_state(preview.readiness) {
        return state;
    }
    if preview.one_cycle_cost.is_none() {
        return CheckItemState::Missing;
    }
    if value >= 0.0 {
        CheckItemState::Ok
    } else {
        CheckItemState::Warn
    }
}

pub(super) fn max_loss_state(preview: &ExecutionPreview) -> CheckItemState {
    if let Some(state) = readiness_state(preview.readiness) {
        return state;
    }
    if preview.max_loss_usd <= preview.estimated_funding_usd.abs().max(1.0) {
        CheckItemState::Ok
    } else {
        CheckItemState::Block
    }
}

pub(super) fn decision_state(preview: &ExecutionPreview) -> CheckItemState {
    if let Some(state) = readiness_state(preview.readiness) {
        return state;
    }
    if preview.can_submit() {
        CheckItemState::Ok
    } else {
        CheckItemState::Block
    }
}

pub(super) fn liq_distance_state(value: Option<f64>) -> CheckItemState {
    match value {
        Some(distance) if distance >= 15.0 => CheckItemState::Ok,
        Some(_) => CheckItemState::Block,
        None => CheckItemState::Missing,
    }
}

pub(super) fn current_liq_value(preview: &ExecutionPreview) -> String {
    if let Some(value) = preview.liquidation.current_account_pct {
        return pct(value);
    }
    if preview.execution_mode_label == "模拟" {
        return "模拟无需".into();
    }
    if positions_evidence_needs_attention(preview) {
        return "数据待确认".into();
    }
    "--".into()
}

pub(super) fn current_liq_state(preview: &ExecutionPreview) -> CheckItemState {
    if let Some(value) = preview.liquidation.current_account_pct {
        return liq_distance_state(Some(value));
    }
    if preview.execution_mode_label == "模拟" {
        return CheckItemState::Ok;
    }
    if positions_evidence_needs_attention(preview) {
        return CheckItemState::Block;
    }
    CheckItemState::Missing
}

pub(super) fn positions_evidence_needs_attention(preview: &ExecutionPreview) -> bool {
    if preview.execution_mode_label == "模拟" {
        return false;
    }
    preview
        .liquidation
        .positions_evidence
        .as_ref()
        .is_some_and(|evidence| evidence.status != ListStatus::Fresh)
}

pub(super) fn guard_state(guard: &ExecutionGuard) -> CheckItemState {
    match guard
        .preflight_outcome
        .as_ref()
        .map(|outcome| outcome.status)
    {
        Some(HedgePreflightStatus::Passed) => CheckItemState::Ok,
        Some(HedgePreflightStatus::Blocked) => CheckItemState::Block,
        Some(HedgePreflightStatus::Failed) => CheckItemState::Error,
        Some(HedgePreflightStatus::Skipped) => CheckItemState::Unknown,
        None if guard.passed => CheckItemState::Ok,
        None => CheckItemState::Block,
    }
}

pub(super) fn readiness_state(readiness: PreviewReadiness) -> Option<CheckItemState> {
    match readiness {
        PreviewReadiness::Pending => Some(CheckItemState::Missing),
        PreviewReadiness::Stale => Some(CheckItemState::Stale),
        PreviewReadiness::Error => Some(CheckItemState::Error),
        PreviewReadiness::Ready => None,
    }
}
