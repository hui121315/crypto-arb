//! 进程内 metrics 计数器。
//!
//! 不依赖 `prometheus` crate，保持极小依赖；Prometheus 文本格式渲染在
//! [`crate::routers::metrics`] 中手工完成。每个值都是无锁 `AtomicU64`，被
//! 后台 updater（写）和 `/health` / `/metrics` 路由（读）并发访问。

use parking_lot::RwLock;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Default)]
pub(crate) struct Metrics {
    // 套利 snapshot
    pub arb_scan_total: AtomicU64,
    pub arb_last_scan_ms: AtomicU64,
    pub arb_last_count: AtomicU64,
    pub arb_last_at_ms: AtomicU64,
    pub ws_arbitrage_payload_bytes: AtomicU64,
    pub ws_arbitrage_top_ids: AtomicU64,
    pub ws_arbitrage_changed_ids: AtomicU64,
    pub ws_arbitrage_changed_rows: AtomicU64,
    pub ws_arbitrage_removed_ids: AtomicU64,
    pub rest_opportunity_list_payload_bytes: AtomicU64,
    pub rest_opportunity_list_serde_ms: AtomicU64,
    pub rest_opportunity_list_rows: AtomicU64,
    pub rest_opportunity_detail_seed_payload_bytes: AtomicU64,
    pub rest_opportunity_detail_seed_serde_ms: AtomicU64,

    // 资金费率
    pub funding_fetch_total: AtomicU64,
    pub funding_last_scan_ms: AtomicU64,
    pub funding_last_count: AtomicU64,
    pub funding_last_at_ms: AtomicU64,
    funding_last_by_exchange: RwLock<BTreeMap<String, u64>>,

    pub alerts_fired_total: AtomicU64,
}

impl Metrics {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn record_arb_scan(&self, scan_ms: u64, count: usize, at_ms: i64) {
        self.arb_scan_total.fetch_add(1, Ordering::Relaxed);
        self.arb_last_scan_ms.store(scan_ms, Ordering::Relaxed);
        self.arb_last_count.store(count as u64, Ordering::Relaxed);
        self.arb_last_at_ms
            .store(at_ms.max(0) as u64, Ordering::Relaxed);
    }

    pub(crate) fn record_ws_arbitrage_payload(
        &self,
        payload_bytes: usize,
        top_ids: usize,
        changed_ids: usize,
        changed_rows: usize,
        removed_ids: usize,
    ) {
        self.ws_arbitrage_payload_bytes
            .store(payload_bytes as u64, Ordering::Relaxed);
        self.ws_arbitrage_top_ids
            .store(top_ids as u64, Ordering::Relaxed);
        self.ws_arbitrage_changed_ids
            .store(changed_ids as u64, Ordering::Relaxed);
        self.ws_arbitrage_changed_rows
            .store(changed_rows as u64, Ordering::Relaxed);
        self.ws_arbitrage_removed_ids
            .store(removed_ids as u64, Ordering::Relaxed);
    }

    pub(crate) fn record_rest_opportunity_list_payload(
        &self,
        payload_bytes: usize,
        serde_ms: u64,
        rows: usize,
    ) {
        self.rest_opportunity_list_payload_bytes
            .store(payload_bytes as u64, Ordering::Relaxed);
        self.rest_opportunity_list_serde_ms
            .store(serde_ms, Ordering::Relaxed);
        self.rest_opportunity_list_rows
            .store(rows as u64, Ordering::Relaxed);
    }

    pub(crate) fn record_rest_opportunity_detail_seed_payload(
        &self,
        payload_bytes: usize,
        serde_ms: u64,
    ) {
        self.rest_opportunity_detail_seed_payload_bytes
            .store(payload_bytes as u64, Ordering::Relaxed);
        self.rest_opportunity_detail_seed_serde_ms
            .store(serde_ms, Ordering::Relaxed);
    }

    pub(crate) fn record_funding_fetch(&self, scan_ms: u64, count: usize, at_ms: i64) {
        self.funding_fetch_total.fetch_add(1, Ordering::Relaxed);
        self.funding_last_scan_ms.store(scan_ms, Ordering::Relaxed);
        self.funding_last_count
            .store(count as u64, Ordering::Relaxed);
        self.funding_last_at_ms
            .store(at_ms.max(0) as u64, Ordering::Relaxed);
    }

    pub(crate) fn record_funding_exchange_counts(&self, counts: BTreeMap<String, u64>) {
        *self.funding_last_by_exchange.write() = counts;
    }

    pub(crate) fn funding_exchange_counts(&self) -> BTreeMap<String, u64> {
        self.funding_last_by_exchange.read().clone()
    }

    pub(crate) fn record_alert_fired(&self) {
        self.alerts_fired_total.fetch_add(1, Ordering::Relaxed);
    }

    /// 当前所有计数器的稳定快照，供 health / metrics 路由读取。
    pub(crate) fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            arb_scan_total: self.arb_scan_total.load(Ordering::Relaxed),
            arb_last_scan_ms: self.arb_last_scan_ms.load(Ordering::Relaxed),
            arb_last_count: self.arb_last_count.load(Ordering::Relaxed),
            arb_last_at_ms: self.arb_last_at_ms.load(Ordering::Relaxed) as i64,
            ws_arbitrage_payload_bytes: self.ws_arbitrage_payload_bytes.load(Ordering::Relaxed),
            ws_arbitrage_top_ids: self.ws_arbitrage_top_ids.load(Ordering::Relaxed),
            ws_arbitrage_changed_ids: self.ws_arbitrage_changed_ids.load(Ordering::Relaxed),
            ws_arbitrage_changed_rows: self.ws_arbitrage_changed_rows.load(Ordering::Relaxed),
            ws_arbitrage_removed_ids: self.ws_arbitrage_removed_ids.load(Ordering::Relaxed),
            rest_opportunity_list_payload_bytes: self
                .rest_opportunity_list_payload_bytes
                .load(Ordering::Relaxed),
            rest_opportunity_list_serde_ms: self
                .rest_opportunity_list_serde_ms
                .load(Ordering::Relaxed),
            rest_opportunity_list_rows: self.rest_opportunity_list_rows.load(Ordering::Relaxed),
            rest_opportunity_detail_seed_payload_bytes: self
                .rest_opportunity_detail_seed_payload_bytes
                .load(Ordering::Relaxed),
            rest_opportunity_detail_seed_serde_ms: self
                .rest_opportunity_detail_seed_serde_ms
                .load(Ordering::Relaxed),
            funding_fetch_total: self.funding_fetch_total.load(Ordering::Relaxed),
            funding_last_scan_ms: self.funding_last_scan_ms.load(Ordering::Relaxed),
            funding_last_count: self.funding_last_count.load(Ordering::Relaxed),
            funding_last_at_ms: self.funding_last_at_ms.load(Ordering::Relaxed) as i64,
            alerts_fired_total: self.alerts_fired_total.load(Ordering::Relaxed),
        }
    }
}

#[derive(Debug, Clone, Copy, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MetricsSnapshot {
    pub arb_scan_total: u64,
    pub arb_last_scan_ms: u64,
    pub arb_last_count: u64,
    pub arb_last_at_ms: i64,
    pub ws_arbitrage_payload_bytes: u64,
    pub ws_arbitrage_top_ids: u64,
    pub ws_arbitrage_changed_ids: u64,
    pub ws_arbitrage_changed_rows: u64,
    pub ws_arbitrage_removed_ids: u64,
    pub rest_opportunity_list_payload_bytes: u64,
    pub rest_opportunity_list_serde_ms: u64,
    pub rest_opportunity_list_rows: u64,
    pub rest_opportunity_detail_seed_payload_bytes: u64,
    pub rest_opportunity_detail_seed_serde_ms: u64,
    pub funding_fetch_total: u64,
    pub funding_last_scan_ms: u64,
    pub funding_last_count: u64,
    pub funding_last_at_ms: i64,
    pub alerts_fired_total: u64,
}

#[cfg(test)]
mod tests {
    use super::Metrics;

    #[test]
    fn record_ws_arbitrage_payload_updates_latest_gauges() {
        let metrics = Metrics::new();

        metrics.record_ws_arbitrage_payload(321, 20, 3, 2, 1);

        let snap = metrics.snapshot();
        assert_eq!(snap.ws_arbitrage_payload_bytes, 321);
        assert_eq!(snap.ws_arbitrage_top_ids, 20);
        assert_eq!(snap.ws_arbitrage_changed_ids, 3);
        assert_eq!(snap.ws_arbitrage_changed_rows, 2);
        assert_eq!(snap.ws_arbitrage_removed_ids, 1);
    }

    #[test]
    fn record_rest_opportunity_payloads_update_latest_gauges() {
        let metrics = Metrics::new();

        metrics.record_rest_opportunity_list_payload(12_345, 7, 120);
        metrics.record_rest_opportunity_detail_seed_payload(4_321, 3);

        let snap = metrics.snapshot();
        assert_eq!(snap.rest_opportunity_list_payload_bytes, 12_345);
        assert_eq!(snap.rest_opportunity_list_serde_ms, 7);
        assert_eq!(snap.rest_opportunity_list_rows, 120);
        assert_eq!(snap.rest_opportunity_detail_seed_payload_bytes, 4_321);
        assert_eq!(snap.rest_opportunity_detail_seed_serde_ms, 3);
    }
}
