//! PR-DP-08 D-8：per-venue balance cache。
//!
//! 替换 single-entry `AccountCache<Vec<VenueBalanceInfo>>` 模型。原来 cache key
//! 是 `(adapter_name, epoch, credentials_hash)` 整张多 venue 表，但 dispatcher
//! `spawn_*_private_ws` 把单 venue credentials wrap 成 `AdapterCredentials {
//! venue_live: Some(...), ..None }` 推 cache，而生产读路径
//! `list_configured_balances(current_adapter_credentials())` 用全 venue
//! credentials 读 → 哈希不一致 → cache 永远不命中（参见 reproduction test
//! `crates/api/src/trading_service/private_ws_events.rs::tests::ws_balances_with_single_venue_credentials_does_not_serve_full_credentials_read`）。
//!
//! 新模型：`DashMap<venue, VenueBalanceEntry>`，每 venue 独立 entry + 独立 TTL，
//! mapper 推 `PrivateBalancesSnapshot { venue, rows }` 只替换该 venue 的 entry。
//! `epoch` 与 `TradingService::account_cache_epoch` 关联：adapter 切换增 epoch
//! → 所有 entry 自然 stale。

use dashmap::DashMap;
use shared_types::{normalized_venue_name, VenueBalanceInfo};

use super::{AccountCacheQuality, AccountCacheSnapshot};

#[derive(Debug)]
pub(super) struct VenueBalanceCache {
    entries: DashMap<String, VenueBalanceEntry>,
    ttl_ms: i64,
    max_stale_ms: i64,
}

#[derive(Debug, Clone)]
struct VenueBalanceEntry {
    epoch: u64,
    refreshed_at_ms: i64,
    invalidated: bool,
    rows: Vec<VenueBalanceInfo>,
}

impl VenueBalanceCache {
    pub(super) fn new(ttl_ms: i64, max_stale_ms: i64) -> Self {
        Self {
            entries: DashMap::new(),
            ttl_ms,
            max_stale_ms,
        }
    }

    /// 整表替换某 venue 的 cache rows（覆盖该 venue 全部 currency 行），并打时间戳。
    /// `epoch` 必须等于 service 当前 epoch，否则后续 `fresh` / `stale` 拒绝返回。
    pub(super) fn replace(&self, venue: &str, epoch: u64, rows: Vec<VenueBalanceInfo>) {
        self.replace_at(venue, epoch, rows, common::time::now_ms());
    }

    pub(super) fn replace_at(
        &self,
        venue: &str,
        epoch: u64,
        rows: Vec<VenueBalanceInfo>,
        refreshed_at_ms: i64,
    ) {
        self.entries.insert(
            normalized_venue_name(venue),
            VenueBalanceEntry {
                epoch,
                refreshed_at_ms,
                invalidated: false,
                rows,
            },
        );
    }

    /// 局部更新该 venue 的 currency 行。仅用于官方文档证明 payload 是当前余额
    /// 状态的 private WS 事件；否则 mapper 必须走 `AccountDirty`。
    pub(super) fn upsert(&self, venue: &str, epoch: u64, rows: Vec<VenueBalanceInfo>) {
        let venue = normalized_venue_name(venue);
        let mut current = self
            .entries
            .get(&venue)
            .filter(|entry| entry.epoch == epoch)
            .map(|entry| entry.rows.clone())
            .unwrap_or_default();
        merge_balance_rows(&mut current, rows);
        self.replace(&venue, epoch, current);
    }

    /// 当前 epoch 下该 venue 是否有 fresh rows（age ≤ `ttl_ms`）。
    pub(super) fn fresh(
        &self,
        venue: &str,
        epoch: u64,
        now_ms: i64,
    ) -> Option<Vec<VenueBalanceInfo>> {
        self.entries
            .get(&normalized_venue_name(venue))
            .and_then(|entry| match_entry(&entry, epoch, now_ms, self.ttl_ms, false))
    }

    /// 当前 epoch 下该 venue 是否有 bounded stale rows（fresh 过期但 age ≤ `max_stale_ms`）。
    /// 用于 fetch 失败时的降级返回。
    pub(super) fn stale(
        &self,
        venue: &str,
        epoch: u64,
        now_ms: i64,
    ) -> Option<Vec<VenueBalanceInfo>> {
        self.entries
            .get(&normalized_venue_name(venue))
            .and_then(|entry| match_entry(&entry, epoch, now_ms, self.max_stale_ms, true))
    }

    /// 清空所有 venue entries，仅用于 adapter/credential epoch 切换。
    pub(super) fn clear(&self) {
        self.entries.clear();
    }

    pub(super) fn remove_many(&self, venues: &[String]) {
        for venue in venues {
            self.entries.remove(&normalized_venue_name(venue));
        }
    }

    pub(super) fn invalidate(&self, venue: &str) {
        if let Some(mut entry) = self.entries.get_mut(&normalized_venue_name(venue)) {
            entry.invalidated = true;
        }
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
}

fn entry_snapshot(
    venue: &str,
    entry: &VenueBalanceEntry,
    epoch: u64,
    now_ms: i64,
    ttl_ms: i64,
    max_stale_ms: i64,
) -> AccountCacheSnapshot {
    let freshness_ms = now_ms.saturating_sub(entry.refreshed_at_ms);
    AccountCacheSnapshot {
        venue: venue.to_owned(),
        rows: entry.rows.len() as u64,
        freshness_ms,
        observed_at_ms: entry.refreshed_at_ms,
        quality: cache_quality(
            entry.epoch,
            epoch,
            freshness_ms,
            ttl_ms,
            max_stale_ms,
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
    entry: &VenueBalanceEntry,
    epoch: u64,
    now_ms: i64,
    age_budget_ms: i64,
    allow_invalidated: bool,
) -> Option<Vec<VenueBalanceInfo>> {
    if entry.epoch != epoch || (entry.invalidated && !allow_invalidated) {
        return None;
    }
    let age = now_ms.saturating_sub(entry.refreshed_at_ms);
    if age <= age_budget_ms {
        Some(entry.rows.clone())
    } else {
        None
    }
}

fn merge_balance_rows(current: &mut Vec<VenueBalanceInfo>, updates: Vec<VenueBalanceInfo>) {
    for row in updates {
        current.retain(|existing| {
            !existing.currency.eq_ignore_ascii_case(&row.currency)
                || normalized_venue_name(&existing.venue) != normalized_venue_name(&row.venue)
        });
        current.push(row);
    }
}

#[cfg(test)]
#[path = "venue_balance_cache/tests.rs"]
mod tests;
