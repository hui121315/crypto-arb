use crate::state::load_state::LoadState;
use leptos::prelude::*;
use shared_types::{ApiProblem, OnchainComparisonSnapshot};
use crate::panels::modules::opportunity_counts::snapshot_clock;
use super::freshness::{self, MarketClock};

#[derive(Clone, Copy)]
pub(super) struct SnapshotState {
    state: RwSignal<LoadState<OnchainComparisonSnapshot>>,
    saving: RwSignal<bool>,
    epoch: RwSignal<u64>,
    revision: RwSignal<u64>,
    page: RwSignal<()>,
    display: RwSignal<LoadState<OnchainComparisonSnapshot>>,
    timing: RwSignal<Option<MarketClock>>,
    clock: RwSignal<(i64, i64)>,
    action_started: RwSignal<(i64, i64)>,
    configuration_gate: Option<Signal<bool>>,
}

#[derive(Clone, Copy)]
pub(super) struct ReadStamp {
    epoch: u64,
    revision: u64,
    requested: (i64, i64),
}

impl SnapshotState {
    pub(super) fn new(
        state: RwSignal<LoadState<OnchainComparisonSnapshot>>,
        saving: RwSignal<bool>,
    ) -> Self {
        let snapshots = Self {
            state,
            saving,
            epoch: RwSignal::new(0),
            revision: RwSignal::new(0),
            page: RwSignal::new(()),
            display: RwSignal::new(LoadState::Loading),
            timing: RwSignal::new(None),
            clock: RwSignal::new(snapshot_clock()),
            action_started: RwSignal::new(snapshot_clock()),
            configuration_gate: None,
        };
        Effect::new(move |_| {
            let next = snapshots.projected(snapshots.clock.get());
            if snapshots.display.with_untracked(|old| old != &next) {
                snapshots.display.set(next);
            }
        });
        snapshots
    }

    pub(super) fn display_state(self) -> RwSignal<LoadState<OnchainComparisonSnapshot>> { self.display }

    pub(super) fn raw_state(self) -> RwSignal<LoadState<OnchainComparisonSnapshot>> { self.state }

    pub(super) fn current_state(self) -> LoadState<OnchainComparisonSnapshot> {
        untrack(|| self.projected(snapshot_clock()))
    }

    fn projected(self, clock: (i64, i64)) -> LoadState<OnchainComparisonSnapshot> {
        let mut state = self.state.get();
        let now = self.timing.get().map(|timing| timing.now(clock));
        let interrupted = matches!(state, LoadState::Stale { .. });
        if let LoadState::Ready(snapshot) | LoadState::Stale { value: snapshot, .. } = &mut state {
            freshness::project(snapshot, now);
            if interrupted && snapshot.config.enabled {
                snapshot.quality = shared_types::OnchainComparisonQuality::Stale;
                if snapshot.config.dex_comparison.enabled {
                    snapshot.dex_comparison.quality = shared_types::OnchainDexComparisonQuality::Stale;
                }
                if snapshot.config.cross_chain.enabled {
                    snapshot.cross_chain.quality = shared_types::OnchainCrossChainQuality::Stale;
                    snapshot.cross_chain.preview_ready = false;
                }
            }
        }
        state
    }

    pub(super) fn start_clock(self) {
        self.clock.set(snapshot_clock());
        let interval = StoredValue::new_local(Some(gloo_timers::callback::Interval::new(1_000, move || {
            self.clock.set(snapshot_clock());
        })));
        on_cleanup(move || interval.update_value(|slot| { slot.take(); }));
    }

    pub(super) fn for_page(self) -> Self {
        // Keep configuration receipts across navigation, but discard disposed page reads/builds.
        Self { page: RwSignal::new(()), ..self }
    }

    pub(super) fn with_configuration_gate(self, gate: Signal<bool>) -> Self {
        Self { configuration_gate: Some(gate), ..self }
    }

    fn configuration_blocked(self) -> bool {
        self.configuration_gate.is_some_and(|gate| gate.get_untracked())
    }

    pub(super) fn read_stamp(self) -> Option<ReadStamp> {
        (self.page.try_get_untracked().is_some()
            && !self.configuration_blocked()
            && self.saving.try_get_untracked() == Some(false)).then(|| ReadStamp {
            epoch: self.epoch.get_untracked(),
            revision: self.revision.get_untracked(),
            requested: snapshot_clock(),
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
            Ok(snapshot) => self.apply_snapshot(snapshot, false, stamp.requested),
            Err(problem) if self.revision.try_get_untracked() == Some(stamp.revision) => {
                let _ = self
                    .state
                    .try_update(|current| current.apply_result(Err(problem)));
            }
            Err(_) => {}
        }
    }

    pub(super) fn accepts_read(self, stamp: ReadStamp) -> bool {
        self.page.try_get_untracked().is_some()
            && !self.configuration_blocked()
            && self.saving.try_get_untracked() == Some(false)
            && self.epoch.try_get_untracked() == Some(stamp.epoch)
    }

    pub(super) fn config_epoch(self) -> u64 {
        if let Some(gate) = self.configuration_gate { gate.track(); }
        self.epoch.get()
    }

    pub(super) fn apply_stream(self, result: Result<OnchainComparisonSnapshot, ApiProblem>) {
        if let Some(stamp) = self.read_stamp() {
            self.apply_read(stamp, result);
        }
    }

    pub(super) fn begin_action(self) -> Option<u64> {
        let stamp = self.read_stamp()?;
        self.begin_exclusive_read(stamp.requested)
    }

    // Recovery reads current configuration while all writes/previews remain gated.
    pub(super) fn begin_recovery_read(self) -> Option<u64> {
        self.begin_exclusive_read(snapshot_clock())
    }

    fn begin_exclusive_read(self, requested: (i64, i64)) -> Option<u64> {
        if self.saving.try_get_untracked() != Some(false) { return None; }
        let epoch = self.epoch.get_untracked().wrapping_add(1);
        self.action_started.set(requested);
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
        self.apply_snapshot(snapshot, true, self.action_started.get_untracked());
    }

    pub(super) fn apply_refreshed(self, snapshot: OnchainComparisonSnapshot) {
        self.apply_snapshot(snapshot, false, self.action_started.get_untracked());
    }

    fn apply_snapshot(self, mut next: OnchainComparisonSnapshot, saved: bool, requested: (i64, i64)) {
        if self.epoch.try_get_untracked().is_none() {
            return;
        }
        let mut config_changed = false;
        let observed = next.observed_at_ms;
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
                config_changed = next.config != existing.config;
                if next.batch.observed_at_ms < existing.batch.observed_at_ms {
                    next.batch = existing.batch.clone();
                }
            }
            *current = LoadState::Ready(next);
            true
        });
        if accepted == Some(true) {
            let received = snapshot_clock();
            self.clock.set(received);
            self.timing.update(|timing| *timing = MarketClock::advance(*timing, observed, requested, received));
            if config_changed {
                self.epoch.update(|epoch| *epoch = epoch.wrapping_add(1));
            }
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
    fn remote_configuration_changes_invalidate_builds_but_normal_quotes_do_not() {
        Owner::new().with(|| {
            let state = RwSignal::new(LoadState::Ready(snapshot(100, true)));
            let gate = SnapshotState::new(state, RwSignal::new(false));
            let stamp = gate.read_stamp().unwrap();
            gate.apply_stream(Ok(snapshot(101, true)));
            assert!(gate.accepts_read(stamp));
            gate.apply_stream(Ok(snapshot(102, false)));
            gate.apply_stream(Ok(snapshot(103, true)));
            assert!(!gate.accepts_read(stamp));
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
