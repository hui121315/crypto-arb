use super::tasks::{BackgroundTasks, ShutdownToken};
use crate::state::AppState;
use shared_types::{ActionRunKind, ActionRunStatus, AutomationDecisionKind, WebhookEventKind};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::MissedTickBehavior;

mod execution;
mod opportunities;
mod system;
mod wake;

use execution::{baseline_execution_updates, emit_execution_results, ExecutionAlertCursor};
use opportunities::emit_opportunity;
use system::{emit_system_event, SystemAlertFingerprint};
use wake::{WakeReason, WakeSources};

const RECOVERY_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);

#[derive(Default)]
pub(super) struct Cursor {
    opportunity_artifact_id: Option<String>,
    opportunity_monitor_keys: HashMap<String, i64>,
    opportunity_monitor_initialized: bool,
    opportunity_monitor_arm_after_ms: i64,
    opportunity_monitor_force_arm_after_ms: i64,
    automation_decision_id: Option<String>,
    execution_updates: HashMap<String, ExecutionAlertCursor>,
    compensation_updates: HashMap<String, i64>,
    system_fingerprint: Option<SystemAlertFingerprint>,
    system_notify_after_ms: i64,
    system_notified_risk: Option<shared_types::RiskStatusSlot>,
    system_notified_degraded: bool,
}

impl Cursor {
    pub(super) fn baseline(state: &AppState) -> Self {
        let automation = state.automation().snapshot();
        let opportunity_artifact_id = automation
            .recent_decisions
            .iter()
            .find_map(|decision| decision.execution_artifact.as_ref())
            .map(|artifact| artifact.artifact_id.clone());
        let artifact_opportunity_id = automation
            .recent_decisions
            .iter()
            .find_map(|decision| decision.execution_artifact.as_ref())
            .map(|artifact| artifact.opportunity_id.as_str());
        let opportunity_monitor_keys =
            opportunities::current_monitor_event_keys(state, artifact_opportunity_id);
        let now_ms = common::time::now_ms();
        let automation_decision_id = automation
            .last_decision
            .as_ref()
            .map(|decision| decision.id.clone());
        let execution_updates = baseline_execution_updates(state);
        let compensation_updates = state
            .action_runs()
            .iter()
            .filter(|run| run.kind == ActionRunKind::PortfolioCloseCompensation)
            .map(|run| (run.id.clone(), run.updated_at_ms))
            .collect();
        let system = state.system_health_snapshot().value_now();
        let system_fingerprint = system.as_ref().map(system::fingerprint);
        let system_notify_after_ms = system.as_ref().map_or(0, system::initial_notify_after_ms);
        let system_notified_risk = system.as_ref().map(|health| health.risk);
        let system_notified_degraded = system.as_ref().is_some_and(|health| health.degraded);
        Self {
            opportunity_artifact_id,
            opportunity_monitor_keys,
            opportunity_monitor_initialized: false,
            opportunity_monitor_arm_after_ms: 0,
            opportunity_monitor_force_arm_after_ms: now_ms.saturating_add(180_000),
            automation_decision_id,
            execution_updates,
            compensation_updates,
            system_fingerprint,
            system_notify_after_ms,
            system_notified_risk,
            system_notified_degraded,
        }
    }
}

pub(super) fn spawn_worker(state: &AppState, tasks: &mut BackgroundTasks) {
    let shutdown = tasks.shutdown_token();
    let cursor = Arc::new(Mutex::new(Cursor::baseline(state)));
    let state = state.clone();
    tasks.supervise(
        "webhook-event-bridge",
        RECOVERY_INTERVAL.as_millis() as i64,
        move || {
            let state = state.clone();
            let shutdown = shutdown.clone();
            let cursor = Arc::clone(&cursor);
            async move { run_worker(state, shutdown, cursor).await }
        },
    );
}

async fn run_worker(state: AppState, shutdown: ShutdownToken, cursor: Arc<Mutex<Cursor>>) {
    let registry = state.task_registry().clone();
    let mut sources = WakeSources::new(&state);
    record_bridge(&state, &cursor, &registry, WakeReason::Recovery).await;
    let mut recovery = tokio::time::interval(RECOVERY_INTERVAL);
    recovery.set_missed_tick_behavior(MissedTickBehavior::Skip);
    recovery.reset();
    loop {
        let wake = tokio::select! {
            biased;
            () = shutdown.cancelled() => break,
            reason = sources.changed() => {
                let Some(reason) = reason else {
                    break;
                };
                reason
            }
            _ = recovery.tick() => WakeReason::Recovery,
        };
        record_bridge(&state, &cursor, &registry, wake).await;
    }
}

async fn record_bridge(
    state: &AppState,
    cursor: &Mutex<Cursor>,
    registry: &crate::task_registry::TaskRegistry,
    wake: WakeReason,
) {
    let started_at_ms = common::time::now_ms();
    let mut cursor = cursor.lock().await;
    bridge_for(state, &mut cursor, wake).await;
    registry.record_result_timed("webhook-event-bridge", started_at_ms, Ok::<(), String>(()));
}

pub(super) async fn bridge_once(state: &AppState, cursor: &mut Cursor) {
    emit_opportunity(state, cursor).await;
    emit_automation(state, cursor).await;
    emit_execution_results(state, &mut cursor.execution_updates).await;
    emit_compensations(state, cursor).await;
    emit_system_event(state, cursor).await;
}

async fn bridge_for(state: &AppState, cursor: &mut Cursor, wake: WakeReason) {
    match wake {
        WakeReason::Recovery => bridge_once(state, cursor).await,
        WakeReason::System => emit_system_event(state, cursor).await,
        WakeReason::Opportunities => emit_opportunity(state, cursor).await,
        WakeReason::Automation => {
            emit_opportunity(state, cursor).await;
            emit_automation(state, cursor).await;
        }
        WakeReason::Execution | WakeReason::CloseRuns => {
            emit_execution_results(state, &mut cursor.execution_updates).await;
        }
        WakeReason::ActionRuns => emit_compensations(state, cursor).await,
    }
}

async fn emit_automation(state: &AppState, cursor: &mut Cursor) {
    let status = state.automation().snapshot();
    let Some(decision) = status.last_decision.as_ref() else {
        return;
    };
    if cursor.automation_decision_id.as_deref() == Some(decision.id.as_str()) {
        return;
    }
    if !alert_worthy_automation_decision(decision.kind) {
        cursor.automation_decision_id = Some(decision.id.clone());
        return;
    }
    if emit_value(
        state,
        WebhookEventKind::AutomationDecision,
        format!("automation-{}", decision.id),
        decision,
    )
    .await
    {
        cursor.automation_decision_id = Some(decision.id.clone());
    }
}

const fn alert_worthy_automation_decision(kind: AutomationDecisionKind) -> bool {
    matches!(
        kind,
        AutomationDecisionKind::Submitted
            | AutomationDecisionKind::Replayed
            | AutomationDecisionKind::Paused
            | AutomationDecisionKind::EmergencyStopped
            | AutomationDecisionKind::Failed
    )
}

async fn emit_compensations(state: &AppState, cursor: &mut Cursor) {
    for run in state.action_runs().iter() {
        if run.kind != ActionRunKind::PortfolioCloseCompensation
            || run.status == ActionRunStatus::Accepted
            || cursor.compensation_updates.get(&run.id) == Some(&run.updated_at_ms)
        {
            continue;
        }
        if emit_value(
            state,
            WebhookEventKind::Compensation,
            format!("compensation-{}-{}", run.id, run.updated_at_ms),
            run.value(),
        )
        .await
        {
            cursor
                .compensation_updates
                .insert(run.id.clone(), run.updated_at_ms);
        }
    }
    trim_cursor(&mut cursor.compensation_updates);
}

async fn emit_value<T: serde::Serialize + Sync>(
    state: &AppState,
    kind: WebhookEventKind,
    event_id: String,
    value: &T,
) -> bool {
    let Ok(payload) = serde_json::to_value(value) else {
        return false;
    };
    match crate::services::webhook::emit_idempotent(state, kind, event_id, payload).await {
        Ok(_) => true,
        Err(error) => {
            tracing::warn!(%error, ?kind, "webhook event enqueue failed");
            false
        }
    }
}

fn trim_cursor(values: &mut HashMap<String, i64>) {
    if values.len() <= 256 {
        return;
    }
    let mut rows = values
        .iter()
        .map(|(id, at)| (id.clone(), *at))
        .collect::<Vec<_>>();
    rows.sort_by_key(|(_, at)| *at);
    for (id, _) in rows.into_iter().take(values.len().saturating_sub(256)) {
        values.remove(&id);
    }
}

#[cfg(test)]
#[path = "webhook_events/tests.rs"]
mod tests;
