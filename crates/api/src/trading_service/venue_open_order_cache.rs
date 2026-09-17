//! Complete per-venue open-order snapshots with private WS delta projection.

use dashmap::{mapref::entry::Entry, DashMap};
use shared_types::{normalized_venue_name, OrderInfo, OrderStatus};
use std::sync::atomic::{AtomicI64, Ordering};

const PRIVATE_WS_SESSION_FRESH_MS: i64 = 120_000;

#[derive(Debug)]
pub(super) struct VenueOpenOrderCache {
    entries: DashMap<String, VenueOpenOrderEntry>,
    ttl_ms: i64,
    max_stale_ms: i64,
    latest_change_ms: AtomicI64,
}

#[derive(Debug, Clone)]
struct VenueOpenOrderEntry {
    epoch: u64,
    reconciled_at_ms: i64,
    session_refreshed_at_ms: Option<i64>,
    invalidated: bool,
    rows: Vec<OrderInfo>,
}

impl VenueOpenOrderCache {
    pub(super) fn new(ttl_ms: i64, max_stale_ms: i64) -> Self {
        Self {
            entries: DashMap::new(),
            ttl_ms,
            max_stale_ms,
            latest_change_ms: AtomicI64::new(0),
        }
    }

    pub(super) fn replace(&self, venue: &str, epoch: u64, rows: Vec<OrderInfo>) {
        self.replace_at(venue, epoch, rows, common::time::now_ms());
    }

    fn replace_at(&self, venue: &str, epoch: u64, rows: Vec<OrderInfo>, observed_at_ms: i64) {
        let venue = normalized_venue_name(venue);
        let rows = normalize_open_rows(&venue, rows);
        let changed = match self.entries.entry(venue) {
            Entry::Occupied(mut occupied) => {
                let current = occupied.get_mut();
                let changed = current.epoch != epoch || current.invalidated || current.rows != rows;
                *current = VenueOpenOrderEntry {
                    epoch,
                    reconciled_at_ms: observed_at_ms,
                    session_refreshed_at_ms: None,
                    invalidated: false,
                    rows,
                };
                changed
            }
            Entry::Vacant(vacant) => {
                vacant.insert(VenueOpenOrderEntry {
                    epoch,
                    reconciled_at_ms: observed_at_ms,
                    session_refreshed_at_ms: None,
                    invalidated: false,
                    rows,
                });
                true
            }
        };
        if changed {
            self.note_change(observed_at_ms);
        }
    }

    /// Applies a private order delta only after a complete REST snapshot exists.
    pub(super) fn apply_order(&self, venue: &str, epoch: u64, mut order: OrderInfo) -> bool {
        let venue = normalized_venue_name(venue);
        let Some(mut entry) = self.entries.get_mut(&venue) else {
            return false;
        };
        if entry.epoch != epoch || entry.invalidated {
            return false;
        }
        order.exchange = venue;
        let before = entry.rows.clone();
        merge_order(&mut entry.rows, order);
        let changed = entry.rows != before;
        drop(entry);
        if changed {
            self.note_change(common::time::now_ms());
        }
        changed
    }

    pub(super) fn remove_by_exchange_order_id(
        &self,
        venue: &str,
        epoch: u64,
        exchange_order_id: &str,
    ) -> bool {
        let venue = normalized_venue_name(venue);
        let Some(mut entry) = self.entries.get_mut(&venue) else {
            return false;
        };
        if entry.epoch != epoch || entry.invalidated {
            return false;
        }
        let before = entry.rows.len();
        entry.rows.retain(|row| row.order_id != exchange_order_id);
        let changed = entry.rows.len() != before;
        drop(entry);
        if changed {
            self.note_change(common::time::now_ms());
        }
        changed
    }

    #[cfg(test)]
    pub(super) fn fresh_all(
        &self,
        venues: &[String],
        epoch: u64,
        now_ms: i64,
    ) -> Option<Vec<OrderInfo>> {
        self.rows_all(venues, epoch, now_ms, self.ttl_ms, false)
    }

    pub(super) fn stale_all(
        &self,
        venues: &[String],
        epoch: u64,
        now_ms: i64,
    ) -> Option<Vec<OrderInfo>> {
        self.rows_all(venues, epoch, now_ms, self.max_stale_ms, true)
    }

    pub(super) fn stale(&self, venue: &str, epoch: u64, now_ms: i64) -> Option<Vec<OrderInfo>> {
        self.entries
            .get(&normalized_venue_name(venue))
            .and_then(|entry| match_entry(&entry, epoch, now_ms, self.max_stale_ms, true))
    }

    pub(super) fn fresh(&self, venue: &str, epoch: u64, now_ms: i64) -> Option<Vec<OrderInfo>> {
        self.entries
            .get(&normalized_venue_name(venue))
            .and_then(|entry| match_entry(&entry, epoch, now_ms, self.ttl_ms, false))
    }

    pub(super) fn venues(&self, epoch: u64) -> Vec<String> {
        let mut venues = self
            .entries
            .iter()
            .filter(|entry| entry.epoch == epoch)
            .map(|entry| entry.key().clone())
            .collect::<Vec<_>>();
        venues.sort_unstable();
        venues
    }

    pub(super) fn invalidate(&self, venue: &str) {
        if let Some(mut entry) = self.entries.get_mut(&normalized_venue_name(venue)) {
            if entry.invalidated {
                return;
            }
            entry.invalidated = true;
            drop(entry);
            self.note_change(common::time::now_ms());
        }
    }

    pub(super) fn touch(&self, venue: &str, epoch: u64) {
        if let Some(mut entry) = self.entries.get_mut(&normalized_venue_name(venue)) {
            if entry.epoch == epoch && !entry.invalidated {
                entry.session_refreshed_at_ms = Some(common::time::now_ms());
            }
        }
    }

    pub(super) fn clear(&self) {
        if self.entries.is_empty() {
            return;
        }
        self.entries.clear();
        self.note_change(common::time::now_ms());
    }

    pub(super) fn latest_change_ms(&self) -> i64 {
        self.latest_change_ms.load(Ordering::Relaxed)
    }

    fn rows_all(
        &self,
        venues: &[String],
        epoch: u64,
        now_ms: i64,
        age_budget_ms: i64,
        allow_invalidated: bool,
    ) -> Option<Vec<OrderInfo>> {
        let mut rows = Vec::new();
        for venue in venues {
            let entry = self.entries.get(&normalized_venue_name(venue))?;
            rows.extend(match_entry(
                &entry,
                epoch,
                now_ms,
                age_budget_ms,
                allow_invalidated,
            )?);
        }
        sort_rows(&mut rows);
        Some(rows)
    }

    fn note_change(&self, observed_at_ms: i64) -> i64 {
        let mut current = self.latest_change_ms.load(Ordering::Relaxed);
        loop {
            let next = observed_at_ms.max(current.saturating_add(1));
            match self.latest_change_ms.compare_exchange_weak(
                current,
                next,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return next,
                Err(actual) => current = actual,
            }
        }
    }
}

fn normalize_open_rows(venue: &str, rows: Vec<OrderInfo>) -> Vec<OrderInfo> {
    let mut rows = rows
        .into_iter()
        .filter(|row| is_open_status(row.status))
        .map(|mut row| {
            row.exchange = venue.to_owned();
            row
        })
        .collect::<Vec<_>>();
    sort_rows(&mut rows);
    rows
}

fn merge_order(rows: &mut Vec<OrderInfo>, order: OrderInfo) {
    rows.retain(|row| !same_order(row, &order));
    if is_open_status(order.status) {
        rows.push(order);
        sort_rows(rows);
    }
}

fn same_order(left: &OrderInfo, right: &OrderInfo) -> bool {
    if !left.order_id.is_empty() && !right.order_id.is_empty() {
        return left.order_id == right.order_id;
    }
    left.client_order_id.is_some()
        && left.client_order_id == right.client_order_id
        && left.symbol.eq_ignore_ascii_case(&right.symbol)
}

fn is_open_status(status: OrderStatus) -> bool {
    matches!(
        status,
        OrderStatus::Pending | OrderStatus::Open | OrderStatus::PartiallyFilled
    )
}

fn match_entry(
    entry: &VenueOpenOrderEntry,
    epoch: u64,
    now_ms: i64,
    age_budget_ms: i64,
    allow_invalidated: bool,
) -> Option<Vec<OrderInfo>> {
    if entry.epoch != epoch || (entry.invalidated && !allow_invalidated) {
        return None;
    }
    let refreshed_at_ms = if entry.invalidated {
        entry.reconciled_at_ms
    } else {
        entry
            .session_refreshed_at_ms
            .unwrap_or(entry.reconciled_at_ms)
    };
    let age_budget_ms = if entry.invalidated || entry.session_refreshed_at_ms.is_none() {
        age_budget_ms
    } else {
        age_budget_ms.max(PRIVATE_WS_SESSION_FRESH_MS)
    };
    (now_ms.saturating_sub(refreshed_at_ms) <= age_budget_ms).then(|| entry.rows.clone())
}

fn sort_rows(rows: &mut [OrderInfo]) {
    rows.sort_by(|left, right| {
        left.exchange
            .cmp(&right.exchange)
            .then(left.symbol.cmp(&right.symbol))
            .then(left.order_id.cmp(&right.order_id))
    });
}

#[cfg(test)]
#[path = "venue_open_order_cache_tests.rs"]
mod tests;
