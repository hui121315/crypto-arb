//! Binance USDT-M `fundingInfo` metadata parsing and cache types.

use dashmap::DashMap;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};

/// `/fapi/v1/fundingInfo` cache.
///
/// Binance only returns symbols with non-default funding configuration. Missing
/// symbols must be treated as the default 8h funding interval by the caller.
#[derive(Debug, Default)]
pub(super) struct FundingIntervalCache {
    /// symbol -> funding interval in hours.
    map: DashMap<String, u32>,
    fetched_at_ms: AtomicI64,
}

pub(super) const FUNDING_INFO_TTL_MS: i64 = 24 * 60 * 60 * 1000; // 24h

impl FundingIntervalCache {
    pub(super) fn is_fresh(&self, now_ms: i64) -> bool {
        let fetched_at = self.fetched_at_ms.load(Ordering::Relaxed);
        fetched_at != 0 && now_ms.saturating_sub(fetched_at) < FUNDING_INFO_TTL_MS
    }

    pub(super) fn interval_for(&self, symbol: &str) -> u32 {
        self.map.get(symbol).map(|entry| *entry).unwrap_or(8)
    }

    pub(super) fn replace(&self, intervals: HashMap<String, u32>, fetched_at_ms: i64) {
        self.map.clear();
        for (symbol, interval) in intervals {
            self.map.insert(symbol, interval);
        }
        self.fetched_at_ms.store(fetched_at_ms, Ordering::Relaxed);
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct FundingInfoItem {
    symbol: String,
    #[serde(default)]
    funding_interval_hours: i32,
}

pub(super) fn funding_interval_map(items: Vec<FundingInfoItem>) -> HashMap<String, u32> {
    items
        .into_iter()
        .filter_map(|item| {
            let hours = u32::try_from(item.funding_interval_hours).ok()?;
            (hours != 0 && hours != 8).then_some((item.symbol, hours))
        })
        .collect()
}

#[cfg(test)]
#[path = "binance_funding_info_tests.rs"]
mod tests;
