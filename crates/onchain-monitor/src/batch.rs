use arc_swap::ArcSwap;
use dashmap::DashMap;
use shared_types::{
    OnchainBatchItemSnapshot, OnchainBatchSnapshot, OnchainComparisonConfig,
    ONCHAIN_BATCH_MAX_ITEMS,
};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::{OnchainCrossChainQuoteSet, OnchainDexCrossQuoteSet, OnchainQuotePair};

const ACTIVE_TARGET_KEY: &str = "__active__";

#[derive(Debug, Clone)]
pub enum OnchainQuoteTarget {
    Active {
        config: OnchainComparisonConfig,
    },
    Batch {
        item_id: String,
        config: OnchainComparisonConfig,
    },
}

#[derive(Debug, Default)]
struct SchedulerState {
    cursor: usize,
}

#[derive(Debug)]
pub struct OnchainBatchMonitor {
    snapshot: ArcSwap<OnchainBatchSnapshot>,
    quotes: DashMap<String, Arc<OnchainQuotePair>>,
    dex_cross_quotes: DashMap<String, Arc<OnchainDexCrossQuoteSet>>,
    dex_cross_problems: DashMap<String, String>,
    dex_cross_in_flight: DashMap<String, ()>,
    cross_chain_quotes: DashMap<String, Arc<OnchainCrossChainQuoteSet>>,
    cross_chain_problems: DashMap<String, String>,
    cross_chain_in_flight: DashMap<String, ()>,
    cross_chain_attempts: DashMap<String, i64>,
    target_attempts: DashMap<String, i64>,
    provider_attempts: DashMap<String, i64>,
    provider_backoff_until: DashMap<String, i64>,
    provider_failures: DashMap<String, u32>,
    provider_problems: DashMap<String, String>,
    writer: Mutex<SchedulerState>,
}

impl Default for OnchainBatchMonitor {
    fn default() -> Self {
        Self {
            snapshot: ArcSwap::from_pointee(OnchainBatchSnapshot::default()),
            quotes: DashMap::new(),
            dex_cross_quotes: DashMap::new(),
            dex_cross_problems: DashMap::new(),
            dex_cross_in_flight: DashMap::new(),
            cross_chain_quotes: DashMap::new(),
            cross_chain_problems: DashMap::new(),
            cross_chain_in_flight: DashMap::new(),
            cross_chain_attempts: DashMap::new(),
            target_attempts: DashMap::new(),
            provider_attempts: DashMap::new(),
            provider_backoff_until: DashMap::new(),
            provider_failures: DashMap::new(),
            provider_problems: DashMap::new(),
            writer: Mutex::new(SchedulerState::default()),
        }
    }
}

impl OnchainBatchMonitor {
    pub fn snapshot(&self) -> Arc<OnchainBatchSnapshot> {
        self.snapshot.load_full()
    }

    pub fn snapshot_with_active(
        &self,
        active: &OnchainComparisonConfig,
        active_interval_ms: i64,
    ) -> OnchainBatchSnapshot {
        let mut snapshot = (*self.snapshot()).clone();
        let active_id = batch_item_id(active);
        if active.enabled && !snapshot.items.iter().any(|item| item.item_id == active_id) {
            snapshot.estimated_sweep_ms = snapshot
                .estimated_sweep_ms
                .saturating_add(active_interval_ms.max(0));
        }
        snapshot
    }

    pub fn upsert(
        &self,
        mut config: OnchainComparisonConfig,
        quote_interval_ms: i64,
        now_ms: i64,
    ) -> Result<Arc<OnchainBatchSnapshot>, String> {
        config.enabled = true;
        let item_id = batch_item_id(&config);
        let _writer = self.lock_writer();
        let current = self.snapshot();
        let mut next = (*current).clone();
        if let Some(item) = next.items.iter_mut().find(|item| item.item_id == item_id) {
            item.config = config;
            item.quote_interval_ms = quote_interval_ms;
            item.observed_at_ms = now_ms;
        } else {
            if next.items.len() >= ONCHAIN_BATCH_MAX_ITEMS {
                return Err(format!(
                    "batch monitor supports at most {ONCHAIN_BATCH_MAX_ITEMS} items"
                ));
            }
            next.items.push(OnchainBatchItemSnapshot::pending(
                item_id.clone(),
                config,
                quote_interval_ms,
                now_ms,
            ));
        }
        next.items.sort_by(|left, right| {
            left.config
                .base_token
                .cmp(&right.config.base_token)
                .then_with(|| left.config.cex_venue.cmp(&right.config.cex_venue))
        });
        self.target_attempts.remove(&item_id);
        publish_snapshot(&self.snapshot, next, now_ms);
        Ok(self.snapshot())
    }

    pub fn remove(&self, item_id: &str, now_ms: i64) -> bool {
        let _writer = self.lock_writer();
        let current = self.snapshot();
        let mut next = (*current).clone();
        let previous_len = next.items.len();
        next.items.retain(|item| item.item_id != item_id);
        if next.items.len() == previous_len {
            return false;
        }
        self.quotes.remove(item_id);
        self.dex_cross_quotes.remove(item_id);
        self.dex_cross_problems.remove(item_id);
        self.dex_cross_in_flight.remove(item_id);
        self.cross_chain_quotes.remove(item_id);
        self.cross_chain_problems.remove(item_id);
        self.cross_chain_in_flight.remove(item_id);
        self.cross_chain_attempts.remove(item_id);
        self.target_attempts.remove(item_id);
        publish_snapshot(&self.snapshot, next, now_ms);
        true
    }

    pub fn reset_active_attempt(&self) {
        self.target_attempts.remove(ACTIVE_TARGET_KEY);
    }

    pub fn next_due_target(
        &self,
        active: &OnchainComparisonConfig,
        active_interval_ms: i64,
        now_ms: i64,
    ) -> Option<OnchainQuoteTarget> {
        let mut writer = self.lock_writer();
        let active_id = active.enabled.then(|| batch_item_id(active));
        let mut candidates = Vec::new();
        if active.enabled {
            candidates.push(ScheduledTarget {
                key: ACTIVE_TARGET_KEY.to_owned(),
                provider: active.provider.clone(),
                interval_ms: active_interval_ms,
                target: OnchainQuoteTarget::Active {
                    config: active.clone(),
                },
            });
        }
        candidates.extend(
            self.snapshot()
                .items
                .iter()
                .filter(|item| {
                    item.config.enabled
                        && active_id
                            .as_deref()
                            .is_none_or(|active_id| item.item_id != active_id)
                })
                .map(|item| ScheduledTarget {
                    key: item.item_id.clone(),
                    provider: item.config.provider.clone(),
                    interval_ms: item.quote_interval_ms,
                    target: OnchainQuoteTarget::Batch {
                        item_id: item.item_id.clone(),
                        config: item.config.clone(),
                    },
                }),
        );
        if candidates.is_empty() {
            return None;
        }
        let start = writer.cursor % candidates.len();
        for offset in 0..candidates.len() {
            let index = (start + offset) % candidates.len();
            let candidate = &candidates[index];
            if !self.target_due(candidate, now_ms) {
                continue;
            }
            self.target_attempts.insert(candidate.key.clone(), now_ms);
            self.provider_attempts
                .insert(candidate.provider.to_ascii_lowercase(), now_ms);
            writer.cursor = (index + 1) % candidates.len();
            return Some(candidate.target.clone());
        }
        None
    }

    pub fn try_begin_active(
        &self,
        active: &OnchainComparisonConfig,
        interval_ms: i64,
        now_ms: i64,
    ) -> bool {
        let _writer = self.lock_writer();
        let candidate = ScheduledTarget {
            key: ACTIVE_TARGET_KEY.to_owned(),
            provider: active.provider.clone(),
            interval_ms,
            target: OnchainQuoteTarget::Active {
                config: active.clone(),
            },
        };
        if !self.target_due(&candidate, now_ms) {
            return false;
        }
        self.target_attempts
            .insert(ACTIVE_TARGET_KEY.to_owned(), now_ms);
        self.provider_attempts
            .insert(active.provider.to_ascii_lowercase(), now_ms);
        true
    }

    pub fn configs(&self) -> Vec<(String, OnchainComparisonConfig)> {
        self.snapshot()
            .items
            .iter()
            .map(|item| (item.item_id.clone(), item.config.clone()))
            .collect()
    }

    pub fn quote_pair(&self, item_id: &str) -> Option<Arc<OnchainQuotePair>> {
        self.quotes.get(item_id).map(|quote| Arc::clone(&quote))
    }

    pub fn publish_quote_pair(&self, item_id: &str, quotes: OnchainQuotePair) {
        self.quotes.insert(item_id.to_owned(), Arc::new(quotes));
    }

    pub fn dex_cross_quotes(&self, item_id: &str) -> Option<Arc<OnchainDexCrossQuoteSet>> {
        self.dex_cross_quotes
            .get(item_id)
            .map(|quote| Arc::clone(&quote))
    }

    pub fn publish_dex_cross_quotes(&self, item_id: &str, quotes: OnchainDexCrossQuoteSet) {
        self.dex_cross_quotes
            .insert(item_id.to_owned(), Arc::new(quotes));
        self.dex_cross_problems.remove(item_id);
    }

    pub fn dex_cross_problem(&self, item_id: &str) -> Option<String> {
        self.dex_cross_problems
            .get(item_id)
            .map(|problem| problem.value().clone())
    }

    pub fn publish_dex_cross_problem(&self, item_id: &str, problem: String) {
        self.dex_cross_problems.insert(item_id.to_owned(), problem);
    }

    pub fn try_begin_dex_cross_refresh(&self, item_id: &str) -> bool {
        self.dex_cross_in_flight
            .insert(item_id.to_owned(), ())
            .is_none()
    }

    pub fn finish_dex_cross_refresh(&self, item_id: &str) {
        self.dex_cross_in_flight.remove(item_id);
    }

    pub fn cross_chain_quotes(&self, item_id: &str) -> Option<Arc<OnchainCrossChainQuoteSet>> {
        self.cross_chain_quotes
            .get(item_id)
            .map(|quote| Arc::clone(&quote))
    }

    pub fn publish_cross_chain_quotes(&self, item_id: &str, quotes: OnchainCrossChainQuoteSet) {
        self.cross_chain_quotes
            .insert(item_id.to_owned(), Arc::new(quotes));
        self.cross_chain_problems.remove(item_id);
    }

    pub fn cross_chain_problem(&self, item_id: &str) -> Option<String> {
        self.cross_chain_problems
            .get(item_id)
            .map(|problem| problem.value().clone())
    }

    pub fn publish_cross_chain_problem(&self, item_id: &str, problem: String) {
        self.cross_chain_problems
            .insert(item_id.to_owned(), problem);
    }

    pub fn try_begin_cross_chain_refresh(
        &self,
        item_id: &str,
        now_ms: i64,
        min_interval_ms: i64,
    ) -> bool {
        if self
            .cross_chain_in_flight
            .insert(item_id.to_owned(), ())
            .is_some()
        {
            return false;
        }
        let due = self
            .cross_chain_attempts
            .get(item_id)
            .is_none_or(|previous| now_ms.saturating_sub(*previous) >= min_interval_ms.max(1));
        if !due {
            self.cross_chain_in_flight.remove(item_id);
            return false;
        }
        self.cross_chain_attempts.insert(item_id.to_owned(), now_ms);
        true
    }

    pub fn finish_cross_chain_refresh(&self, item_id: &str) {
        self.cross_chain_in_flight.remove(item_id);
    }

    pub fn record_provider_success(&self, provider: &str) {
        let key = provider.trim().to_ascii_lowercase();
        self.provider_failures.remove(&key);
        self.provider_backoff_until.remove(&key);
        self.provider_problems.remove(&key);
    }

    pub fn record_provider_problem(&self, provider: &str, problem: &str) {
        self.provider_problems.insert(
            provider.trim().to_ascii_lowercase(),
            problem.trim().to_owned(),
        );
    }

    pub fn provider_problem(&self, provider: &str) -> Option<String> {
        self.provider_problems
            .get(&provider.trim().to_ascii_lowercase())
            .map(|problem| problem.value().clone())
    }

    pub fn record_provider_failure(
        &self,
        provider: &str,
        now_ms: i64,
        base_delay_ms: i64,
        max_delay_ms: i64,
    ) -> i64 {
        let key = provider.trim().to_ascii_lowercase();
        let failure_count = {
            let mut failures = self.provider_failures.entry(key.clone()).or_insert(0);
            *failures = failures.saturating_add(1);
            *failures
        };
        let multiplier = 1_i64 << failure_count.saturating_sub(1).min(4);
        let delay_ms = base_delay_ms
            .max(1)
            .saturating_mul(multiplier)
            .min(max_delay_ms.max(1));
        self.provider_backoff_until
            .insert(key, now_ms.saturating_add(delay_ms));
        delay_ms
    }

    pub fn record_provider_retry_after(
        &self,
        provider: &str,
        now_ms: i64,
        retry_after_ms: i64,
    ) -> i64 {
        let key = provider.trim().to_ascii_lowercase();
        let delay_ms = retry_after_ms.max(1);
        self.provider_backoff_until
            .insert(key, now_ms.saturating_add(delay_ms));
        delay_ms
    }

    pub fn provider_retry_after_ms(&self, provider: &str, now_ms: i64) -> Option<i64> {
        let key = provider.trim().to_ascii_lowercase();
        let retry_after_ms = self
            .provider_backoff_until
            .get(&key)
            .map(|until_ms| until_ms.saturating_sub(now_ms))?;
        (retry_after_ms > 0).then_some(retry_after_ms)
    }

    pub fn update_item(&self, item: OnchainBatchItemSnapshot, now_ms: i64, force: bool) -> bool {
        let _writer = self.lock_writer();
        let current = self.snapshot();
        let mut next = (*current).clone();
        let Some(existing) = next
            .items
            .iter_mut()
            .find(|existing| existing.item_id == item.item_id)
        else {
            return false;
        };
        let source_changed = existing.config != item.config
            || existing.quality != item.quality
            || existing.best_direction != item.best_direction
            || existing.best_gross_spread_bps != item.best_gross_spread_bps
            || existing.best_net_spread_bps != item.best_net_spread_bps
            || existing.observable_notional_usd != item.observable_notional_usd
            || existing.quote_observed_at_ms != item.quote_observed_at_ms
            || existing.dex_quality != item.dex_quality
            || existing.best_dex_direction != item.best_dex_direction
            || existing.best_dex_net_return_bps != item.best_dex_net_return_bps
            || existing.dex_problem != item.dex_problem
            || existing.cross_chain_quality != item.cross_chain_quality
            || existing.best_cross_chain_net_return_bps != item.best_cross_chain_net_return_bps
            || existing.cross_chain_problem != item.cross_chain_problem
            || existing.cex_observed_at_ms != item.cex_observed_at_ms
            || existing.cex_problem != item.cex_problem
            || existing.cex_retry_after_ms != item.cex_retry_after_ms
            || existing.provider_configured != item.provider_configured
            || existing.provider_problem != item.provider_problem
            || existing.degradation_reasons != item.degradation_reasons;
        let heartbeat_due = now_ms.saturating_sub(existing.observed_at_ms) >= 1_000;
        if !(force || source_changed || heartbeat_due) {
            return false;
        }
        *existing = item;
        publish_snapshot(&self.snapshot, next, now_ms);
        true
    }

    fn target_due(&self, target: &ScheduledTarget, now_ms: i64) -> bool {
        let interval_ms = target.interval_ms.max(1);
        let target_due = self
            .target_attempts
            .get(&target.key)
            .is_none_or(|previous| now_ms.saturating_sub(*previous) >= interval_ms);
        let provider_key = target.provider.to_ascii_lowercase();
        let provider_backoff_elapsed = self
            .provider_backoff_until
            .get(&provider_key)
            .is_none_or(|until_ms| now_ms >= *until_ms);
        let provider_due = self
            .provider_attempts
            .get(&provider_key)
            .is_none_or(|previous| now_ms.saturating_sub(*previous) >= interval_ms);
        target_due && provider_due && provider_backoff_elapsed
    }

    fn lock_writer(&self) -> MutexGuard<'_, SchedulerState> {
        self.writer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[derive(Debug, Clone)]
struct ScheduledTarget {
    key: String,
    provider: String,
    interval_ms: i64,
    target: OnchainQuoteTarget,
}

pub fn batch_item_id(config: &OnchainComparisonConfig) -> String {
    let mut hasher = DefaultHasher::new();
    for value in [
        &config.chain,
        &config.provider,
        &config.base_mint,
        &config.quote_mint,
        &config.cex_venue,
        &config.cex_symbol,
        &config.dex_comparison.peer_provider,
        &config.cross_chain.peer_item_id,
        &config.cross_chain.provider,
    ] {
        value.trim().to_ascii_lowercase().hash(&mut hasher);
    }
    config.dex_comparison.enabled.hash(&mut hasher);
    config.cross_chain.enabled.hash(&mut hasher);
    format!("watch-{:016x}", hasher.finish())
}

fn publish_snapshot(
    target: &ArcSwap<OnchainBatchSnapshot>,
    mut next: OnchainBatchSnapshot,
    now_ms: i64,
) {
    next.estimated_sweep_ms = next
        .items
        .iter()
        .map(|item| item.quote_interval_ms.max(0))
        .sum();
    next.observed_at_ms = now_ms;
    target.store(Arc::new(next));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(symbol: &str) -> OnchainComparisonConfig {
        OnchainComparisonConfig {
            enabled: true,
            base_token: symbol.to_owned(),
            base_mint: format!("mint-{symbol}"),
            cex_symbol: format!("{symbol}/USDC"),
            ..OnchainComparisonConfig::default()
        }
    }

    fn upsert_ok(
        monitor: &OnchainBatchMonitor,
        config: OnchainComparisonConfig,
        interval_ms: i64,
        now_ms: i64,
    ) {
        assert!(monitor.upsert(config, interval_ms, now_ms).is_ok());
    }

    #[test]
    fn watchlist_is_bounded_and_upserts_duplicate_identity() {
        let monitor = OnchainBatchMonitor::default();
        for index in 0..ONCHAIN_BATCH_MAX_ITEMS {
            upsert_ok(&monitor, config(&format!("T{index}")), 2_250, index as i64);
        }
        assert_eq!(monitor.snapshot().items.len(), ONCHAIN_BATCH_MAX_ITEMS);
        upsert_ok(&monitor, config("T0"), 4_500, 99);
        assert_eq!(monitor.snapshot().items.len(), ONCHAIN_BATCH_MAX_ITEMS);
        assert!(monitor.upsert(config("OVER"), 2_250, 100).is_err());
    }

    #[test]
    fn scheduler_shares_provider_budget_and_skips_active_duplicate() {
        let monitor = OnchainBatchMonitor::default();
        let active = config("SOL");
        upsert_ok(&monitor, active.clone(), 2_250, 0);
        upsert_ok(&monitor, config("WIF"), 2_250, 0);

        assert!(matches!(
            monitor.next_due_target(&active, 2_250, 10_000),
            Some(OnchainQuoteTarget::Active { .. })
        ));
        assert!(monitor.next_due_target(&active, 2_250, 12_249).is_none());
        assert!(matches!(
            monitor.next_due_target(&active, 2_250, 12_250),
            Some(OnchainQuoteTarget::Batch { .. })
        ));
    }

    #[test]
    fn disabled_focus_does_not_suppress_the_same_watch_item() {
        let monitor = OnchainBatchMonitor::default();
        let mut active = config("SOL");
        upsert_ok(&monitor, active.clone(), 2_250, 0);
        active.enabled = false;

        assert!(matches!(
            monitor.next_due_target(&active, 2_250, 10_000),
            Some(OnchainQuoteTarget::Batch { .. })
        ));
        assert_eq!(
            monitor
                .snapshot_with_active(&active, 2_250)
                .estimated_sweep_ms,
            2_250
        );
    }

    #[test]
    fn provider_failures_back_off_every_target_and_reset_after_success() {
        let monitor = OnchainBatchMonitor::default();
        let active = config("SOL");

        assert!(monitor.try_begin_active(&active, 2_250, 10_000));
        monitor.record_provider_problem(&active.provider, "Jupiter API 已限速（HTTP 429）");
        assert_eq!(
            monitor.provider_problem(&active.provider).as_deref(),
            Some("Jupiter API 已限速（HTTP 429）")
        );
        assert_eq!(
            monitor.record_provider_failure(&active.provider, 10_100, 5_000, 60_000),
            5_000
        );
        assert_eq!(
            monitor.provider_retry_after_ms(&active.provider, 12_000),
            Some(3_100)
        );
        assert!(!monitor.try_begin_active(&active, 2_250, 15_099));
        assert!(monitor.try_begin_active(&active, 2_250, 15_100));
        assert_eq!(
            monitor.record_provider_failure(&active.provider, 15_200, 5_000, 60_000),
            10_000
        );
        assert!(!monitor.try_begin_active(&active, 2_250, 25_199));

        monitor.record_provider_success(&active.provider);

        assert_eq!(
            monitor.provider_retry_after_ms(&active.provider, 25_200),
            None
        );
        assert_eq!(monitor.provider_problem(&active.provider), None);
        assert!(monitor.try_begin_active(&active, 2_250, 25_200));
    }

    #[test]
    fn provider_retry_after_uses_the_authoritative_delay_without_exponential_growth() {
        let monitor = OnchainBatchMonitor::default();
        let active = config("SOL");

        assert_eq!(
            monitor.record_provider_retry_after(&active.provider, 10_000, 3_250),
            3_250
        );
        assert_eq!(
            monitor.provider_retry_after_ms(&active.provider, 11_000),
            Some(2_250)
        );
        assert_eq!(
            monitor.record_provider_retry_after(&active.provider, 11_500, 750),
            750
        );
        assert_eq!(
            monitor.provider_retry_after_ms(&active.provider, 12_249),
            Some(1)
        );
        assert_eq!(
            monitor.provider_retry_after_ms(&active.provider, 12_250),
            None
        );
    }
}
