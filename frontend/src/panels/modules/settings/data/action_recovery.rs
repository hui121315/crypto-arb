use crate::state::action_state::{action_state_from_action_run, ActionState};
use crate::state::load_state::LoadState;
use leptos::prelude::*;
use shared_types::{ActionRun, ActionRunKind};

use super::resources::SettingsResource;

pub(in crate::panels::modules::settings) fn use_action_run_recovery(
    state: RwSignal<ActionState>,
    runs: SettingsResource<Vec<ActionRun>>,
    kinds: Vec<ActionRunKind>,
) {
    Effect::new(move |_| {
        let runs = runs.get();
        if state.get_untracked().is_pending() {
            return;
        }
        let rows = match &runs {
            LoadState::Ready(rows) | LoadState::Stale { value: rows, .. } => rows,
            LoadState::Loading | LoadState::Error(_) => return,
        };
        if let Some(run) = matching_recovery_run(&state.get_untracked(), rows, &kinds) {
            state.set(action_state_from_action_run(run));
        }
    });
}

fn matching_recovery_run<'a>(
    state: &ActionState,
    runs: &'a [ActionRun],
    kinds: &[ActionRunKind],
) -> Option<&'a ActionRun> {
    if !matches!(
        state,
        ActionState::Accepted { .. } | ActionState::Failed { .. }
    ) {
        return None;
    }
    let evidence = state.evidence()?;
    runs.iter()
        .filter(|run| kinds.contains(&run.kind))
        .filter(|run| {
            if let Some(id) = evidence
                .action_run_id
                .as_deref()
                .filter(|id| !id.is_empty())
            {
                run.id == id
            } else if let Some(key) = evidence
                .idempotency_key
                .as_deref()
                .filter(|key| !key.is_empty())
            {
                run.idempotency_key.as_deref() == Some(key)
            } else {
                evidence
                    .request_id
                    .as_deref()
                    .filter(|id| !id.is_empty())
                    .is_some_and(|id| run.request_id.as_deref() == Some(id))
            }
        })
        .max_by_key(|run| run.updated_at_ms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::{ActionEvidence, ActionRunStatus, ApiProblem};

    #[test]
    fn recovery_requires_the_current_request_not_the_latest_similar_action() {
        let run = ActionRun {
            id: "save-1".into(),
            kind: ActionRunKind::VenueCredentialsUpdate,
            status: ActionRunStatus::Succeeded,
            actor: "fixture".into(),
            target: Some("okx".into()),
            request_id: Some("req-1".into()),
            idempotency_key: Some("key-1".into()),
            message: "saved".into(),
            problem: None,
            result: None,
            mutation: None,
            started_at_ms: 1,
            updated_at_ms: 2,
        };
        let rows = vec![run];
        let kinds = [ActionRunKind::VenueCredentialsUpdate];
        assert!(matching_recovery_run(&ActionState::Idle, &rows, &kinds).is_none());
        let failed = |key: &str| {
            ActionState::failed("unknown", ApiProblem::new("TIMEOUT", "timeout"))
                .with_evidence(ActionEvidence::default().with_idempotency_key(Some(key.into())))
        };
        assert!(matching_recovery_run(&failed("key-2"), &rows, &kinds).is_none());
        assert_eq!(
            matching_recovery_run(&failed("key-1"), &rows, &kinds).map(|r| r.id.as_str()),
            Some("save-1")
        );
        let done = ActionState::succeeded("saved")
            .with_evidence(ActionEvidence::from_action_run(&rows[0]));
        assert!(matching_recovery_run(&done, &rows, &kinds).is_none());
    }
}
