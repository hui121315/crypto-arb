//! 多交易所并发聚合器。
//!
//! 职责：注册所有 [`ExchangeAdapter`]，对外提供"一次调用获取所有交易所数据"的入口；
//! 容忍部分交易所失败，失败者打日志后跳过，不影响整体返回。
//!
//! 缓存（30s TTL）由 M6 [`realtime`] crate 在外层包装，本层不缓存。

use crate::adapter::{ExchangeAdapter, MetadataRefreshOutcome};
use crate::error::ExchangeError;
use crate::venue_spec::VenueId;
use arc_swap::ArcSwap;
use futures::future::join_all;
use shared_types::{ExchangeProblem, FundingRateData, MarketDataCoverage, SpotTick, TickerInfo};
use std::fmt;
use std::sync::Arc;
use std::sync::{RwLock, RwLockWriteGuard};
use std::time::Duration;
use tokio::sync::{Semaphore, SemaphorePermit};
use tokio::time::timeout;
use tracing::{debug, warn};

/// 聚合请求按 venue 设置超时。OKX funding-rate 仍需要逐 instId paced REST baseline；
/// Hyperliquid builder DEX 共用父级限速器，需要覆盖排队时间；其他 venue 使用 10s。
fn fanout_timeout(name: &str) -> Duration {
    let seconds = VenueId::from_exchange_name(name)
        .map(|venue| venue.defaults().fanout_timeout_secs)
        .unwrap_or(10);
    Duration::from_secs(seconds)
}

const FANOUT_OP_FUNDING_RATES: &str = "funding_rates";
const FANOUT_OP_PERP_TICKERS: &str = "perp_tickers";
const FANOUT_OP_SPOT_TICKS: &str = "spot_ticks";
const FANOUT_MAX_CONCURRENCY: usize = 2;

pub struct Aggregator {
    registry: RwLock<Vec<AdapterSlot>>,
    snapshot: ArcSwap<Vec<AdapterSlot>>,
    fanout_permits: Semaphore,
}

#[derive(Debug)]
pub struct FanoutReport<T> {
    pub rows: Vec<T>,
    pub venues: Vec<FanoutVenueResult>,
}

#[derive(Debug)]
pub struct FanoutVenueResult {
    pub venue: String,
    pub operation: &'static str,
    pub rows: usize,
    /// Wall-clock latency of this venue's fanout call (including the timeout
    /// window for venues that timed out). Always recorded, never fabricated.
    pub latency_ms: u64,
    pub error: Option<ExchangeError>,
    pub problem: Option<ExchangeProblem>,
}

impl FanoutVenueResult {
    /// Transport success is tracked separately from product coverage.
    pub fn succeeded(&self) -> bool {
        self.error.is_none()
    }

    /// Product coverage requires at least one usable row, not only HTTP success.
    pub fn covered(&self) -> bool {
        self.succeeded() && self.rows > 0
    }
}

impl<T> FanoutReport<T> {
    /// Total venues that participated in the fanout.
    pub fn total_venues(&self) -> usize {
        self.venues.len()
    }

    /// Venues that returned without an error (timeouts/errors excluded).
    pub fn succeeded_venues(&self) -> usize {
        self.venues.iter().filter(|venue| venue.succeeded()).count()
    }

    /// Venues that failed (error or timeout).
    pub fn failed_venues(&self) -> usize {
        self.total_venues() - self.succeeded_venues()
    }

    pub fn covered_venues(&self) -> usize {
        self.venues.iter().filter(|venue| venue.covered()).count()
    }

    /// Uses the shared 0.0–1.0 product coverage contract. Empty successful
    /// responses remain uncovered so a venue cannot look healthy without rows.
    pub fn coverage(&self) -> MarketDataCoverage {
        MarketDataCoverage::new(self.total_venues() as u64, self.covered_venues() as u64)
    }
}

#[derive(Clone)]
struct AdapterSlot {
    name: Arc<str>,
    adapter: Arc<dyn ExchangeAdapter>,
}

impl Default for Aggregator {
    fn default() -> Self {
        Self {
            registry: RwLock::new(Vec::new()),
            snapshot: ArcSwap::from_pointee(Vec::new()),
            fanout_permits: Semaphore::new(FANOUT_MAX_CONCURRENCY),
        }
    }
}

impl fmt::Debug for Aggregator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Aggregator")
            .field("adapters", &self.names())
            .finish()
    }
}

impl Aggregator {
    pub fn new() -> Self {
        Self::default()
    }

    /// 注册（或替换）一个适配器。
    pub fn register(&self, adapter: Arc<dyn ExchangeAdapter>) {
        let name = Arc::<str>::from(adapter.name());
        let mut registry = self.write_registry();
        match registry.iter_mut().find(|slot| slot.name == name) {
            Some(slot) => slot.adapter = adapter,
            None => registry.push(AdapterSlot { name, adapter }),
        }
        registry.sort_by(|left, right| left.name.cmp(&right.name));
        self.publish_snapshot(&registry);
    }

    pub fn unregister(&self, name: &str) -> bool {
        let mut registry = self.write_registry();
        let before = registry.len();
        registry.retain(|slot| slot.name.as_ref() != name);
        let removed = registry.len() != before;
        if removed {
            self.publish_snapshot(&registry);
        }
        removed
    }

    pub fn names(&self) -> Vec<String> {
        self.snapshot
            .load()
            .iter()
            .map(|slot| slot.name.as_ref().to_owned())
            .collect()
    }

    pub fn len(&self) -> usize {
        self.snapshot.load().len()
    }

    pub fn is_empty(&self) -> bool {
        self.snapshot.load().is_empty()
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn ExchangeAdapter>> {
        let snapshot = self.snapshot.load();
        snapshot
            .iter()
            .find(|slot| slot.name.as_ref() == name)
            .or_else(|| cross_ex_family_slot(&snapshot, name))
            .map(|slot| Arc::clone(&slot.adapter))
    }

    fn snapshot(&self) -> Arc<Vec<AdapterSlot>> {
        self.snapshot.load_full()
    }

    fn snapshot_for(&self, venues: &[String]) -> Vec<AdapterSlot> {
        let snapshot = self.snapshot.load();
        snapshot
            .iter()
            .filter(|slot| {
                venues.iter().any(|venue| {
                    shared_types::venue_names_equal(venue.as_str(), slot.name.as_ref())
                })
            })
            .cloned()
            .collect()
    }

    fn write_registry(&self) -> RwLockWriteGuard<'_, Vec<AdapterSlot>> {
        match self.registry.write() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn publish_snapshot(&self, registry: &[AdapterSlot]) {
        self.snapshot.store(Arc::new(registry.to_vec()));
    }

    async fn acquire_fanout_permit(&self) -> Result<SemaphorePermit<'_>, ExchangeError> {
        self.fanout_permits.acquire().await.map_err(|_| {
            ExchangeError::Network("exchange fanout scheduler is unavailable".to_owned())
        })
    }

    pub async fn fetch_all_funding_rates_report(&self) -> FanoutReport<FundingRateData> {
        let snap = self.snapshot();
        self.fetch_funding_rates_report_from(&snap).await
    }

    pub async fn fetch_funding_rates_report_for(
        &self,
        venues: &[String],
    ) -> FanoutReport<FundingRateData> {
        let snap = self.snapshot_for(venues);
        self.fetch_funding_rates_report_from(&snap).await
    }

    async fn fetch_funding_rates_report_from(
        &self,
        snap: &[AdapterSlot],
    ) -> FanoutReport<FundingRateData> {
        debug!(count = snap.len(), "fetch_all_funding_rates");
        let futs = snap.iter().map(|slot| async move {
            let _permit = match self.acquire_fanout_permit().await {
                Ok(permit) => permit,
                Err(error) => {
                    return fanout_err(slot.name.as_ref(), FANOUT_OP_FUNDING_RATES, error, 0);
                }
            };
            let timeout_limit = fanout_timeout(&slot.name);
            let started = tokio::time::Instant::now();
            let outcome = timeout(timeout_limit, slot.adapter.get_funding_rates(None)).await;
            let latency_ms = started.elapsed().as_millis() as u64;
            match outcome {
                Ok(Ok(rates)) => fanout_ok(
                    slot.name.as_ref(),
                    FANOUT_OP_FUNDING_RATES,
                    rates,
                    latency_ms,
                ),
                Ok(Err(e)) => {
                    warn!(exchange = %slot.name, error = %e, "get_funding_rates failed");
                    fanout_err(slot.name.as_ref(), FANOUT_OP_FUNDING_RATES, e, latency_ms)
                }
                Err(_) => {
                    warn!(
                        exchange = %slot.name,
                        timeout_secs = timeout_limit.as_secs(),
                        "get_funding_rates timed out"
                    );
                    fanout_err(
                        slot.name.as_ref(),
                        FANOUT_OP_FUNDING_RATES,
                        ExchangeError::Timeout {
                            seconds: timeout_limit.as_secs(),
                        },
                        latency_ms,
                    )
                }
            }
        });
        fanout_report(join_all(futs).await)
    }

    pub async fn fetch_all_tickers_report(&self) -> FanoutReport<TickerInfo> {
        let snap = self.snapshot();
        self.fetch_tickers_report_from(&snap).await
    }

    pub async fn fetch_tickers_report_for(&self, venues: &[String]) -> FanoutReport<TickerInfo> {
        let snap = self.snapshot_for(venues);
        self.fetch_tickers_report_from(&snap).await
    }

    async fn fetch_tickers_report_from(&self, snap: &[AdapterSlot]) -> FanoutReport<TickerInfo> {
        debug!(count = snap.len(), "fetch_all_tickers");
        let futs = snap.iter().map(|slot| async move {
            let _permit = match self.acquire_fanout_permit().await {
                Ok(permit) => permit,
                Err(error) => {
                    return fanout_err(slot.name.as_ref(), FANOUT_OP_PERP_TICKERS, error, 0);
                }
            };
            let timeout_limit = fanout_timeout(&slot.name);
            let started = tokio::time::Instant::now();
            let outcome = timeout(timeout_limit, slot.adapter.get_tickers(None)).await;
            let latency_ms = started.elapsed().as_millis() as u64;
            match outcome {
                Ok(Ok(tickers)) => fanout_ok(
                    slot.name.as_ref(),
                    FANOUT_OP_PERP_TICKERS,
                    tickers,
                    latency_ms,
                ),
                Ok(Err(e)) => {
                    warn!(exchange = %slot.name, error = %e, "get_tickers failed");
                    fanout_err(slot.name.as_ref(), FANOUT_OP_PERP_TICKERS, e, latency_ms)
                }
                Err(_) => {
                    warn!(
                        exchange = %slot.name,
                        timeout_secs = timeout_limit.as_secs(),
                        "get_tickers timed out"
                    );
                    fanout_err(
                        slot.name.as_ref(),
                        FANOUT_OP_PERP_TICKERS,
                        ExchangeError::Timeout {
                            seconds: timeout_limit.as_secs(),
                        },
                        latency_ms,
                    )
                }
            }
        });
        fanout_report(join_all(futs).await)
    }

    /// PR-DP-04 follow-up: 冷启动 metadata prewarm。并发触发每家 venue 的
    /// [`ExchangeAdapter::refresh_metadata`]，per-venue 复用 [`fanout_timeout`]，
    /// 单家失败 / 超时不阻塞其余。返回 `(venue_name, result)` 给 lifecycle
    /// 选择性记录失败 venue。
    pub async fn refresh_all_metadata(
        &self,
    ) -> Vec<(String, crate::error::ExchangeResult<MetadataRefreshOutcome>)> {
        let snap = self.snapshot();
        debug!(count = snap.len(), "refresh_all_metadata");
        let futs = snap.iter().map(|slot| async move {
            let _permit = match self.acquire_fanout_permit().await {
                Ok(permit) => permit,
                Err(error) => return (slot.name.as_ref().to_owned(), Err(error)),
            };
            let timeout_limit = fanout_timeout(&slot.name);
            let outcome = match timeout(timeout_limit, slot.adapter.refresh_metadata()).await {
                Ok(result) => result,
                Err(_) => Err(crate::error::ExchangeError::Network(format!(
                    "{}: refresh_metadata timed out after {}s",
                    slot.name,
                    timeout_limit.as_secs()
                ))),
            };
            (slot.name.as_ref().to_owned(), outcome)
        });
        join_all(futs).await
    }

    pub async fn fetch_all_spot_ticks_report(&self) -> FanoutReport<SpotTick> {
        let snap = self.snapshot();
        self.fetch_spot_ticks_report_from(&snap).await
    }

    pub async fn fetch_spot_ticks_report_for(&self, venues: &[String]) -> FanoutReport<SpotTick> {
        let snap = self.snapshot_for(venues);
        self.fetch_spot_ticks_report_from(&snap).await
    }

    async fn fetch_spot_ticks_report_from(&self, snap: &[AdapterSlot]) -> FanoutReport<SpotTick> {
        debug!(count = snap.len(), "fetch_all_spot_ticks");
        let futs = snap.iter().map(|slot| async move {
            let _permit = match self.acquire_fanout_permit().await {
                Ok(permit) => permit,
                Err(error) => {
                    return fanout_err(slot.name.as_ref(), FANOUT_OP_SPOT_TICKS, error, 0);
                }
            };
            let timeout_limit = fanout_timeout(&slot.name);
            let started = tokio::time::Instant::now();
            let outcome = timeout(timeout_limit, slot.adapter.get_spot_tickers(None)).await;
            let latency_ms = started.elapsed().as_millis() as u64;
            match outcome {
                Ok(Ok(ticks)) => {
                    fanout_ok(slot.name.as_ref(), FANOUT_OP_SPOT_TICKS, ticks, latency_ms)
                }
                Ok(Err(e)) => {
                    debug!(exchange = %slot.name, error = %e, "get_spot_tickers skipped");
                    fanout_err(slot.name.as_ref(), FANOUT_OP_SPOT_TICKS, e, latency_ms)
                }
                Err(_) => {
                    warn!(
                        exchange = %slot.name,
                        timeout_secs = timeout_limit.as_secs(),
                        "get_spot_tickers timed out"
                    );
                    fanout_err(
                        slot.name.as_ref(),
                        FANOUT_OP_SPOT_TICKS,
                        ExchangeError::Timeout {
                            seconds: timeout_limit.as_secs(),
                        },
                        latency_ms,
                    )
                }
            }
        });
        fanout_report(join_all(futs).await)
    }
}

fn cross_ex_family_slot<'a>(slots: &'a [AdapterSlot], requested: &str) -> Option<&'a AdapterSlot> {
    let family = shared_types::venue_family(requested);
    (family != requested && VenueId::from_exchange_name(requested) == Some(VenueId::GateCrossEx))
        .then(|| slots.iter().find(|slot| slot.name.as_ref() == family))
        .flatten()
}

fn fanout_ok<T>(
    venue: &str,
    operation: &'static str,
    rows: Vec<T>,
    latency_ms: u64,
) -> (Vec<T>, FanoutVenueResult) {
    let row_count = rows.len();
    (
        rows,
        FanoutVenueResult {
            venue: venue.to_owned(),
            operation,
            rows: row_count,
            latency_ms,
            error: None,
            problem: None,
        },
    )
}

fn fanout_err<T>(
    venue: &str,
    operation: &'static str,
    error: ExchangeError,
    latency_ms: u64,
) -> (Vec<T>, FanoutVenueResult) {
    let problem = error
        .to_problem(venue, operation)
        .with_latency_ms(Some(latency_ms));
    (
        Vec::new(),
        FanoutVenueResult {
            venue: venue.to_owned(),
            operation,
            rows: 0,
            latency_ms,
            error: Some(error),
            problem: Some(problem),
        },
    )
}

fn fanout_report<T>(chunks: Vec<(Vec<T>, FanoutVenueResult)>) -> FanoutReport<T> {
    let total_len = chunks.iter().map(|(rows, _)| rows.len()).sum();
    let mut rows = Vec::with_capacity(total_len);
    let mut venues = Vec::with_capacity(chunks.len());
    for (mut chunk, outcome) in chunks {
        rows.append(&mut chunk);
        venues.push(outcome);
    }
    FanoutReport { rows, venues }
}

#[cfg(test)]
fn flatten_with_capacity<T>(chunks: Vec<Vec<T>>) -> Vec<T> {
    let total_len = chunks.iter().map(Vec::len).sum();
    let mut out = Vec::with_capacity(total_len);
    for mut chunk in chunks {
        out.append(&mut chunk);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use rust_decimal::Decimal;
    use shared_types::{FundingRateData, OrderBookInfo, SpotTick, TickerInfo};
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::error::{ExchangeError, ExchangeResult};

    struct MockAdapter {
        name: &'static str,
        fail: bool,
        ticker_delay: Duration,
        calls: AtomicUsize,
        refresh_metadata_fail: bool,
        refresh_metadata_delay: Duration,
        refresh_metadata_calls: AtomicUsize,
    }

    impl MockAdapter {
        fn new(name: &'static str, fail: bool) -> Self {
            Self {
                name,
                fail,
                ticker_delay: Duration::ZERO,
                calls: AtomicUsize::new(0),
                refresh_metadata_fail: false,
                refresh_metadata_delay: Duration::ZERO,
                refresh_metadata_calls: AtomicUsize::new(0),
            }
        }

        fn with_ticker_delay(mut self, delay: Duration) -> Self {
            self.ticker_delay = delay;
            self
        }

        fn with_refresh_metadata_fail(mut self) -> Self {
            self.refresh_metadata_fail = true;
            self
        }

        fn with_refresh_metadata_delay(mut self, delay: Duration) -> Self {
            self.refresh_metadata_delay = delay;
            self
        }

        fn fr(&self, sym: &str) -> FundingRateData {
            FundingRateData {
                symbol: sym.into(),
                exchange: self.name.into(),
                rate: 0.0001,
                rate_8h: 0.0001,
                predicted_rate: None,
                next_funding_time: 0,
                funding_interval: 8,
                volume_24h: 1_000_000.0,
                timestamp: 0,
                smoothed_rate: None,
                rate_std: None,
                is_outlier: false,
            }
        }
    }

    #[async_trait]
    impl ExchangeAdapter for MockAdapter {
        fn name(&self) -> &'static str {
            self.name
        }

        async fn refresh_metadata(&self) -> ExchangeResult<MetadataRefreshOutcome> {
            self.refresh_metadata_calls.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(self.refresh_metadata_delay).await;
            if self.refresh_metadata_fail {
                Err(ExchangeError::Network("mock metadata fail".into()))
            } else {
                Ok(MetadataRefreshOutcome::Refreshed)
            }
        }

        async fn get_funding_rate(&self, symbol: &str) -> ExchangeResult<FundingRateData> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                Err(ExchangeError::Network("mock fail".into()))
            } else {
                Ok(self.fr(symbol))
            }
        }

        async fn get_funding_rates(
            &self,
            _symbols: Option<&[String]>,
        ) -> ExchangeResult<Vec<FundingRateData>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                Err(ExchangeError::Network("mock fail".into()))
            } else {
                Ok(vec![self.fr("BTC"), self.fr("ETH")])
            }
        }

        async fn get_ticker(&self, _symbol: &str) -> ExchangeResult<TickerInfo> {
            Err(ExchangeError::NotImplemented("mock"))
        }

        async fn get_tickers(
            &self,
            _symbols: Option<&[String]>,
        ) -> ExchangeResult<Vec<TickerInfo>> {
            tokio::time::sleep(self.ticker_delay).await;
            if self.fail {
                Err(ExchangeError::Network("mock fail".into()))
            } else {
                Ok(vec![TickerInfo {
                    symbol: "BTC".into(),
                    exchange: self.name.into(),
                    bid: 100.0,
                    ask: 101.0,
                    last: 100.5,
                    volume_24h: 0.0,
                    timestamp: 0,
                }])
            }
        }

        async fn get_spot_tickers(
            &self,
            _symbols: Option<&[String]>,
        ) -> ExchangeResult<Vec<SpotTick>> {
            if self.fail {
                Err(ExchangeError::Network("mock fail".into()))
            } else {
                Ok(vec![SpotTick {
                    venue: self.name.into(),
                    symbol: "BTC/USDT".into(),
                    bid: Decimal::new(100, 0),
                    ask: Decimal::new(101, 0),
                    last: Decimal::new(1005, 1),
                    bid_size: Some(Decimal::ZERO),
                    ask_size: Some(Decimal::ZERO),
                    volume_24h: Decimal::new(1_000_000, 0),
                    exchange_ts_ms: None,
                    received_at_ms: 0,
                }])
            }
        }

        async fn get_orderbook(&self, _symbol: &str, _depth: u32) -> ExchangeResult<OrderBookInfo> {
            Err(ExchangeError::NotImplemented("mock"))
        }

        fn normalize_symbol(&self, s: &str) -> String {
            s.to_uppercase()
        }

        fn to_exchange_symbol(&self, s: &str) -> String {
            s.to_uppercase()
        }
    }

    #[tokio::test]
    async fn fanout_tolerates_partial_failure() {
        let agg = Aggregator::new();
        agg.register(Arc::new(MockAdapter::new("a", false)));
        agg.register(Arc::new(MockAdapter::new("b", true))); // 故意失败
        agg.register(Arc::new(MockAdapter::new("c", false)));

        let rates = agg.fetch_all_funding_rates_report().await.rows;
        // a + c 各 2 条 = 4 条；b 失败被吞
        assert_eq!(rates.len(), 4);
        let mut exchanges: Vec<_> = rates.iter().map(|r| r.exchange.as_str()).collect();
        exchanges.sort();
        exchanges.dedup();
        assert_eq!(exchanges, vec!["a", "c"]);
    }

    #[tokio::test]
    async fn filtered_fanout_never_calls_disabled_venues() {
        let agg = Aggregator::new();
        let enabled = Arc::new(MockAdapter::new("enabled", false));
        let disabled = Arc::new(MockAdapter::new("disabled", false));
        agg.register(Arc::clone(&enabled) as Arc<dyn ExchangeAdapter>);
        agg.register(Arc::clone(&disabled) as Arc<dyn ExchangeAdapter>);

        let report = agg
            .fetch_funding_rates_report_for(&["enabled".to_owned()])
            .await;

        assert_eq!(report.rows.len(), 2);
        assert_eq!(report.venues.len(), 1);
        assert_eq!(report.venues[0].venue, "enabled");
        assert_eq!(enabled.calls.load(Ordering::SeqCst), 1);
        assert_eq!(disabled.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn names_sorted() {
        let agg = Aggregator::new();
        agg.register(Arc::new(MockAdapter::new("c", false)));
        agg.register(Arc::new(MockAdapter::new("a", false)));
        agg.register(Arc::new(MockAdapter::new("b", false)));
        assert_eq!(agg.names(), vec!["a", "b", "c"]);
    }

    #[tokio::test]
    async fn crossex_route_resolves_shared_family_adapter_only() {
        let agg = Aggregator::new();
        agg.register(Arc::new(MockAdapter::new("gate_crossex", false)));

        assert!(agg.get("gate_crossex:gate").is_some());
        assert!(agg.get("gate_crossex:kraken").is_some());
        assert!(agg.get("hyperliquid:not-configured").is_none());
    }

    #[tokio::test]
    async fn unregister_removes() {
        let agg = Aggregator::new();
        agg.register(Arc::new(MockAdapter::new("a", false)));
        assert_eq!(agg.len(), 1);
        assert!(agg.unregister("a"));
        assert_eq!(agg.len(), 0);
        assert!(!agg.unregister("a")); // 已不存在
    }

    #[tokio::test]
    async fn fetch_all_tickers_aggregates() {
        let agg = Aggregator::new();
        agg.register(Arc::new(MockAdapter::new("a", false)));
        agg.register(Arc::new(MockAdapter::new("b", false)));
        let tickers = agg.fetch_all_tickers_report().await.rows;
        assert_eq!(tickers.len(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn fetch_all_tickers_times_out_slow_venue() {
        let agg = Aggregator::new();
        agg.register(Arc::new(MockAdapter::new("binance", false)));
        agg.register(Arc::new(
            MockAdapter::new("hyperliquid", false).with_ticker_delay(Duration::from_secs(16)),
        ));

        let tickers = agg.fetch_all_tickers_report().await.rows;

        assert_eq!(tickers.len(), 1);
        assert_eq!(tickers[0].exchange, "binance");
    }

    #[tokio::test(start_paused = true)]
    async fn ticker_report_keeps_timeout_outcome() {
        let agg = Aggregator::new();
        agg.register(Arc::new(MockAdapter::new("binance", false)));
        agg.register(Arc::new(
            MockAdapter::new("hyperliquid", false).with_ticker_delay(Duration::from_secs(16)),
        ));

        let report = agg.fetch_all_tickers_report().await;

        assert_eq!(report.rows.len(), 1);
        assert_eq!(report.venues.len(), 2);
        let timed_out = report
            .venues
            .iter()
            .find(|outcome| outcome.venue == "hyperliquid")
            .expect("hyperliquid outcome");
        assert_eq!(timed_out.rows, 0);
        assert!(matches!(
            timed_out.error.as_ref(),
            Some(ExchangeError::Timeout { seconds: 15 })
        ));
        assert_eq!(
            timed_out
                .problem
                .as_ref()
                .map(|problem| (problem.venue.as_str(), problem.operation.as_str())),
            Some(("hyperliquid", FANOUT_OP_PERP_TICKERS))
        );
    }

    #[tokio::test]
    async fn fetch_all_spot_ticks_aggregates() {
        let agg = Aggregator::new();
        agg.register(Arc::new(MockAdapter::new("a", false)));
        agg.register(Arc::new(MockAdapter::new("b", false)));
        agg.register(Arc::new(MockAdapter::new("c", true)));
        let ticks = agg.fetch_all_spot_ticks_report().await.rows;
        assert_eq!(ticks.len(), 2);
        assert!(ticks.iter().all(|tick| tick.symbol == "BTC/USDT"));
    }

    #[test]
    fn flatten_with_capacity_preserves_items() {
        let rows = flatten_with_capacity(vec![vec![1, 2], Vec::new(), vec![3]]);
        assert_eq!(rows, vec![1, 2, 3]);
        assert_eq!(rows.capacity(), 3);
    }

    #[test]
    fn fanout_timeout_uses_venue_defaults() {
        assert_eq!(fanout_timeout("okx"), Duration::from_secs(30));
        assert_eq!(fanout_timeout("hyperliquid:xyz"), Duration::from_secs(15));
        assert_eq!(fanout_timeout("kucoin"), Duration::from_secs(20));
        assert_eq!(fanout_timeout("binance"), Duration::from_secs(10));
        assert_eq!(fanout_timeout("unknown"), Duration::from_secs(10));
    }

    #[tokio::test]
    async fn fanout_scheduler_caps_parallel_exchange_calls() {
        let aggregator = Aggregator::new();
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let jobs = (0..FANOUT_MAX_CONCURRENCY * 3).map(|_| {
            let aggregator = &aggregator;
            let active = Arc::clone(&active);
            let peak = Arc::clone(&peak);
            async move {
                let _permit = aggregator
                    .acquire_fanout_permit()
                    .await
                    .expect("test fanout scheduler remains open");
                let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(current, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(10)).await;
                active.fetch_sub(1, Ordering::SeqCst);
            }
        });

        join_all(jobs).await;

        assert_eq!(active.load(Ordering::SeqCst), 0);
        assert_eq!(peak.load(Ordering::SeqCst), FANOUT_MAX_CONCURRENCY);
    }

    /// PR-DP-04: 所有 venue `refresh_metadata` 都成功时，结果列表全部 Ok。
    #[tokio::test]
    async fn refresh_all_metadata_returns_ok_for_each_venue() {
        let agg = Aggregator::new();
        let a = Arc::new(MockAdapter::new("a", false));
        let b = Arc::new(MockAdapter::new("b", false));
        agg.register(Arc::clone(&a) as Arc<dyn ExchangeAdapter>);
        agg.register(Arc::clone(&b) as Arc<dyn ExchangeAdapter>);

        let outcomes = agg.refresh_all_metadata().await;

        assert_eq!(outcomes.len(), 2);
        assert!(outcomes.iter().all(|(_, result)| result.is_ok()));
        assert_eq!(a.refresh_metadata_calls.load(Ordering::SeqCst), 1);
        assert_eq!(b.refresh_metadata_calls.load(Ordering::SeqCst), 1);
    }

    /// PR-DP-04: 单家 `refresh_metadata` 失败不阻塞其他 venue。
    #[tokio::test]
    async fn refresh_all_metadata_isolates_failures() {
        let agg = Aggregator::new();
        agg.register(Arc::new(MockAdapter::new("a", false)));
        agg.register(Arc::new(
            MockAdapter::new("b", false).with_refresh_metadata_fail(),
        ));
        agg.register(Arc::new(MockAdapter::new("c", false)));

        let outcomes = agg.refresh_all_metadata().await;

        assert_eq!(outcomes.len(), 3);
        let failures: Vec<&str> = outcomes
            .iter()
            .filter(|(_, result)| result.is_err())
            .map(|(name, _)| name.as_str())
            .collect();
        assert_eq!(failures, vec!["b"]);
    }

    /// PR-DP-04: 慢 venue 触发 `fanout_timeout` 后返回 Err，其他 venue 正常 Ok。
    #[tokio::test(start_paused = true)]
    async fn refresh_all_metadata_times_out_slow_venue() {
        let agg = Aggregator::new();
        agg.register(Arc::new(MockAdapter::new("binance", false)));
        // Hyperliquid 默认 fanout_timeout 为 15s；暂停时钟下模拟 16s 阻塞。
        agg.register(Arc::new(
            MockAdapter::new("hyperliquid", false)
                .with_refresh_metadata_delay(Duration::from_secs(16)),
        ));

        let outcomes = agg.refresh_all_metadata().await;

        assert_eq!(outcomes.len(), 2);
        let by_venue: std::collections::HashMap<_, _> = outcomes
            .iter()
            .map(|(name, result)| (name.as_str(), result.is_ok()))
            .collect();
        assert_eq!(by_venue.get("binance"), Some(&true));
        assert_eq!(by_venue.get("hyperliquid"), Some(&false));
    }

    /// PR-AN: 报告携带 per-venue 真实延迟与归一化覆盖率。
    /// 超时 venue 仍记录其等待延迟，不被算作 covered。
    #[tokio::test(start_paused = true)]
    async fn report_records_latency_and_coverage() {
        let agg = Aggregator::new();
        agg.register(Arc::new(MockAdapter::new("binance", false)));
        // okx fanout_timeout=30s，2s 延迟仍成功，延迟应被记录。
        agg.register(Arc::new(
            MockAdapter::new("okx", false).with_ticker_delay(Duration::from_secs(2)),
        ));
        // Hyperliquid fanout_timeout=15s，暂停时钟下 16s 延迟 → 超时失败。
        agg.register(Arc::new(
            MockAdapter::new("hyperliquid", false).with_ticker_delay(Duration::from_secs(16)),
        ));

        let report = agg.fetch_all_tickers_report().await;

        assert_eq!(report.total_venues(), 3);
        assert_eq!(report.succeeded_venues(), 2);
        assert_eq!(report.failed_venues(), 1);
        assert_eq!(report.coverage(), MarketDataCoverage::new(3, 2));

        let okx = report
            .venues
            .iter()
            .find(|venue| venue.venue == "okx")
            .expect("okx outcome");
        assert!(okx.succeeded());
        assert!(okx.latency_ms >= 2_000, "okx latency_ms={}", okx.latency_ms);

        let hl = report
            .venues
            .iter()
            .find(|venue| venue.venue == "hyperliquid")
            .expect("hyperliquid outcome");
        assert!(!hl.succeeded());
        assert!(hl.latency_ms >= 15_000, "hl latency_ms={}", hl.latency_ms);
    }

    /// PR-AN: 空 fanout 报告 fail-closed 返回 0% coverage，而非误导性的 100%。
    #[tokio::test]
    async fn empty_fanout_reports_zero_coverage() {
        let agg = Aggregator::new();
        let report = agg.fetch_all_tickers_report().await;
        assert_eq!(report.total_venues(), 0);
        assert_eq!(report.succeeded_venues(), 0);
        assert_eq!(report.coverage(), MarketDataCoverage::new(0, 0));
    }

    #[test]
    fn successful_empty_venue_remains_uncovered() {
        let report = FanoutReport::<()> {
            rows: Vec::new(),
            venues: vec![FanoutVenueResult {
                venue: "binance".to_owned(),
                operation: FANOUT_OP_PERP_TICKERS,
                rows: 0,
                latency_ms: 3,
                error: None,
                problem: None,
            }],
        };

        assert_eq!(report.succeeded_venues(), 1);
        assert_eq!(report.covered_venues(), 0);
        assert_eq!(report.coverage(), MarketDataCoverage::new(1, 0));
    }
}
