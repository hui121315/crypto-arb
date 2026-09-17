#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::lifecycle::market_data) struct WsLiveIngestStats {
    pub(in crate::lifecycle::market_data) requested: usize,
    pub(in crate::lifecycle::market_data) rows: usize,
    pub(in crate::lifecycle::market_data) changed_rows: usize,
    pub(in crate::lifecycle::market_data) mark_changed_rows: usize,
}

impl WsLiveIngestStats {
    pub(super) fn merge(&mut self, other: Self) {
        self.requested = self.requested.saturating_add(other.requested);
        self.rows = self.rows.saturating_add(other.rows);
        self.changed_rows = self.changed_rows.saturating_add(other.changed_rows);
        self.mark_changed_rows = self
            .mark_changed_rows
            .saturating_add(other.mark_changed_rows);
    }
}

#[derive(Clone, Copy)]
pub(super) enum WsFeed {
    Perp,
    Spot,
    Funding,
    Mark,
}
