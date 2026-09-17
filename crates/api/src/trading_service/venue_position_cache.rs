//! Per-venue position cache for private WS snapshots and deltas.

use dashmap::DashMap;
use shared_types::{normalized_venue_name, PositionInfo};

use super::{AccountCacheQuality, AccountCacheSnapshot};

mod merge;
use merge::merge_position_rows;

// Bitget UTA position snapshots are followed by incremental events, so the
// authenticated session heartbeat owns freshness after the initial snapshot.
const PRIVATE_WS_SESSION_FRESH_MS: i64 = 120_000;

#[derive(Debug)]
pub(super) struct VenuePositionCache {
    entries: DashMap<String, VenuePositionEntry>,
    ttl_ms: i64,
    max_stale_ms: i64,
}

#[derive(Debug, Clone)]
struct VenuePositionEntry {
    epoch: u64,
    refreshed_at_ms: i64,
    session_refreshed_at_ms: Option<i64>,
    invalidated: bool,
    rows: Vec<PositionInfo>,
}

impl VenuePositionCache {
    pub(super) fn new(ttl_ms: i64, max_stale_ms: i64) -> Self {
        Self {
            entries: DashMap::new(),
            ttl_ms,
            max_stale_ms,
        }
    }

    pub(super) fn replace(&self, venue: &str, epoch: u64, rows: Vec<PositionInfo>) {
        self.entries.insert(
            normalized_venue_name(venue),
            VenuePositionEntry {
                epoch,
                refreshed_at_ms: common::time::now_ms(),
                session_refreshed_at_ms: None,
                invalidated: false,
                rows,
            },
        );
    }

    pub(super) fn upsert(&self, venue: &str, epoch: u64, rows: Vec<PositionInfo>) -> bool {
        let venue = normalized_venue_name(venue);
        let Some(mut current) = self
            .entries
            .get(&venue)
            .filter(|entry| entry.epoch == epoch)
            .map(|entry| entry.rows.clone())
        else {
            return false;
        };
        merge_position_rows(&mut current, rows);
        self.replace(&venue, epoch, current);
        true
    }

    pub(super) fn fresh_all(
        &self,
        venues: &[String],
        epoch: u64,
        now_ms: i64,
    ) -> Option<Vec<PositionInfo>> {
        self.rows_all(venues, epoch, now_ms, self.ttl_ms, false)
    }

    pub(super) fn stale_all(
        &self,
        venues: &[String],
        epoch: u64,
        now_ms: i64,
    ) -> Option<Vec<PositionInfo>> {
        self.rows_all(venues, epoch, now_ms, self.max_stale_ms, true)
    }

    pub(super) fn stale(&self, venue: &str, epoch: u64, now_ms: i64) -> Option<Vec<PositionInfo>> {
        self.entries
            .get(&normalized_venue_name(venue))
            .and_then(|entry| match_entry(&entry, epoch, now_ms, self.max_stale_ms, true))
    }

    pub(super) fn fresh(&self, venue: &str, epoch: u64, now_ms: i64) -> Option<Vec<PositionInfo>> {
        self.entries
            .get(&normalized_venue_name(venue))
            .and_then(|entry| match_entry(&entry, epoch, now_ms, self.ttl_ms, false))
    }

    pub(super) fn invalidate(&self, venue: &str) {
        if let Some(mut entry) = self.entries.get_mut(&normalized_venue_name(venue)) {
            entry.invalidated = true;
        }
    }

    pub(super) fn touch(&self, venue: &str, epoch: u64) {
        self.touch_at(venue, epoch, common::time::now_ms());
    }

    fn touch_at(&self, venue: &str, epoch: u64, observed_at_ms: i64) {
        if let Some(mut entry) = self.entries.get_mut(&normalized_venue_name(venue)) {
            if entry.epoch == epoch && !entry.invalidated {
                entry.session_refreshed_at_ms = Some(observed_at_ms);
            }
        }
    }

    pub(super) fn fresh_venues(&self, epoch: u64, now_ms: i64) -> Vec<String> {
        let mut venues = self
            .entries
            .iter()
            .filter(|entry| {
                let refreshed_at_ms = effective_refreshed_at_ms(entry);
                entry.epoch == epoch
                    && !entry.invalidated
                    && now_ms.saturating_sub(refreshed_at_ms)
                        <= effective_age_budget(entry, self.ttl_ms)
            })
            .map(|entry| entry.key().clone())
            .collect::<Vec<_>>();
        venues.sort_unstable();
        venues
    }

    pub(super) fn clear(&self) {
        self.entries.clear();
    }

    pub(super) fn snapshots(&self, epoch: u64, now_ms: i64) -> Vec<AccountCacheSnapshot> {
        self.entries
            .iter()
            .map(|entry| {
                entry_snapshot(
                    entry.key(),
                    &entry,
                    epoch,
                    now_ms,
                    self.ttl_ms,
                    self.max_stale_ms,
                )
            })
            .collect()
    }

    fn rows_all(
        &self,
        venues: &[String],
        epoch: u64,
        now_ms: i64,
        age_budget_ms: i64,
        allow_invalidated: bool,
    ) -> Option<Vec<PositionInfo>> {
        let mut out = Vec::new();
        for venue in venues {
            let rows = self.entries.get(venue).and_then(|entry| {
                match_entry(&entry, epoch, now_ms, age_budget_ms, allow_invalidated)
            })?;
            out.extend(rows);
        }
        Some(out)
    }
}

fn entry_snapshot(
    venue: &str,
    entry: &VenuePositionEntry,
    epoch: u64,
    now_ms: i64,
    ttl_ms: i64,
    max_stale_ms: i64,
) -> AccountCacheSnapshot {
    let refreshed_at_ms = effective_refreshed_at_ms(entry);
    let freshness_ms = now_ms.saturating_sub(refreshed_at_ms);
    AccountCacheSnapshot {
        venue: venue.to_owned(),
        rows: entry.rows.len() as u64,
        freshness_ms,
        observed_at_ms: refreshed_at_ms,
        quality: cache_quality(
            entry.epoch,
            epoch,
            freshness_ms,
            effective_age_budget(entry, ttl_ms),
            effective_age_budget(entry, max_stale_ms),
            entry.invalidated,
        ),
    }
}

fn cache_quality(
    entry_epoch: u64,
    epoch: u64,
    freshness_ms: i64,
    ttl_ms: i64,
    max_stale_ms: i64,
    invalidated: bool,
) -> AccountCacheQuality {
    if entry_epoch != epoch {
        return AccountCacheQuality::WrongEpoch;
    }
    if !invalidated && freshness_ms <= ttl_ms {
        AccountCacheQuality::Fresh
    } else if freshness_ms <= max_stale_ms {
        AccountCacheQuality::Stale
    } else {
        AccountCacheQuality::Expired
    }
}

fn match_entry(
    entry: &VenuePositionEntry,
    epoch: u64,
    now_ms: i64,
    age_budget_ms: i64,
    allow_invalidated: bool,
) -> Option<Vec<PositionInfo>> {
    if entry.epoch != epoch || (entry.invalidated && !allow_invalidated) {
        return None;
    }
    let age = now_ms.saturating_sub(effective_refreshed_at_ms(entry));
    if age <= effective_age_budget(entry, age_budget_ms) {
        Some(entry.rows.clone())
    } else {
        None
    }
}

fn effective_refreshed_at_ms(entry: &VenuePositionEntry) -> i64 {
    if entry.invalidated {
        entry.refreshed_at_ms
    } else {
        entry
            .session_refreshed_at_ms
            .unwrap_or(entry.refreshed_at_ms)
    }
}

fn effective_age_budget(entry: &VenuePositionEntry, default_ms: i64) -> i64 {
    if entry.invalidated || entry.session_refreshed_at_ms.is_none() {
        default_ms
    } else {
        default_ms.max(PRIVATE_WS_SESSION_FRESH_MS)
    }
}

#[cfg(test)]
#[path = "venue_position_cache_tests.rs"]
mod tests;
