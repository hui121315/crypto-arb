use crate::state::AppState;
use tokio::sync::watch;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WakeReason {
    Recovery,
    System,
    Opportunities,
    Automation,
    Execution,
    CloseRuns,
    ActionRuns,
}

pub(super) struct WakeSources {
    system: watch::Receiver<u64>,
    opportunities: watch::Receiver<u64>,
    automation: watch::Receiver<u64>,
    execution: watch::Receiver<u64>,
    close_runs: watch::Receiver<u64>,
    action_runs: watch::Receiver<u64>,
}

impl WakeSources {
    pub(super) fn new(state: &AppState) -> Self {
        Self {
            system: state.system_health_snapshot().subscribe_updates(),
            opportunities: state.opportunity_index().subscribe_updates(),
            automation: state
                .ws_hub()
                .subscribe_activity(realtime::channels::AUTOMATION),
            execution: state
                .ws_hub()
                .subscribe_activity(realtime::channels::EXECUTION),
            close_runs: state
                .ws_hub()
                .subscribe_activity(realtime::channels::CLOSE_RUN_ACTIVITY),
            action_runs: state
                .ws_hub()
                .subscribe_activity(realtime::channels::ACTION_RUN_ACTIVITY),
        }
    }

    pub(super) async fn changed(&mut self) -> Option<WakeReason> {
        tokio::select! {
            result = self.system.changed() => result.ok().map(|()| WakeReason::System),
            result = self.opportunities.changed() => result.ok().map(|()| WakeReason::Opportunities),
            result = self.automation.changed() => result.ok().map(|()| WakeReason::Automation),
            result = self.execution.changed() => result.ok().map(|()| WakeReason::Execution),
            result = self.close_runs.changed() => result.ok().map(|()| WakeReason::CloseRuns),
            result = self.action_runs.changed() => result.ok().map(|()| WakeReason::ActionRuns),
        }
    }
}
