use crate::state::load_state::LoadState;
use leptos::prelude::*;
use shared_types::{ApiProblem, OnchainComparisonSnapshot};

#[derive(Clone, Copy)]
pub(super) struct SnapshotState {
    state: RwSignal<LoadState<OnchainComparisonSnapshot>>,
    saving: RwSignal<bool>,
    epoch: RwSignal<u64>,
    revision: RwSignal<u64>,
}

#[derive(Clone, Copy)]
pub(super) struct ReadStamp {
    epoch: u64,
    revision: u64,
}

impl SnapshotState {
    pub(super) fn new(
        state: RwSignal<LoadState<OnchainComparisonSnapshot>>,
        saving: RwSignal<bool>,
    ) -> Self {
        Self {
            state,
            saving,
            epoch: RwSignal::new(0),
            revision: RwSignal::new(0),
        }
    }

    pub(super) fn read_stamp(self) -> Option<ReadStamp> {
        (self.saving.try_get_untracked() == Some(false)).then(|| ReadStamp {
            epoch: self.epoch.get_untracked(),
            revision: self.revision.get_untracked(),
        })
    }

    pub(super) fn apply_read(
        self,
        stamp: ReadStamp,
        result: Result<OnchainComparisonSnapshot, ApiProblem>,
    ) {
        if !self.accepts_read(stamp) {
            return;
        }
        match result {
            Ok(snapshot) => self.apply_snapshot(snapshot, false),
            Err(problem) if self.revision.try_get_untracked() == Some(stamp.revision) => {
                let _ = self
                    .state
                    .try_update(|current| current.apply_result(Err(problem)));
            }
            Err(_) => {}
        }
    }

    pub(super) fn accepts_read(self, stamp: ReadStamp) -> bool {
        self.saving.try_get_untracked() == Some(false)
            && self.epoch.try_get_untracked() == Some(stamp.epoch)
    }

    pub(super) fn apply_stream(self, result: Result<OnchainComparisonSnapshot, ApiProblem>) {
        if let Some(stamp) = self.read_stamp() {
            self.apply_read(stamp, result);
        }
    }

    pub(super) fn begin_action(self) -> Option<u64> {
        let stamp = self.read_stamp()?;
        let epoch = stamp.epoch.wrapping_add(1);
        self.epoch.set(epoch);
        self.saving.set(true);
        Some(epoch)
    }

    pub(super) fn finish_action(self, epoch: u64) -> bool {
        if self.epoch.try_get_untracked() != Some(epoch) {
            return false;
        }
        self.saving.set(false);
        true
    }

    pub(super) fn apply_saved(self, snapshot: OnchainComparisonSnapshot) {
        self.apply_snapshot(snapshot, true);
    }

    fn apply_snapshot(self, mut next: OnchainComparisonSnapshot, saved: bool) {
        if self.epoch.try_get_untracked().is_none() {
            return;
        }
        let accepted = self.state.try_update(|current| {
            // Equal-time frames may refresh evidence, but cannot roll back a saved configuration.
            if !saved
                && current.value().is_some_and(|existing| {
                    next.observed_at_ms < existing.observed_at_ms
                        || (next.observed_at_ms == existing.observed_at_ms
                            && next.config != existing.config)
                })
            {
                return false;
            }
            if let Some(existing) = current.value() {
                if next.batch.observed_at_ms < existing.batch.observed_at_ms {
                    next.batch = existing.batch.clone();
                }
            }
            *current = LoadState::Ready(next);
            true
        });
        if accepted == Some(true) {
            self.revision
                .update(|revision| *revision = revision.wrapping_add(1));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(at: i64, enabled: bool) -> OnchainComparisonSnapshot {
        let mut value = OnchainComparisonSnapshot {
            observed_at_ms: at,
            ..Default::default()
        };
        value.config.enabled = enabled;
        value
    }

    #[test]
    fn save_serializes_actions_and_invalidates_reads_before_and_during_save() {
        Owner::new().with(|| {
            let state = RwSignal::new(LoadState::Ready(snapshot(100, true)));
            let gate = SnapshotState::new(state, RwSignal::new(false));
            let read = gate.read_stamp().unwrap();
            let action = gate.begin_action().unwrap();
            assert!(gate.begin_action().is_none());
            gate.apply_stream(Ok(snapshot(110, true)));
            assert_eq!(state.get_untracked().value().unwrap().observed_at_ms, 100);
            assert!(gate.finish_action(action));
            gate.apply_saved(snapshot(120, false));
            gate.apply_read(read, Ok(snapshot(130, true)));
            gate.apply_stream(Ok(snapshot(119, true)));
            gate.apply_stream(Ok(snapshot(120, true)));
            assert!(!state.get_untracked().value().unwrap().config.enabled);
            gate.apply_stream(Ok(snapshot(121, false)));
            assert_eq!(state.get_untracked().value().unwrap().observed_at_ms, 121);
        });
    }

    #[test]
    fn late_failure_does_not_degrade_newer_stream_and_current_failure_keeps_old_values() {
        Owner::new().with(|| {
            let state = RwSignal::new(LoadState::Loading);
            let gate = SnapshotState::new(state, RwSignal::new(false));
            let read = gate.read_stamp().unwrap();
            gate.apply_stream(Ok(snapshot(200, true)));
            gate.apply_read(read, Err(ApiProblem::new("TIMEOUT", "old read")));
            assert!(matches!(state.get_untracked(), LoadState::Ready(_)));
            gate.apply_stream(Err(ApiProblem::new("DISCONNECTED", "stream failed")));
            assert!(matches!(state.get_untracked(), LoadState::Stale { .. }));
            gate.apply_stream(Ok(snapshot(200, true)));
            assert!(matches!(state.get_untracked(), LoadState::Ready(_)));
        });
    }

    #[test]
    fn route_disposal_ignores_late_callbacks_even_with_persistent_snapshot() {
        Owner::new().with(|| {
            let state = RwSignal::new(LoadState::Ready(snapshot(100, true)));
            let page = Owner::new();
            let gate = page.with(|| SnapshotState::new(state, RwSignal::new(false)));
            let read = gate.read_stamp().unwrap();
            let action = gate.begin_action().unwrap();
            drop(page);
            assert!(!gate.finish_action(action));
            gate.apply_read(read, Ok(snapshot(200, false)));
            gate.apply_stream(Ok(snapshot(200, false)));
            assert!(state.get_untracked().value().unwrap().config.enabled);
        });
    }

    #[test]
    fn a_new_quote_does_not_rollback_a_newer_batch_receipt() {
        Owner::new().with(|| {
            let mut previous = snapshot(100, true);
            previous.batch.observed_at_ms = 300;
            let state = RwSignal::new(LoadState::Ready(previous));
            let gate = SnapshotState::new(state, RwSignal::new(false));
            gate.apply_stream(Ok(snapshot(200, true)));
            let value = state.get_untracked();
            assert_eq!(value.value().unwrap().observed_at_ms, 200);
            assert_eq!(value.value().unwrap().batch.observed_at_ms, 300);
        });
    }
}
