use super::*;

pub(in crate::services::automated_arbitrage::worker) fn record_if_changed(
    state: &AppState,
    runtime_state: AutomationRuntimeState,
    reason: &'static str,
    active_run_count: usize,
    now_ms: i64,
) {
    let current = state.automation().snapshot();
    if current.state == runtime_state
        && current.active_run_count == active_run_count
        && current
            .last_decision
            .as_ref()
            .is_some_and(|decision| decision.reason == reason)
    {
        return;
    }
    let kind = if runtime_state == AutomationRuntimeState::Paused {
        AutomationDecisionKind::Paused
    } else {
        AutomationDecisionKind::NoEligibleCandidate
    };
    record(
        state,
        decision(kind, None, reason.to_owned(), (None, None), now_ms),
        runtime_state,
        active_run_count,
        current.cooldown_until_ms,
    );
}

pub(in crate::services::automated_arbitrage::worker) fn sync_runtime_if_changed(
    state: &AppState,
    runtime_state: AutomationRuntimeState,
    active_run_count: usize,
    now_ms: i64,
) {
    let Some(status) =
        state
            .automation()
            .sync_runtime_state(runtime_state, active_run_count, now_ms)
    else {
        return;
    };
    if let Err(error) = crate::services::ws_publish::publish_automation_status(state, &status) {
        tracing::warn!(%error, "automation status websocket publish failed");
    }
}
