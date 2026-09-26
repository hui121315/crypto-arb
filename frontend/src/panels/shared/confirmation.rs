use crate::panels::modules::opportunity_counts::snapshot_clock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ConfirmedAt(i64, i64);

impl ConfirmedAt {
    pub(crate) fn now() -> Self {
        let (wall, monotonic) = snapshot_clock();
        Self(wall, monotonic)
    }

    pub(crate) fn expired(self) -> bool {
        let (wall, monotonic) = snapshot_clock();
        // Wall time covers sleep; monotonic time prevents clock rollback renewal.
        wall.saturating_sub(self.0).max(monotonic.saturating_sub(self.1)) > 15_000
    }

    pub(crate) fn latest(self, other: Self) -> Self {
        if self.1 >= other.1 { self } else { other }
    }
}
