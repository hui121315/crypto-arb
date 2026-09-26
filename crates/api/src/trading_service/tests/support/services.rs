use super::super::adapters::{BalanceProbeAdapter, ReconcileTestAdapter};
use super::super::submit_rate_limit_adapter::SubmitRateLimitAdapter;
use super::*;

pub(in crate::trading_service::tests) fn service_with_reconcile_adapter(
    open_orders: Vec<OrderInfo>,
    order: Option<OrderInfo>,
) -> TradingService {
    service_with_reconcile_adapter_handle(open_orders, order).0
}

pub(in crate::trading_service::tests) fn service_with_reconcile_adapter_handle(
    open_orders: Vec<OrderInfo>,
    order: Option<OrderInfo>,
) -> (TradingService, Arc<ReconcileTestAdapter>) {
    let adapter = Arc::new(ReconcileTestAdapter::new(open_orders, order));
    service_from_reconcile_adapter(adapter)
}

pub(in crate::trading_service::tests) fn service_from_reconcile_adapter(
    adapter: Arc<ReconcileTestAdapter>,
) -> (TradingService, Arc<ReconcileTestAdapter>) {
    let journal = Arc::new(OrderJournal::new());
    let risk = RiskEngine::new(RiskConfig::default());
    let adapter_for_engine = Arc::clone(&adapter);
    let engine_adapter: Arc<dyn LiveTradingAdapter> = adapter_for_engine;
    let engine = Arc::new(ExecutionEngine::new(
        engine_adapter,
        risk.clone(),
        Arc::clone(&journal),
    ));
    (
        test_service(engine, journal, risk, "reconcile_test"),
        adapter,
    )
}

pub(in crate::trading_service::tests) fn service_with_reconcile_adapter_error(
    open_orders: Vec<OrderInfo>,
    error: &str,
) -> TradingService {
    service_from_reconcile_adapter(Arc::new(ReconcileTestAdapter::with_order_error(
        open_orders,
        error,
    )))
    .0
}

pub(in crate::trading_service::tests) fn service_with_submit_timeout(
    order: Option<OrderInfo>,
) -> (TradingService, Arc<ReconcileTestAdapter>) {
    service_from_reconcile_adapter(Arc::new(ReconcileTestAdapter::with_place_timeout(
        order, 10,
    )))
}

pub(in crate::trading_service::tests) fn service_with_open_orders_error(
    order: Option<OrderInfo>,
    error: &str,
) -> (TradingService, Arc<ReconcileTestAdapter>) {
    service_from_reconcile_adapter(Arc::new(ReconcileTestAdapter::with_open_orders_error(
        order, error,
    )))
}

pub(in crate::trading_service::tests) fn service_with_balance_probe_adapter(
) -> (TradingService, Arc<BalanceProbeAdapter>) {
    service_from_balance_probe_adapter(Arc::new(BalanceProbeAdapter::new()))
}

pub(in crate::trading_service::tests) fn service_with_slow_balance_probe_adapter(
    delay_ms: u64,
) -> (TradingService, Arc<BalanceProbeAdapter>) {
    service_from_balance_probe_adapter(Arc::new(BalanceProbeAdapter::with_delay(delay_ms)))
}

pub(in crate::trading_service::tests) fn service_with_full_rate_limited_balance_probe_adapter(
    delay_ms: u64,
    retry_after_secs: u64,
) -> (TradingService, Arc<BalanceProbeAdapter>) {
    service_from_balance_probe_adapter(Arc::new(BalanceProbeAdapter::with_full_rate_limit(
        delay_ms,
        retry_after_secs,
    )))
}

pub(in crate::trading_service::tests) fn service_with_submit_rate_limit_adapter(
    retry_after_secs: u64,
) -> (TradingService, Arc<SubmitRateLimitAdapter>) {
    let adapter = Arc::new(SubmitRateLimitAdapter::new(retry_after_secs));
    let journal = Arc::new(OrderJournal::new());
    let risk = RiskEngine::new(RiskConfig::default());
    let adapter_for_engine = Arc::clone(&adapter);
    let engine_adapter: Arc<dyn LiveTradingAdapter> = adapter_for_engine;
    let engine = Arc::new(ExecutionEngine::new(
        engine_adapter,
        risk.clone(),
        Arc::clone(&journal),
    ));
    (
        test_service(engine, journal, risk, "submit_rate_limit"),
        adapter,
    )
}

pub(in crate::trading_service::tests) fn service_from_balance_probe_adapter(
    adapter: Arc<BalanceProbeAdapter>,
) -> (TradingService, Arc<BalanceProbeAdapter>) {
    let journal = Arc::new(OrderJournal::new());
    let risk = RiskEngine::new(RiskConfig::default());
    let adapter_for_engine = Arc::clone(&adapter);
    let engine_adapter: Arc<dyn LiveTradingAdapter> = adapter_for_engine;
    let engine = Arc::new(ExecutionEngine::new(
        engine_adapter,
        risk.clone(),
        Arc::clone(&journal),
    ));
    (
        test_service(engine, journal, risk, "balance_probe"),
        adapter,
    )
}

fn test_service(
    engine: Arc<ExecutionEngine>,
    journal: Arc<OrderJournal>,
    risk: RiskEngine,
    adapter_name: &'static str,
) -> TradingService {
    TradingService {
        engine,
        journal,
        risk,
        live_order_proof_health: Arc::new(
            crate::services::live_order_proof_health::LiveOrderProofHealthStore::default(),
        ),
        adapter_name: RwLock::new(adapter_name),
        account_reader: ArcSwapOption::empty(),
        private_ws_accounts: RwLock::new(HashMap::new()),
        account_cache_epoch: AtomicU64::new(0),
        balance_cache: VenueBalanceCache::new(BALANCE_CACHE_TTL_MS, BALANCE_CACHE_MAX_STALE_MS),
        account_summaries: DashMap::new(),
        asset_valuations: DashMap::new(),
        account_evidence_refresh_after_ms: DashMap::new(),
        balance_fetch_locks: DashMap::new(),
        balance_fetch_backoffs: DashMap::new(),
        open_order_fetch_lock: tokio::sync::Mutex::new(()),
        open_order_fetch_backoffs: DashMap::new(),
        open_order_cache: VenueOpenOrderCache::new(
            OPEN_ORDER_CACHE_TTL_MS,
            OPEN_ORDER_CACHE_MAX_STALE_MS,
        ),
        position_fetch_lock: tokio::sync::Mutex::new(()),
        position_fetch_backoffs: DashMap::new(),
        position_cache: VenuePositionCache::new(POSITION_CACHE_TTL_MS, POSITION_CACHE_MAX_STALE_MS),
        route_failures: Arc::new(RouteFailureSink::default()),
        latest_funding_payment_ingest: ArcSwapOption::empty(),
    }
}
