use super::snapshot_state::{ReadStamp, SnapshotState};
use leptos::prelude::*;
use shared_types::{ApiProblem, ApiRecoveryAction, OnchainComparisonDirection};

#[derive(Clone, Copy)]
pub(in crate::panels::modules::onchain) struct PreviewContext {
    snapshots: SnapshotState,
    selection: RwSignal<(OnchainComparisonDirection, u64)>,
}

#[derive(Clone, Copy)]
pub(super) struct PreviewStamp {
    snapshot: ReadStamp,
    selection: (OnchainComparisonDirection, u64),
}

impl PreviewContext {
    pub(super) fn new(snapshots: SnapshotState) -> Self {
        Self {
            snapshots,
            selection: RwSignal::new((OnchainComparisonDirection::BuyOnchainSellCex, 0)),
        }
    }

    pub(in crate::panels::modules::onchain) fn get(self) -> OnchainComparisonDirection {
        self.selection.get().0
    }

    pub(in crate::panels::modules::onchain) fn get_untracked(self) -> OnchainComparisonDirection {
        self.selection.get_untracked().0
    }

    pub(in crate::panels::modules::onchain) fn set(self, direction: OnchainComparisonDirection) {
        let current = self.selection.get_untracked();
        if direction != current.0 {
            // Increment at selection time so switching away and back cannot revive a late preview.
            self.selection.set((direction, current.1.wrapping_add(1)));
        }
    }

    pub(super) fn track_changes(self) {
        let _ = (self.snapshots.config_epoch(), self.selection.get());
    }

    pub(super) fn stamp(self, direction: OnchainComparisonDirection) -> Option<PreviewStamp> {
        let selection = self.selection.try_get_untracked()?;
        if selection.0 != direction {
            return None;
        }
        Some(PreviewStamp { snapshot: self.snapshots.read_stamp()?, selection })
    }

    pub(super) fn accepts(self, stamp: PreviewStamp) -> bool {
        self.selection.try_get_untracked() == Some(stamp.selection)
            && self.snapshots.accepts_read(stamp.snapshot)
    }
}

pub(super) fn direction_mismatch() -> ApiProblem {
    ApiProblem::new("ONCHAIN_PREVIEW_DIRECTION_MISMATCH", "返回计划与所选方向不一致，请重新构建")
        .with_source("frontend.onchain.preview")
        .with_recovery_action(ApiRecoveryAction::RefreshState)
}
