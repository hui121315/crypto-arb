mod account;
mod calendar;
mod chain_cost;
mod chain_execution;
mod execution;
mod submission;
mod comparison;
mod peers;
mod peer_preflight;
mod peer_funding;
mod peer_order;
mod peer_plan;
mod peer_plan_store;
mod peer_execution;
mod peer_recovery;
mod peer_conversion;
mod peer_inventory;
mod peer_native_topup;
mod peer_settlement;
mod context;
pub(crate) mod credentials;
mod monitor;
mod batch;
mod order_compile;
mod order_protocol;
mod orders;
mod plan_store;
mod plans;
mod plan_build;
mod settlement;
mod native_topup;
mod recovery;
mod preflight;
mod funding;
mod stablecoin;
mod stablecoin_store;
mod exchange_conversion;
mod funding_plan;
mod funding_store;
mod funding_followup;
mod funding_withdrawal;
mod funding_transfer;
pub(crate) mod protocol;
mod rfq;
mod rfq_acceptance;
mod rfq_history;
mod rfq_protocol;
mod rfq_runtime;
mod rfq_store;
mod runtime;

use exchange::http::HttpClient;
use parking_lot::{Mutex, RwLock};
use shared_types::stocks::*;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::{sync::Arc, time::Duration};
use tokio::task::JoinHandle;

const CATALOG_TTL_MS: i64 = 300_000;
const ROOT: &str = "https://api.backpack.exchange";

pub(crate) struct BackpackStocks {
    peer_sources: Option<(Arc<super::instrument_registry::InstrumentRegistry>, Arc<super::market_data::MarketDataCache>)>,
    peer_feed: Option<(Arc<exchange::Aggregator>,Arc<super::market_subscriptions::MarketSubscriptions>)>,
    http: HttpClient,
    root: String,
    ws_url: String,
    catalog: RwLock<Option<StockCatalog>>,
    catalog_lock: tokio::sync::Mutex<()>,
    watch_lock: tokio::sync::Mutex<()>,
    snapshot: RwLock<StockMarketSnapshot>,
    worker: Mutex<Option<JoinHandle<()>>>,
    quote_lock: tokio::sync::Mutex<()>,
    generation: AtomicU64,
    context_lock: tokio::sync::Mutex<()>,
    calendar: RwLock<Option<calendar::Calendar>>,
    monitor_worker: Mutex<Option<JoinHandle<()>>>,
    batch: RwLock<StockBatchStatus>,
    batch_generation: AtomicU64,
    batch_worker: Mutex<Option<JoinHandle<()>>>,
    quote_source: super::onchain_comparison::stock_quotes::Source,
    context_error: RwLock<Option<String>>,
    rfq_store: rfq_store::RfqStore,
    rfq_lock: tokio::sync::Mutex<()>,
    rfq_state_lock: Mutex<()>,
    rfq_worker: Mutex<Option<JoinHandle<()>>>,
    rfq_subscription: tokio::sync::watch::Sender<Option<String>>,
    rfq_problem: RwLock<Option<String>>,
    account: RwLock<account::AccountCache>,
    account_lock: tokio::sync::Mutex<()>,
    account_tracking_until_ms: AtomicI64,
    account_subscription: tokio::sync::watch::Sender<Option<String>>,
    order_subscription: tokio::sync::watch::Sender<Option<String>>,
    order_tracking_until_ms: AtomicI64,
    order_lock: tokio::sync::Mutex<()>,
    chain_lock: tokio::sync::Mutex<()>,
    submission_lock: Arc<tokio::sync::Mutex<()>>,
    preflight_lock: tokio::sync::Mutex<()>,
    credential_loader: fn() -> Result<credentials::Credentials, String>,
    wallet_claims: Arc<super::onchain_wallet_claims::WalletClaims>,
    plan_store: plan_store::PlanStore,
    peer_plan_store: peer_plan_store::Store,
    peer_receipt_worker: Mutex<Option<(String, JoinHandle<()>)>>,
    funding_store: funding_store::FundingStore,
    funding_worker: Mutex<Option<JoinHandle<()>>>,
    stablecoin_store: stablecoin_store::StablecoinStore,
    stablecoin_preview: RwLock<Option<(u64, StockStablecoinPreview)>>,
    exchange_conversion_store: exchange_conversion::store::Store,
    webhook: Option<Arc<webhook::WebhookDispatcher>>,
    alert_worker: Mutex<Option<JoinHandle<()>>>,
}

impl BackpackStocks {
    pub(crate) fn new() -> anyhow::Result<Self> {
        Ok(Self {
            peer_sources: None,
            peer_feed: None,
            http: HttpClient::builder("backpack")
                .timeout_secs(12)
                .connect_timeout_secs(5)
                .max_retries(0)
                .build()?,
            root: ROOT.into(),
            ws_url: "wss://ws.backpack.exchange".into(),
            catalog: RwLock::new(None),
            catalog_lock: tokio::sync::Mutex::new(()),
            watch_lock: tokio::sync::Mutex::new(()),
            snapshot: RwLock::new(StockMarketSnapshot {
                monitor: StockMonitorStatus { revision: uuid::Uuid::new_v4().to_string(), ..Default::default() },
                observed_at_ms: common::time::now_ms(),
                ..Default::default()
            }),
            worker: Mutex::new(None),
            quote_lock: tokio::sync::Mutex::new(()),
            generation: AtomicU64::new(0),
            context_lock: tokio::sync::Mutex::new(()),
            calendar: RwLock::new(None),
            monitor_worker: Mutex::new(None),
            batch: RwLock::new(StockBatchStatus {
                revision: uuid::Uuid::new_v4().to_string(),
                ..Default::default()
            }),
            batch_generation: AtomicU64::new(0),
            batch_worker: Mutex::new(None),
            quote_source: Default::default(),
            context_error: RwLock::new(None),
            rfq_store: rfq_store::RfqStore::load(None),
            rfq_lock: tokio::sync::Mutex::new(()),
            rfq_state_lock: Mutex::new(()),
            rfq_worker: Mutex::new(None),
            rfq_subscription: tokio::sync::watch::channel(None).0,
            rfq_problem: RwLock::new(None),
            account: RwLock::new(account::AccountCache::default()),
            account_lock: tokio::sync::Mutex::new(()),
            account_tracking_until_ms: AtomicI64::new(0),
            account_subscription: tokio::sync::watch::channel(None).0,
            order_subscription: tokio::sync::watch::channel(None).0,
            order_tracking_until_ms: AtomicI64::new(0),
            order_lock: tokio::sync::Mutex::new(()),
            chain_lock: tokio::sync::Mutex::new(()),
            submission_lock: Arc::new(tokio::sync::Mutex::new(())),
            preflight_lock: tokio::sync::Mutex::new(()),
            credential_loader: credentials::Credentials::load,
            wallet_claims: Default::default(),
            plan_store: plan_store::PlanStore::load(None, Default::default()),
            peer_plan_store: peer_plan_store::Store::load(None, Default::default()),
            peer_receipt_worker: Mutex::new(None),
            funding_store: funding_store::FundingStore::load(None, Default::default()),
            funding_worker: Mutex::new(None),
            stablecoin_store: stablecoin_store::StablecoinStore::load(None, Default::default()),
            stablecoin_preview: RwLock::new(None),
            exchange_conversion_store: exchange_conversion::store::Store::load(None, Default::default()),
            webhook: None,
            alert_worker: Mutex::new(None),
        })
    }

    pub(crate) fn with_wallet_claims(
        mut self,
        claims: Arc<super::onchain_wallet_claims::WalletClaims>,
    ) -> Self {
        self.wallet_claims = claims;
        self
    }

    #[cfg(test)]
    pub(crate) fn with_public_fixture(mut self, root: &str) -> Self {
        assert!(root.starts_with("http://127.0.0.1:"));
        self.root = root.into();
        self.ws_url = format!("{}/ws", root.replacen("http:", "ws:", 1));
        self.quote_source = super::onchain_comparison::stock_quotes::Source::fixture(root);
        self.credential_loader = || Err("private accounts are disabled in this fixture".into());
        self
    }

    pub(crate) fn with_webhook(mut self, webhook: Arc<webhook::WebhookDispatcher>) -> Self {
        self.webhook = Some(webhook);
        self
    }

    fn background_monitoring(&self) -> bool {
        let snapshot = self.snapshot.read();
        snapshot.monitor.enabled
            && snapshot.monitor.alerts.enabled
            && self.webhook.as_ref().is_some_and(|w| {
                w.durable_outbox() && w.enabled_for(shared_types::WebhookEventKind::StockSpread)
            })
    }

    async fn read(&self, path: &str) -> Result<Vec<u8>, String> {
        let url = format!("{}{path}", self.root);
        tokio::time::timeout(Duration::from_secs(15), async {
            let mut response = self
                .http
                .execute_once(|| self.http.request(reqwest::Method::GET, &url))
                .await
                .map_err(|_| format!("Backpack {path} 读取失败"))?;
            let mut bytes = Vec::new();
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|_| format!("Backpack {path} 响应未完整收到"))?
            {
                if bytes.len() + chunk.len() > 8 * 1024 * 1024 {
                    return Err("Backpack 元数据响应过大".into());
                }
                bytes.extend_from_slice(&chunk);
            }
            Ok(bytes)
        })
        .await
        .map_err(|_| format!("Backpack {path} 查询超时"))?
    }

    pub(crate) async fn catalog(&self) -> Result<StockCatalog, String> {
        let _guard = self.catalog_lock.lock().await;
        let now = common::time::now_ms();
        if let Some(cached) = self
            .catalog
            .read()
            .as_ref()
            .filter(|c| now >= c.observed_at_ms && now - c.observed_at_ms < CATALOG_TTL_MS)
        {
            return Ok(cached.clone());
        }
        let securities = self.read("/api/v1/securities").await?;
        let markets = self.read("/api/v1/markets").await?;
        let catalog = protocol::catalog(&securities, &markets, common::time::now_ms())?;
        *self.catalog.write() = Some(catalog.clone());
        Ok(catalog)
    }

    pub(crate) fn snapshot(&self) -> StockMarketSnapshot {
        let mut snapshot = self.snapshot.read().clone();
        snapshot.batch = self.batch.read().clone();
        snapshot.rfqs = self.visible_rfqs();
        snapshot.rfq_connected = self.rfq_subscription.borrow().is_some();
        snapshot.rfq_problem = self
            .rfq_store
            .problem()
            .or_else(|| self.rfq_problem.read().clone());
        snapshot.plans = self.plan_store.records();
        snapshot.peer_plans = self.peer_plan_store.records();
        snapshot.peer_accounting = snapshot.peer_plans.iter()
            .filter(|p| p.cex_order.is_some())
            .map(StockPeerPlan::accounting).collect();
        snapshot.peer_plan_problem = self.peer_plan_store.problem();
        snapshot.funding_plans = self.funding_store.records();
        snapshot.funding_problem = self.funding_store.problem();
        snapshot.stablecoin_plans = self.stablecoin_store.records();
        snapshot.stablecoin_problem = self.stablecoin_store.problem();
        snapshot.exchange_conversions = self.exchange_conversion_store.rows();
        snapshot.exchange_conversions.truncate(48);
        snapshot.exchange_conversion_problem = self.exchange_conversion_store.problem();
        snapshot.claimed_conversion_cost_ids = self.plan_store.claimed_conversion_cost_ids(common::time::now_ms());
        snapshot.plan_problem = self.plan_store.problem();
        snapshot
    }

    pub(crate) fn review_plans(&self, id: Option<&str>) -> (Vec<StockExecutionPlan>, Option<String>) {
        (self.plan_store.review_records(id), self.plan_store.problem())
    }

    pub(crate) fn review_peer_plans(&self, id: Option<&str>) -> (Vec<StockPeerPlan>, Option<String>) {
        (self.peer_plan_store.review_records(id), self.peer_plan_store.problem())
    }

    #[cfg(test)]
    pub(crate) fn seed_review_records(&self, plan: StockExecutionPlan, peer: StockPeerPlan) {
        self.plan_store.seed_review_record(plan);
        self.peer_plan_store.seed_review_record(peer);
    }

    pub(crate) fn with_rfq_store(mut self, path: std::path::PathBuf) -> Self {
        self.rfq_store = rfq_store::RfqStore::load(Some(path));
        self
    }

    pub(crate) async fn watch(
        self: &Arc<Self>,
        request: StockWatchRequest,
        hub: realtime::WsHub,
    ) -> Result<StockMarketSnapshot, String> {
        let _guard = self.watch_lock.lock().await;
        let Some(asset) = request.asset.filter(|s| !s.trim().is_empty()) else {
            self.generation.fetch_add(1, Ordering::SeqCst);
            let mut snapshot = self.snapshot.write();
            *snapshot = StockMarketSnapshot {
                monitor: StockMonitorStatus { revision: uuid::Uuid::new_v4().to_string(), ..Default::default() },
                observed_at_ms: common::time::now_ms()
                    .max(snapshot.observed_at_ms.saturating_add(1)),
                ..Default::default()
            };
            drop(snapshot);
            self.publish(&hub);
            return Ok(self.snapshot());
        };
        let reusable = {
            let snapshot = self.snapshot.read();
            let now = common::time::now_ms();
            snapshot.security.as_ref().is_some_and(|s| s.asset == asset)
                && snapshot
                    .token_metadata_at_ms
                    .is_some_and(|t| now >= t && now - t < CATALOG_TTL_MS)
        };
        if reusable {
            self.ensure_started(hub);
            return Ok(self.snapshot());
        }
        let security = self
            .catalog()
            .await?
            .rows
            .into_iter()
            .find(|s| s.asset == asset)
            .ok_or("所选股票不在官方证券目录中")?;
        let (tokens, funding_assets, problem, token_metadata_at_ms) = match self
            .read("/api/v1/assets")
            .await
            .and_then(|b| protocol::asset_context(&b, &asset))
        {
            Ok((tokens, funding)) => (tokens, funding, None, Some(common::time::now_ms())),
            Err(error) => (vec![], vec![], Some(error), None),
        };
        self.generation.fetch_add(1, Ordering::SeqCst);
        let mut snapshot = self.snapshot.write();
        *snapshot = StockMarketSnapshot {
            monitor: StockMonitorStatus { revision: uuid::Uuid::new_v4().to_string(), ..Default::default() },
            security: Some(security),
            tokens,
            funding_assets,
            token_metadata_at_ms,
            token_metadata_problem: problem,
            observed_at_ms: common::time::now_ms().max(snapshot.observed_at_ms.saturating_add(1)),
            ..Default::default()
        };
        drop(snapshot);
        self.ensure_started(hub.clone());
        self.publish(&hub);
        Ok(self.snapshot())
    }

    pub(crate) fn ensure_started(self: &Arc<Self>, hub: realtime::WsHub) {
        self.resume_peer_receipts(&hub);
        {
            let needed = alerts::needs_worker(&self.snapshot.read(), common::time::now_ms());
            let mut worker = self.alert_worker.lock();
            if needed {
                if worker.as_ref().is_none_or(|h| h.is_finished()) {
                    *worker = Some(tokio::spawn(alerts::run(Arc::downgrade(self), hub.clone())));
                }
            } else {
                if let Some(worker) = worker.take() {
                    worker.abort();
                }
                self.snapshot.write().alerts.phase = StockAlertPhase::Disabled;
            }
        }
        {
            let mut worker = self.monitor_worker.lock();
            if worker.as_ref().is_none_or(|h| h.is_finished()) {
                *worker = Some(tokio::spawn(monitor::run(
                    Arc::downgrade(self),
                    hub.clone(),
                )));
            }
        }
        let mut worker = self.worker.lock();
        if worker.as_ref().is_some_and(|h| !h.is_finished()) {
            return;
        }
        *worker = Some(tokio::spawn(runtime::run(
            Arc::downgrade(self),
            hub,
            self.ws_url.clone(),
        )));
    }

    fn publish(&self, hub: &realtime::WsHub) {
        if let Ok(message) = realtime::WsMessage::json(&self.snapshot()) {
            hub.publish_throttled(realtime::channels::STOCKS, message);
        }
    }
}

impl Drop for BackpackStocks {
    fn drop(&mut self) {
        if let Some(handle) = self.batch_worker.get_mut().take() { handle.abort(); }
        if let Some((_, handle)) = self.peer_receipt_worker.get_mut().take() { handle.abort(); }
        if let Some(handle) = self.funding_worker.get_mut().take() {
            handle.abort();
        }
        if let Some(handle) = self.worker.get_mut().take() {
            handle.abort();
        }
        if let Some(handle) = self.monitor_worker.get_mut().take() {
            handle.abort();
        }
        if let Some(handle) = self.rfq_worker.get_mut().take() {
            handle.abort();
        }
        if let Some(handle) = self.alert_worker.get_mut().take() {
            handle.abort();
        }
    }
}

mod alerts;
#[cfg(test)]
mod rfq_tests;
#[cfg(test)]
mod tests;
