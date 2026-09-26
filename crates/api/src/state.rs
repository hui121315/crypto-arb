//! 应用全局状态。
//!
//! M10 接入：交易所聚合器、套利引擎、自动刷新快照、WsHub 等共享单例。

#[cfg(test)]
mod stock_plan_tests;

use crate::lifecycle::nav_persist;
use crate::metrics::Metrics;
use crate::middleware::audit;
use crate::services::close_run_store::CloseRunStore;
use crate::services::execution_run_store::ExecutionRunStore;
use crate::services::fees::TradeFeeCache;
use crate::services::gate_crossex_mode::GateCrossExModeService;
use crate::services::instrument_registry::InstrumentRegistry;
use crate::services::live_order_proof_health::LiveOrderProofHealthStore;
use crate::services::market_data::MarketDataCache;
use crate::services::market_subscriptions::MarketSubscriptions;
use crate::services::onchain_comparison_config::{
    OnchainComparisonConfigReplay, OnchainComparisonConfigStore,
};
use crate::services::onchain_cross_chain_run_store::OnchainCrossChainRunStore;
use crate::services::onchain_execution_build_store::OnchainExecutionBuildStore;
use crate::services::onchain_execution_run_store::OnchainExecutionRunStore;
use crate::services::onchain_replenishment_plan_store::OnchainReplenishmentPlanStore;
use crate::services::onchain_token_approval_store::OnchainTokenApprovalStore;
use crate::services::onchain_token_approval_run_store::OnchainTokenApprovalRunStore;
use crate::services::opportunity_index::OpportunityIndex;
use crate::services::portfolio_pnl::PortfolioPnlSnapshot;
use crate::services::private_account_refresh::PrivateAccountRefreshQueue;
use crate::services::private_ws_health::PrivateWsHealthStore;
use crate::services::reconciliation_health::ReconciliationHealthStore;
use crate::services::run_finality_health::RunFinalityHealthStore;
use crate::services::snapshots::{new_funding_diff_stats_snapshot, new_system_health_snapshot};
use crate::services::trading_runtime_config::{
    TradingRuntimeConfigReplay, TradingRuntimeConfigStore,
};
use crate::services::ws_auth::WsTicketStore;
use crate::trading_service::TradingService;
use anyhow::Result;
use arbitrage::ArbitrageEngineV3;
use automation::{AutomationConfigStore, AutomationController};
use common::config::AppConfig;
use dashmap::DashMap;
use exchange::Aggregator;
#[cfg(feature = "legacy-chat")]
use llm::LlmRouter;
use onchain_monitor::OnchainMonitor;
use realtime::{
    AlertRule, HistoryStore, RefreshingSnapshot, VenueQualityTracker, WatchlistAlertReplay,
    WatchlistAlertStore, WatchlistItem, WsHub,
};
use shared_types::{
    ActionRun, ArbitrageConfig, CloseRun, ExecutionRun, FundingDiffStatsRow, HedgePreviewResponse,
    HedgeTicket, MissedOpportunity, OnchainExecutionSubmitResponse, OnchainRpcMode,
    OpportunityScanReport, PortfolioSnapshot, PortfolioSnapshotEnvelope, ReviewRuntimeSnapshot,
    SystemHealth, VenueOpenOrdersEnvelope,
};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, Notify, RwLock};
use webhook::WebhookDispatcher;

type NavHistory = Vec<(i64, f64)>;
type SharedNavHistory = Arc<RwLock<NavHistory>>;

#[derive(Clone)]
pub(crate) struct CachedAccountOpenOrders {
    pub(crate) account_cache_epoch: u64,
    pub(crate) private_ws_change_ms: i64,
    pub(crate) envelope: VenueOpenOrdersEnvelope,
}

#[derive(Clone)]
pub(crate) struct AppState {
    inner: Arc<AppStateInner>,
}

struct AppStateInner {
    config: AppConfig,
    market: MarketRuntimeState,
    trading: TradingRuntimeState,
    product: ProductRuntimeState,
    diagnostics: DiagnosticsRuntimeState,
    storage: StorageRuntimeState,
}

struct MarketRuntimeState {
    backpack_stocks: Arc<crate::services::backpack_stocks::BackpackStocks>,
    aggregator: Arc<Aggregator>,
    funding_diff_stats_snapshot: Arc<RefreshingSnapshot<Vec<FundingDiffStatsRow>>>,
    trade_fee_cache: Arc<TradeFeeCache>,
    market_data: Arc<MarketDataCache>,
    instrument_registry: Arc<InstrumentRegistry>,
    market_subscriptions: Arc<MarketSubscriptions>,
    market_subscription_mutation_lock: Arc<Mutex<()>>,
    gate_crossex_mode: Arc<GateCrossExModeService>,
    gate_crossex_mode_mutation_lock: Arc<Mutex<()>>,
}

struct TradingRuntimeState {
    service: Arc<TradingService>,
    hedge_ticket_hot_cache: Arc<DashMap<String, HedgeTicket>>,
    hedge_preview_hot_cache: Arc<DashMap<String, HedgePreviewResponse>>,
    execution_run_hot_cache: Arc<DashMap<String, ExecutionRun>>,
    execution_run_store: Arc<ExecutionRunStore>,
    close_run_hot_cache: Arc<DashMap<String, CloseRun>>,
    close_run_store: Arc<CloseRunStore>,
    action_run_hot_cache: Arc<DashMap<String, ActionRun>>,
    live_order_proof_health: Arc<LiveOrderProofHealthStore>,
    private_ws_health: Arc<PrivateWsHealthStore>,
    reconciliation_health: Arc<ReconciliationHealthStore>,
    run_finality_health: Arc<RunFinalityHealthStore>,
    sql_ledger_health: trading::SqlLedgerMigrationHealth,
    runtime_config_store: Arc<TradingRuntimeConfigStore>,
    runtime_config_mutation_lock: Arc<Mutex<()>>,
    private_account_refresh_queue: PrivateAccountRefreshQueue,
}

struct ProductRuntimeState {
    arbitrage_engine: Arc<ArbitrageEngineV3>,
    arbitrage_scan_lock: Arc<Mutex<()>>,
    arbitrage_refresh: Arc<Notify>,
    opportunity_index: Arc<OpportunityIndex>,
    automation: Arc<AutomationController>,
    automation_config_store: Arc<AutomationConfigStore>,
    automation_mutation_lock: Arc<Mutex<()>>,
    onchain_monitor: Arc<OnchainMonitor>,
    onchain_config_store: Arc<OnchainComparisonConfigStore>,
    onchain_config_mutation_lock: Arc<Mutex<()>>,
    onchain_execution_builds: Arc<OnchainExecutionBuildStore>,
    onchain_execution_runs: Arc<DashMap<String, OnchainExecutionSubmitResponse>>,
    onchain_execution_run_store: Arc<OnchainExecutionRunStore>,
    onchain_replenishment_plans: Arc<OnchainReplenishmentPlanStore>,
    onchain_cross_chain_runs: Arc<OnchainCrossChainRunStore>,
    onchain_token_approval_builds: Arc<OnchainTokenApprovalStore>,
    onchain_token_approval_runs: Arc<OnchainTokenApprovalRunStore>,
    webhook: Arc<WebhookDispatcher>,
    webhook_config_mutation_lock: Arc<Mutex<()>>,
    missed_opportunity_hot_cache: Arc<DashMap<String, MissedOpportunity>>,
    watchlist: Arc<RwLock<Vec<WatchlistItem>>>,
    alert_rules: Arc<RwLock<Vec<AlertRule>>>,
    alert_cooldowns: Arc<DashMap<i64, i64>>,
    watchlist_alert_mutation_lock: Arc<Mutex<()>>,
    portfolio_nav_history: SharedNavHistory,
    portfolio_pnl_snapshot: Arc<RefreshingSnapshot<PortfolioPnlSnapshot>>,
    portfolio_pnl_refresh: Arc<Notify>,
    portfolio_snapshot: Arc<RefreshingSnapshot<PortfolioSnapshot>>,
    portfolio_snapshot_envelope: Arc<RefreshingSnapshot<PortfolioSnapshotEnvelope>>,
    portfolio_refresh: Arc<Notify>,
    review_snapshot: Arc<RefreshingSnapshot<ReviewRuntimeSnapshot>>,
    account_open_orders_snapshot: Arc<RefreshingSnapshot<CachedAccountOpenOrders>>,
    account_open_orders_refresh_lock: Arc<Mutex<()>>,
}

struct DiagnosticsRuntimeState {
    ws_hub: WsHub,
    #[cfg(feature = "legacy-chat")]
    llm_router: Arc<LlmRouter>,
    venue_quality: Arc<VenueQualityTracker>,
    system_health_snapshot: Arc<RefreshingSnapshot<SystemHealth>>,
    ws_tickets: Arc<WsTicketStore>,
    metrics: Arc<Metrics>,
    task_registry: crate::task_registry::TaskRegistry,
}

struct StorageRuntimeState {
    history_store: Arc<HistoryStore>,
    watchlist_alert_store: Arc<WatchlistAlertStore>,
    portfolio_nav_storage_health: Arc<nav_persist::NavStorageHealthStore>,
}

struct RuntimeHealthStores {
    live_order_proof_health: Arc<LiveOrderProofHealthStore>,
    private_ws_health: Arc<PrivateWsHealthStore>,
    reconciliation_health: Arc<ReconciliationHealthStore>,
    run_finality_health: Arc<RunFinalityHealthStore>,
    venue_quality: Arc<VenueQualityTracker>,
}

struct TradingRuntimeInit {
    health: RuntimeHealthStores,
    service: Arc<TradingService>,
    config_store: Arc<TradingRuntimeConfigStore>,
}

struct WatchlistAlertRuntimeState {
    watchlist: Arc<RwLock<Vec<WatchlistItem>>>,
    alert_rules: Arc<RwLock<Vec<AlertRule>>>,
    alert_cooldowns: Arc<DashMap<i64, i64>>,
    store: Arc<WatchlistAlertStore>,
    mutation_lock: Arc<Mutex<()>>,
}

struct OnchainComparisonRuntimeState {
    monitor: Arc<OnchainMonitor>,
    store: Arc<OnchainComparisonConfigStore>,
}

struct RunRuntimeState {
    execution_runs: Arc<DashMap<String, ExecutionRun>>,
    execution_run_store: Arc<ExecutionRunStore>,
    close_runs: Arc<DashMap<String, CloseRun>>,
    close_run_store: Arc<CloseRunStore>,
    action_runs: Arc<DashMap<String, ActionRun>>,
}

fn new_runtime_health_stores(config: &AppConfig) -> RuntimeHealthStores {
    let live_order_replay = LiveOrderProofHealthStore::load_checkpoint(
        live_order_proof_checkpoint_path(config),
        TradingService::configured_account_scope,
    );
    report_live_order_proof_replay(&live_order_replay);
    RuntimeHealthStores {
        live_order_proof_health: Arc::new(live_order_replay.store),
        private_ws_health: Arc::new(PrivateWsHealthStore::default()),
        reconciliation_health: Arc::new(ReconciliationHealthStore::default()),
        run_finality_health: Arc::new(RunFinalityHealthStore::default()),
        venue_quality: Arc::new(VenueQualityTracker::default()),
    }
}

fn init_trading_runtime(
    config: &AppConfig,
    sql_ledger: trading::SqlLedgerInit,
) -> TradingRuntimeInit {
    let runtime_health = new_runtime_health_stores(config);
    let trading_service = Arc::new(
        new_trading_service(config, sql_ledger)
            .with_live_order_proof_health(Arc::clone(&runtime_health.live_order_proof_health)),
    );
    let replay = TradingRuntimeConfigStore::load(trading_runtime_config_path(config));
    restore_trading_runtime(&trading_service, &replay);
    TradingRuntimeInit {
        health: runtime_health,
        service: trading_service,
        config_store: Arc::new(replay.store),
    }
}

fn live_order_proof_checkpoint_path(config: &AppConfig) -> Option<PathBuf> {
    execution_ledger_sidecar_path(config, "live-order-proof.json")
}

fn trading_runtime_config_path(config: &AppConfig) -> Option<PathBuf> {
    execution_ledger_sidecar_path(config, "trading-runtime.json")
}

fn execution_ledger_sidecar_path(config: &AppConfig, extension: &str) -> Option<PathBuf> {
    config
        .storage
        .execution_ledger_path
        .as_deref()
        .map(|value| config.storage.resolve_runtime_path(value))
        .map(|path| path.with_extension(extension))
}

fn report_live_order_proof_replay(
    replay: &crate::services::live_order_proof_health::LiveOrderProofCheckpointReplay,
) {
    if let Some(problem) = replay.problem.as_deref() {
        tracing::warn!(%problem, "live order proof replay failed; current runtime evidence required");
        return;
    }
    report_restored_live_order_proofs(replay.restored);
}

fn report_restored_live_order_proofs(restored: usize) {
    if restored == 0 {
        return;
    }
    tracing::info!(
        restored,
        "credential-bound live order proof replay completed"
    );
}

fn restore_trading_runtime(service: &TradingService, replay: &TradingRuntimeConfigReplay) {
    if let Some(problem) = replay.problem.as_deref() {
        tracing::warn!(%problem, "trading runtime config replay failed; using safe defaults");
        return;
    }
    restore_trading_runtime_snapshot(service, replay.snapshot.as_ref());
}

fn restore_trading_runtime_snapshot(
    service: &TradingService,
    snapshot: Option<&crate::services::trading_runtime_config::TradingRuntimeConfigSnapshot>,
) {
    let Some(snapshot) = snapshot else { return };
    if let Err(problem) = apply_trading_runtime_snapshot(service, snapshot) {
        tracing::warn!(%problem, "trading runtime config restore failed; using safe defaults");
    }
}

fn apply_trading_runtime_snapshot(
    service: &TradingService,
    snapshot: &crate::services::trading_runtime_config::TradingRuntimeConfigSnapshot,
) -> Result<(), String> {
    // Validate the complete checkpoint before activating any execution adapter.
    let mut risk = crate::services::risk_config::restored_config(&snapshot.risk)
        .map_err(|error| error.to_string())?;
    let live = match snapshot.adapter_id.as_str() {
        "mock" => {
            service.select_mock_adapter();
            false
        }
        crate::trading_service::LIVE_ROUTER_ADAPTER_ID => {
            service
                .try_select_adapter(
                    crate::trading_service::LIVE_ROUTER_ADAPTER_ID,
                    crate::services::trading_credentials::current_adapter_credentials(),
                )
                .map_err(|error| error.to_string())?;
            true
        }
        other => return Err(format!("unsupported persisted trading adapter: {other}")),
    };
    risk.live_trading_enabled = live;
    service.update_risk_config(move |current| *current = risk);
    Ok(())
}

#[cfg(test)]
#[test]
fn invalid_runtime_checkpoint_is_validated_before_adapter_selection() {
    let service = TradingService::new_mock();
    let before = crate::services::risk_config::snapshot(&service.risk_config());
    let mut risk = before.clone();
    risk.max_order_notional = 0.0;
    let result = apply_trading_runtime_snapshot(&service,
        &crate::services::trading_runtime_config::TradingRuntimeConfigSnapshot {
            adapter_id: crate::trading_service::LIVE_ROUTER_ADAPTER_ID.to_owned(), risk,
        });
    assert!(result.is_err_and(|message| message.contains("maxOrderNotional")));
    assert_eq!(service.adapter_name(), "mock");
    assert_eq!(crate::services::risk_config::snapshot(&service.risk_config()), before);
}

fn init_run_runtime(
    config: &AppConfig,
    finality_events: &[trading::SqlRunFinalityReplayEvent],
) -> Result<RunRuntimeState> {
    let execution_replay = ExecutionRunStore::load(config);
    let execution_runs = execution_runs_from_replay(execution_replay.runs);
    let close_replay = CloseRunStore::load(config);
    let close_runs = close_runs_from_replay(close_replay.runs);
    apply_sql_run_finality_replay(&execution_runs, &close_runs, finality_events);
    Ok(RunRuntimeState {
        execution_runs,
        execution_run_store: Arc::new(execution_replay.store),
        close_runs,
        close_run_store: Arc::new(close_replay.store),
        action_runs: load_action_runs_from_audit(config)?,
    })
}

impl AppState {
    #[allow(clippy::too_many_lines)]
    pub(crate) async fn new(mut config: AppConfig) -> Result<Self> {
        isolate_default_runtime_paths_for_tests(&mut config);
        #[cfg(not(test))]
        let wallet_claims = crate::services::onchain_wallet_claims::WalletClaims::exclusive(
            &config.storage.resolve_runtime_path("onchain-wallets.lock"),
        )
        .map_err(anyhow::Error::msg)?;
        #[cfg(test)]
        let wallet_claims = Arc::new(crate::services::onchain_wallet_claims::WalletClaims::default());
        let aggregator = Arc::new(Aggregator::new());
        let arbitrage_config = ArbitrageConfig::default();
        let arbitrage_scan_lock = Arc::new(Mutex::new(()));
        let arbitrage_refresh = Arc::new(Notify::new());
        let funding_diff_stats_snapshot = new_funding_diff_stats_snapshot();
        let trade_fee_cache = Arc::new(TradeFeeCache::default());
        let market_data = Arc::new(MarketDataCache::default());
        #[cfg(not(test))]
        let instrument_checkpoint_path = Some(
            config
                .storage
                .resolve_runtime_path("instrument_registry.json"),
        );
        #[cfg(test)]
        let instrument_checkpoint_path = None;
        let instrument_registry =
            Arc::new(InstrumentRegistry::load(instrument_checkpoint_path).await);
        let market_subscriptions = Arc::new(MarketSubscriptions::load(
            config
                .storage
                .market_subscriptions_path
                .as_deref()
                .map(|value| config.storage.resolve_runtime_path(value)),
        ));
        let gate_crossex_mode = Arc::new(GateCrossExModeService::load(
            gate_crossex_mode_config_path(&config),
        ));
        let ws_hub = WsHub::new(1024);
        #[cfg(feature = "legacy-chat")]
        let llm_router = Arc::new(LlmRouter::new());
        let trading_sql_ledger =
            trading::init_sql_ledger_store(storage_postgres_url(&config)).await;
        let trading_sql_ledger_health = trading_sql_ledger.migration_health.clone();
        let hedge_tickets = Arc::new(DashMap::new());
        let hedge_previews = Arc::new(DashMap::new());
        let arbitrage_index = Arc::new(OpportunityIndex::default());
        let run_runtime =
            init_run_runtime(&config, &trading_sql_ledger.replay.run_finality_events)?;
        let webhook_dispatcher = init_webhook_runtime(&config, &run_runtime.execution_runs).await?;
        let missed_opportunities = Arc::new(DashMap::new());
        let trading_runtime = init_trading_runtime(&config, trading_sql_ledger);
        let history_store = init_history_store(&config).await;
        let watchlist_alert_runtime = init_watchlist_alert_runtime(&config).await;
        let portfolio_runtime = init_portfolio_runtime(&config).await;
        let ws_tickets = Arc::new(WsTicketStore::default());
        let metrics = Arc::new(Metrics::new());
        let task_registry = crate::task_registry::TaskRegistry::default();
        let arbitrage_engine = new_arbitrage_engine(
            Arc::clone(&funding_diff_stats_snapshot),
            Arc::clone(&market_data),
            arbitrage_config,
            &config,
        );
        let (automation, automation_config_store) = init_automation_runtime(&config);
        let onchain_comparison_runtime = init_onchain_comparison_runtime(&config);
        let mut onchain_execution_replay = OnchainExecutionRunStore::load(&config);
        onchain_execution_replay.store = onchain_execution_replay
            .store
            .with_wallet_claims(wallet_claims.clone());
        let onchain_execution_runs = Arc::new(DashMap::new());
        for run in onchain_execution_replay.runs {
            onchain_execution_runs.insert(run.run_id.clone(), run);
        }
        let pending_onchain_executions = onchain_execution_replay.pending;
        let onchain_replenishment_plans = Arc::new(
            OnchainReplenishmentPlanStore::load(&config).with_wallet_claims(wallet_claims.clone()),
        );
        let onchain_cross_chain_runs = Arc::new(
            OnchainCrossChainRunStore::load(&config).with_wallet_claims(wallet_claims.clone()),
        );
        let onchain_token_approval_runs = Arc::new(
            OnchainTokenApprovalRunStore::load(&config).with_wallet_claims(wallet_claims.clone()),
        );
        let backpack_stocks = crate::services::backpack_stocks::BackpackStocks::new()?
            .with_peer_markets(instrument_registry.clone(),market_data.clone())
            .with_peer_feed(aggregator.clone(),market_subscriptions.clone())
            .with_wallet_claims(wallet_claims)
            .with_webhook(webhook_dispatcher.clone());
        #[cfg(not(test))]
        let backpack_stocks = backpack_stocks
            .with_rfq_store(config.storage.resolve_runtime_path("stocks/rfq-history.jsonl"))
            .with_plan_store(config.storage.resolve_runtime_path("stocks/plans.jsonl"))
            .with_peer_plan_store(config.storage.resolve_runtime_path("stocks/peer-plans.jsonl"))
            .with_funding_store(config.storage.resolve_runtime_path("stocks/funding-plans.jsonl"))
            .with_stablecoin_store(config.storage.resolve_runtime_path("stocks/stablecoin-plans.jsonl"))
            .with_exchange_conversion_store(config.storage.resolve_runtime_path("stocks/exchange-conversions.jsonl"));
        let backpack_stocks = Arc::new(backpack_stocks);

        let state = Self {
            inner: Arc::new(AppStateInner {
                config,
                market: MarketRuntimeState {
                    backpack_stocks,
                    aggregator,
                    funding_diff_stats_snapshot,
                    trade_fee_cache,
                    market_data,
                    instrument_registry,
                    market_subscriptions,
                    market_subscription_mutation_lock: Arc::new(Mutex::new(())),
                    gate_crossex_mode,
                    gate_crossex_mode_mutation_lock: Arc::new(Mutex::new(())),
                },
                trading: TradingRuntimeState {
                    service: trading_runtime.service,
                    hedge_ticket_hot_cache: hedge_tickets,
                    hedge_preview_hot_cache: hedge_previews,
                    execution_run_hot_cache: run_runtime.execution_runs,
                    execution_run_store: run_runtime.execution_run_store,
                    close_run_hot_cache: run_runtime.close_runs,
                    close_run_store: run_runtime.close_run_store,
                    action_run_hot_cache: run_runtime.action_runs,
                    live_order_proof_health: trading_runtime.health.live_order_proof_health,
                    private_ws_health: trading_runtime.health.private_ws_health,
                    reconciliation_health: trading_runtime.health.reconciliation_health,
                    run_finality_health: trading_runtime.health.run_finality_health,
                    sql_ledger_health: trading_sql_ledger_health,
                    runtime_config_store: trading_runtime.config_store,
                    runtime_config_mutation_lock: Arc::new(Mutex::new(())),
                    private_account_refresh_queue: PrivateAccountRefreshQueue::default(),
                },
                product: ProductRuntimeState {
                    arbitrage_engine,
                    arbitrage_scan_lock,
                    arbitrage_refresh,
                    opportunity_index: arbitrage_index,
                    automation,
                    automation_config_store,
                    automation_mutation_lock: Arc::new(Mutex::new(())),
                    onchain_monitor: onchain_comparison_runtime.monitor,
                    onchain_config_store: onchain_comparison_runtime.store,
                    onchain_config_mutation_lock: Arc::new(Mutex::new(())),
                    onchain_execution_builds: Arc::new(OnchainExecutionBuildStore::default()),
                    onchain_execution_runs,
                    onchain_execution_run_store: Arc::new(onchain_execution_replay.store),
                    onchain_replenishment_plans,
                    onchain_cross_chain_runs,
                    onchain_token_approval_builds: Arc::new(OnchainTokenApprovalStore::default()),
                    onchain_token_approval_runs,
                    webhook: webhook_dispatcher,
                    webhook_config_mutation_lock: Arc::new(Mutex::new(())),
                    missed_opportunity_hot_cache: missed_opportunities,
                    watchlist: watchlist_alert_runtime.watchlist,
                    alert_rules: watchlist_alert_runtime.alert_rules,
                    alert_cooldowns: watchlist_alert_runtime.alert_cooldowns,
                    watchlist_alert_mutation_lock: watchlist_alert_runtime.mutation_lock,
                    portfolio_nav_history: portfolio_runtime.nav_history,
                    portfolio_pnl_snapshot: portfolio_runtime.pnl_snapshot,
                    portfolio_pnl_refresh: portfolio_runtime.pnl_refresh,
                    portfolio_snapshot: portfolio_runtime.snapshot,
                    portfolio_snapshot_envelope: portfolio_runtime.snapshot_envelope,
                    portfolio_refresh: portfolio_runtime.refresh,
                    review_snapshot: Arc::new(RefreshingSnapshot::new(Duration::from_secs(30))),
                    account_open_orders_snapshot: portfolio_runtime.account_open_orders_snapshot,
                    account_open_orders_refresh_lock: portfolio_runtime
                        .account_open_orders_refresh_lock,
                },
                diagnostics: DiagnosticsRuntimeState {
                    ws_hub,
                    #[cfg(feature = "legacy-chat")]
                    llm_router,
                    venue_quality: trading_runtime.health.venue_quality,
                    system_health_snapshot: new_system_health_snapshot(),
                    ws_tickets,
                    metrics,
                    task_registry,
                },
                storage: StorageRuntimeState {
                    history_store,
                    watchlist_alert_store: watchlist_alert_runtime.store,
                    portfolio_nav_storage_health: portfolio_runtime.nav_storage_health,
                },
            }),
        };
        crate::services::onchain_comparison::restore_execution_reconciliations(
            &state,
            pending_onchain_executions,
        );
        Ok(state)
    }

    pub(crate) fn config(&self) -> &AppConfig {
        &self.inner.config
    }

    pub(crate) fn aggregator(&self) -> &Aggregator {
        &self.inner.market.aggregator
    }

    pub(crate) fn aggregator_handle(&self) -> &Arc<Aggregator> {
        &self.inner.market.aggregator
    }

    pub(crate) fn instrument_registry(&self) -> &Arc<InstrumentRegistry> {
        &self.inner.market.instrument_registry
    }

    pub(crate) fn market_subscriptions(&self) -> &Arc<MarketSubscriptions> {
        &self.inner.market.market_subscriptions
    }

    pub(crate) fn market_subscription_mutation_lock(&self) -> &Arc<Mutex<()>> {
        &self.inner.market.market_subscription_mutation_lock
    }

    pub(crate) fn gate_crossex_mode(&self) -> &Arc<GateCrossExModeService> {
        &self.inner.market.gate_crossex_mode
    }

    pub(crate) fn backpack_stocks(&self) -> &Arc<crate::services::backpack_stocks::BackpackStocks> {
        &self.inner.market.backpack_stocks
    }

    #[cfg(test)]
    pub(crate) fn use_stock_fixture(&mut self, root: &str) -> anyhow::Result<()> {
        Arc::get_mut(&mut self.inner).expect("fixture must precede background tasks")
            .market.backpack_stocks = Arc::new(
                crate::services::backpack_stocks::BackpackStocks::new()?.with_public_fixture(root),
            );
        Ok(())
    }

    pub(crate) fn gate_crossex_mode_mutation_lock(&self) -> &Arc<Mutex<()>> {
        &self.inner.market.gate_crossex_mode_mutation_lock
    }

    pub(crate) fn arbitrage_engine_handle(&self) -> &Arc<ArbitrageEngineV3> {
        &self.inner.product.arbitrage_engine
    }

    pub(crate) fn task_registry(&self) -> &crate::task_registry::TaskRegistry {
        &self.inner.diagnostics.task_registry
    }

    pub(crate) fn arbitrage_scan_lock(&self) -> &Arc<Mutex<()>> {
        &self.inner.product.arbitrage_scan_lock
    }

    pub(crate) fn arbitrage_refresh_signal(&self) -> &Arc<Notify> {
        &self.inner.product.arbitrage_refresh
    }

    pub(crate) fn request_arbitrage_refresh(&self) {
        self.inner.product.arbitrage_refresh.notify_one();
    }

    pub(crate) fn funding_diff_stats_snapshot_handle(
        &self,
    ) -> &Arc<RefreshingSnapshot<Vec<FundingDiffStatsRow>>> {
        &self.inner.market.funding_diff_stats_snapshot
    }

    pub(crate) fn market_data(&self) -> &Arc<MarketDataCache> {
        &self.inner.market.market_data
    }

    pub(crate) fn trade_fee_cache(&self) -> &Arc<TradeFeeCache> {
        &self.inner.market.trade_fee_cache
    }

    pub(crate) fn ws_hub(&self) -> &WsHub {
        &self.inner.diagnostics.ws_hub
    }

    #[cfg(feature = "legacy-chat")]
    pub(crate) fn llm_router(&self) -> &LlmRouter {
        &self.inner.diagnostics.llm_router
    }

    pub(crate) fn trading_service(&self) -> &Arc<TradingService> {
        &self.inner.trading.service
    }

    pub(crate) fn trading_runtime_config_store(&self) -> &Arc<TradingRuntimeConfigStore> {
        &self.inner.trading.runtime_config_store
    }

    pub(crate) fn trading_runtime_config_mutation_lock(&self) -> &Arc<Mutex<()>> {
        &self.inner.trading.runtime_config_mutation_lock
    }

    pub(crate) fn hedge_previews(&self) -> &Arc<DashMap<String, HedgePreviewResponse>> {
        &self.inner.trading.hedge_preview_hot_cache
    }

    pub(crate) fn hedge_tickets(&self) -> &Arc<DashMap<String, HedgeTicket>> {
        &self.inner.trading.hedge_ticket_hot_cache
    }

    pub(crate) fn opportunity_index(&self) -> &Arc<OpportunityIndex> {
        &self.inner.product.opportunity_index
    }

    pub(crate) fn automation(&self) -> &Arc<AutomationController> {
        &self.inner.product.automation
    }

    pub(crate) fn automation_config_store(&self) -> &Arc<AutomationConfigStore> {
        &self.inner.product.automation_config_store
    }

    pub(crate) fn automation_mutation_lock(&self) -> &Arc<Mutex<()>> {
        &self.inner.product.automation_mutation_lock
    }

    pub(crate) fn onchain_monitor(&self) -> &Arc<OnchainMonitor> {
        &self.inner.product.onchain_monitor
    }

    pub(crate) fn onchain_config_store(&self) -> &Arc<OnchainComparisonConfigStore> {
        &self.inner.product.onchain_config_store
    }

    pub(crate) fn onchain_config_mutation_lock(&self) -> &Arc<Mutex<()>> {
        &self.inner.product.onchain_config_mutation_lock
    }

    pub(crate) fn onchain_execution_builds(&self) -> &Arc<OnchainExecutionBuildStore> {
        &self.inner.product.onchain_execution_builds
    }

    pub(crate) fn onchain_execution_runs(
        &self,
    ) -> &Arc<DashMap<String, OnchainExecutionSubmitResponse>> {
        &self.inner.product.onchain_execution_runs
    }

    pub(crate) fn onchain_execution_run_store(&self) -> &Arc<OnchainExecutionRunStore> {
        &self.inner.product.onchain_execution_run_store
    }

    pub(crate) fn onchain_replenishment_plans(&self) -> &Arc<OnchainReplenishmentPlanStore> {
        &self.inner.product.onchain_replenishment_plans
    }

    pub(crate) fn onchain_cross_chain_runs(&self) -> &Arc<OnchainCrossChainRunStore> {
        &self.inner.product.onchain_cross_chain_runs
    }

    pub(crate) fn onchain_token_approval_builds(&self) -> &Arc<OnchainTokenApprovalStore> {
        &self.inner.product.onchain_token_approval_builds
    }

    pub(crate) fn onchain_token_approval_runs(
        &self,
    ) -> &Arc<OnchainTokenApprovalRunStore> {
        &self.inner.product.onchain_token_approval_runs
    }

    pub(crate) fn webhook(&self) -> &Arc<WebhookDispatcher> {
        &self.inner.product.webhook
    }

    pub(crate) fn webhook_config_mutation_lock(&self) -> &Arc<Mutex<()>> {
        &self.inner.product.webhook_config_mutation_lock
    }

    pub(crate) fn cache_arbitrage_report(&self, report: OpportunityScanReport) {
        let published_at = chrono::Utc::now();
        let snapshot_id = crate::services::opportunity::snapshot_id(published_at, &report.meta);
        self.inner
            .product
            .opportunity_index
            .publish_report(snapshot_id, published_at, report);
    }

    pub(crate) fn execution_runs(&self) -> &Arc<DashMap<String, ExecutionRun>> {
        &self.inner.trading.execution_run_hot_cache
    }

    pub(crate) fn execution_run_store(&self) -> &Arc<ExecutionRunStore> {
        &self.inner.trading.execution_run_store
    }

    pub(crate) fn close_runs(&self) -> &Arc<DashMap<String, CloseRun>> {
        &self.inner.trading.close_run_hot_cache
    }

    pub(crate) fn close_run_store(&self) -> &Arc<CloseRunStore> {
        &self.inner.trading.close_run_store
    }

    pub(crate) fn action_runs(&self) -> &Arc<DashMap<String, ActionRun>> {
        &self.inner.trading.action_run_hot_cache
    }

    pub(crate) fn missed_opportunities(&self) -> &Arc<DashMap<String, MissedOpportunity>> {
        &self.inner.product.missed_opportunity_hot_cache
    }

    pub(crate) fn live_order_proof_health(&self) -> &Arc<LiveOrderProofHealthStore> {
        &self.inner.trading.live_order_proof_health
    }

    pub(crate) fn private_ws_health(&self) -> &Arc<PrivateWsHealthStore> {
        &self.inner.trading.private_ws_health
    }

    pub(crate) fn private_account_refresh_queue(&self) -> &PrivateAccountRefreshQueue {
        &self.inner.trading.private_account_refresh_queue
    }

    pub(crate) fn reconciliation_health(&self) -> &Arc<ReconciliationHealthStore> {
        &self.inner.trading.reconciliation_health
    }

    pub(crate) fn run_finality_health(&self) -> &Arc<RunFinalityHealthStore> {
        &self.inner.trading.run_finality_health
    }

    pub(crate) fn venue_quality(&self) -> &Arc<VenueQualityTracker> {
        &self.inner.diagnostics.venue_quality
    }

    pub(crate) fn history_store(&self) -> &Arc<HistoryStore> {
        &self.inner.storage.history_store
    }

    pub(crate) fn trading_sql_ledger_health(&self) -> &trading::SqlLedgerMigrationHealth {
        &self.inner.trading.sql_ledger_health
    }

    pub(crate) fn watchlist(&self) -> &Arc<RwLock<Vec<WatchlistItem>>> {
        &self.inner.product.watchlist
    }

    pub(crate) fn alert_rules(&self) -> &Arc<RwLock<Vec<AlertRule>>> {
        &self.inner.product.alert_rules
    }

    pub(crate) fn alert_cooldowns(&self) -> &Arc<DashMap<i64, i64>> {
        &self.inner.product.alert_cooldowns
    }

    pub(crate) fn watchlist_alert_store(&self) -> &Arc<WatchlistAlertStore> {
        &self.inner.storage.watchlist_alert_store
    }

    pub(crate) fn watchlist_alert_mutation_lock(&self) -> &Arc<Mutex<()>> {
        &self.inner.product.watchlist_alert_mutation_lock
    }

    pub(crate) fn metrics(&self) -> &Arc<Metrics> {
        &self.inner.diagnostics.metrics
    }

    pub(crate) fn portfolio_nav_storage_health(&self) -> &Arc<nav_persist::NavStorageHealthStore> {
        &self.inner.storage.portfolio_nav_storage_health
    }

    pub(crate) fn portfolio_nav_history(&self) -> &SharedNavHistory {
        &self.inner.product.portfolio_nav_history
    }

    pub(crate) fn portfolio_pnl_snapshot(&self) -> &RefreshingSnapshot<PortfolioPnlSnapshot> {
        &self.inner.product.portfolio_pnl_snapshot
    }

    pub(crate) fn cache_portfolio_pnl_snapshot(&self, snapshot: PortfolioPnlSnapshot) {
        self.inner.product.portfolio_pnl_snapshot.set_now(snapshot);
    }

    pub(crate) fn portfolio_pnl_refresh_signal(&self) -> &Notify {
        &self.inner.product.portfolio_pnl_refresh
    }

    pub(crate) fn request_portfolio_pnl_refresh(&self) {
        self.inner.product.portfolio_pnl_refresh.notify_one();
    }

    pub(crate) fn portfolio_snapshot(&self) -> &RefreshingSnapshot<PortfolioSnapshot> {
        &self.inner.product.portfolio_snapshot
    }

    pub(crate) fn cache_portfolio_snapshot(&self, snapshot: PortfolioSnapshot) {
        self.inner.product.portfolio_snapshot.set_now(snapshot);
    }

    pub(crate) fn portfolio_snapshot_envelope(
        &self,
    ) -> &RefreshingSnapshot<PortfolioSnapshotEnvelope> {
        &self.inner.product.portfolio_snapshot_envelope
    }

    pub(crate) fn cache_portfolio_snapshot_envelope(&self, envelope: PortfolioSnapshotEnvelope) {
        self.inner
            .product
            .portfolio_snapshot_envelope
            .set_now(envelope);
    }

    pub(crate) fn portfolio_refresh_signal(&self) -> &Notify {
        &self.inner.product.portfolio_refresh
    }

    pub(crate) fn request_portfolio_refresh(&self) {
        self.inner.product.portfolio_refresh.notify_one();
    }

    pub(crate) fn review_snapshot(&self) -> &RefreshingSnapshot<ReviewRuntimeSnapshot> {
        &self.inner.product.review_snapshot
    }

    pub(crate) fn cache_review_snapshot(&self, snapshot: ReviewRuntimeSnapshot) {
        self.inner.product.review_snapshot.set_now(snapshot);
    }

    pub(crate) fn account_open_orders_snapshot(
        &self,
    ) -> &RefreshingSnapshot<CachedAccountOpenOrders> {
        &self.inner.product.account_open_orders_snapshot
    }

    pub(crate) fn account_open_orders_refresh_lock(&self) -> &Arc<Mutex<()>> {
        &self.inner.product.account_open_orders_refresh_lock
    }

    pub(crate) fn cache_account_open_orders(&self, snapshot: CachedAccountOpenOrders) {
        self.inner
            .product
            .account_open_orders_snapshot
            .set_now(snapshot);
    }

    pub(crate) fn system_health_snapshot(&self) -> &RefreshingSnapshot<SystemHealth> {
        &self.inner.diagnostics.system_health_snapshot
    }

    pub(crate) fn cache_system_health(&self, health: SystemHealth) {
        self.inner
            .diagnostics
            .system_health_snapshot
            .set_now(health);
    }

    pub(crate) fn ws_tickets(&self) -> &Arc<WsTicketStore> {
        &self.inner.diagnostics.ws_tickets
    }
}

struct PortfolioRuntimeState {
    nav_storage_health: Arc<nav_persist::NavStorageHealthStore>,
    nav_history: SharedNavHistory,
    pnl_snapshot: Arc<RefreshingSnapshot<PortfolioPnlSnapshot>>,
    pnl_refresh: Arc<Notify>,
    snapshot: Arc<RefreshingSnapshot<PortfolioSnapshot>>,
    snapshot_envelope: Arc<RefreshingSnapshot<PortfolioSnapshotEnvelope>>,
    refresh: Arc<Notify>,
    account_open_orders_snapshot: Arc<RefreshingSnapshot<CachedAccountOpenOrders>>,
    account_open_orders_refresh_lock: Arc<Mutex<()>>,
}

async fn init_portfolio_runtime(config: &AppConfig) -> PortfolioRuntimeState {
    let oldest_nav_ms = common::time::now_ms().saturating_sub(26 * 60 * 60 * 1_000);
    let nav_storage_health = Arc::new(nav_persist::NavStorageHealthStore::new(config));
    let nav_history = Arc::new(RwLock::new(
        nav_persist::load(config, oldest_nav_ms, &nav_storage_health).await,
    ));
    PortfolioRuntimeState {
        nav_storage_health,
        nav_history,
        pnl_snapshot: Arc::new(RefreshingSnapshot::new(Duration::from_secs(30))),
        pnl_refresh: Arc::new(Notify::new()),
        snapshot: Arc::new(RefreshingSnapshot::new(Duration::from_secs(2))),
        snapshot_envelope: Arc::new(RefreshingSnapshot::new(Duration::from_secs(2))),
        refresh: Arc::new(Notify::new()),
        account_open_orders_snapshot: Arc::new(RefreshingSnapshot::new(Duration::from_secs(2))),
        account_open_orders_refresh_lock: Arc::new(Mutex::new(())),
    }
}

async fn init_history_store(config: &AppConfig) -> Arc<HistoryStore> {
    if history_is_disabled(config) {
        return disabled_history_store();
    }
    let Some(url) = config.history.database_url(&config.storage) else {
        return memory_history_store();
    };
    init_postgres_history_store(url).await
}

async fn init_watchlist_alert_runtime(config: &AppConfig) -> WatchlistAlertRuntimeState {
    let required = config.api_surface.watchlist_alerts;
    let path = required
        .then_some(config.storage.watchlist_alerts_path.as_deref())
        .flatten()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| config.storage.resolve_runtime_path(value));
    watchlist_alert_runtime_state(WatchlistAlertStore::initialize(path, required).await)
}

fn watchlist_alert_runtime_state(replay: WatchlistAlertReplay) -> WatchlistAlertRuntimeState {
    WatchlistAlertRuntimeState {
        alert_cooldowns: alert_cooldowns_from_replay(&replay.alert_rules),
        watchlist: Arc::new(RwLock::new(replay.watchlist)),
        alert_rules: Arc::new(RwLock::new(replay.alert_rules)),
        store: replay.store,
        mutation_lock: Arc::new(Mutex::new(())),
    }
}

fn alert_cooldowns_from_replay(rules: &[AlertRule]) -> Arc<DashMap<i64, i64>> {
    let now_ms = common::time::now_ms();
    let cooldowns = Arc::new(DashMap::new());
    for rule in rules {
        if rule.enabled {
            if let Some(deadline) = rule
                .runtime
                .next_eligible_at_ms
                .filter(|deadline| *deadline > now_ms)
            {
                cooldowns.insert(rule.id, deadline);
            }
        }
    }
    cooldowns
}

fn init_automation_runtime(
    config: &AppConfig,
) -> (Arc<AutomationController>, Arc<AutomationConfigStore>) {
    let replay = AutomationConfigStore::load(automation_config_path(config));
    report_automation_replay_problem(replay.problem.as_deref());
    let controller = restored_automation_controller(replay.config);
    (Arc::new(controller), Arc::new(replay.store))
}

fn init_onchain_comparison_runtime(_config: &AppConfig) -> OnchainComparisonRuntimeState {
    #[cfg(not(test))]
    let checkpoint_path = Some(
        _config
            .storage
            .resolve_runtime_path("onchain-comparison.json"),
    );
    #[cfg(test)]
    let checkpoint_path = None;
    let OnchainComparisonConfigReplay {
        store,
        config,
        batch_configs,
        problem,
    } = OnchainComparisonConfigStore::load(checkpoint_path);
    if let Some(problem) = problem.as_deref() {
        tracing::warn!(%problem, "on-chain comparison config replay failed; using safe defaults");
    }
    let now_ms = common::time::now_ms();
    let monitor = config.map_or_else(OnchainMonitor::default, |mut config| {
        let custom_rpc_url = (config.rpc.mode == OnchainRpcMode::Custom)
            .then(|| crate::services::onchain_rpc_registry::configured_url(&config.chain))
            .flatten();
        if config.rpc.mode == OnchainRpcMode::Custom && custom_rpc_url.is_none() {
            tracing::warn!(
                chain = %config.chain,
                "restored custom RPC mode has no secure endpoint; using provider-managed reads"
            );
            config.rpc.mode = OnchainRpcMode::ProviderManaged;
        }
        OnchainMonitor::from_config_with_custom_rpc(config, custom_rpc_url, now_ms)
            .unwrap_or_else(|problem| {
            tracing::warn!(%problem, "restored on-chain comparison config is invalid; using safe defaults");
            OnchainMonitor::default()
        })
    });
    crate::services::onchain_comparison::restore_batch_configs(&monitor, batch_configs, now_ms);
    OnchainComparisonRuntimeState {
        monitor: Arc::new(monitor),
        store: Arc::new(store),
    }
}

fn automation_config_path(config: &AppConfig) -> Option<PathBuf> {
    config
        .storage
        .automation_config_path
        .as_deref()
        .map(|value| config.storage.resolve_runtime_path(value))
}

#[cfg(not(test))]
fn gate_crossex_mode_config_path(config: &AppConfig) -> PathBuf {
    config
        .storage
        .resolve_runtime_path("gate_crossex_mode.json")
}

#[cfg(test)]
fn gate_crossex_mode_config_path(_: &AppConfig) -> Option<PathBuf> {
    None
}

async fn init_webhook_runtime(
    config: &AppConfig,
    execution_runs: &DashMap<String, ExecutionRun>,
) -> Result<Arc<WebhookDispatcher>> {
    let dispatcher = WebhookDispatcher::initialize(webhook_outbox_path(config)).await?;
    if dispatcher.requires_outbox_bootstrap() {
        let event_ids = execution_runs
            .iter()
            .filter_map(|run| webhook::execution_result_event_id(run.value()))
            .collect();
        dispatcher.bootstrap_event_ids(event_ids).await?;
    }
    Ok(Arc::new(dispatcher))
}

fn webhook_outbox_path(config: &AppConfig) -> Option<PathBuf> {
    config
        .storage
        .webhook_outbox_path
        .as_deref()
        .map(|value| config.storage.resolve_runtime_path(value))
}

fn report_automation_replay_problem(problem: Option<&str>) {
    if let Some(problem) = problem {
        tracing::warn!(%problem, "automation config replay failed; using safe defaults");
    }
}

fn restored_automation_controller(
    config: shared_types::AutomatedArbitrageConfig,
) -> AutomationController {
    match AutomationController::restored(config, common::time::now_ms()) {
        Ok(controller) => controller,
        Err(error) => {
            tracing::warn!(%error, "automation config replay was invalid; using safe defaults");
            AutomationController::default()
        }
    }
}

fn history_is_disabled(config: &AppConfig) -> bool {
    !config.history.enabled
}

fn disabled_history_store() -> Arc<HistoryStore> {
    tracing::info!("history store disabled by config");
    Arc::new(HistoryStore::disabled())
}

fn storage_postgres_url(config: &AppConfig) -> Option<&str> {
    config
        .storage
        .postgres_url
        .as_deref()
        .filter(|url| !url.trim().is_empty())
}

async fn init_postgres_history_store(url: &str) -> Arc<HistoryStore> {
    match HistoryStore::postgres(url).await {
        Ok(store) => ready_history_store(store),
        Err(error) => fallback_history_store(&error),
    }
}

fn ready_history_store(store: HistoryStore) -> Arc<HistoryStore> {
    tracing::info!(backend = store.backend_name(), "history store initialized");
    Arc::new(store)
}

fn fallback_history_store(error: &realtime::history::HistoryError) -> Arc<HistoryStore> {
    tracing::warn!(
        %error,
        "postgres history store unavailable; falling back to in-memory history"
    );
    Arc::new(HistoryStore::memory_fallback(error.to_string()))
}

fn memory_history_store() -> Arc<HistoryStore> {
    Arc::new(HistoryStore::default())
}

#[cfg(test)]
fn isolate_default_runtime_paths_for_tests(config: &mut AppConfig) {
    clear_default_runtime_path(
        &mut config.storage.portfolio_nav_path,
        common::config::DEFAULT_PORTFOLIO_NAV_FILE,
    );
    clear_default_runtime_path(
        &mut config.storage.watchlist_alerts_path,
        common::config::DEFAULT_WATCHLIST_ALERTS_FILE,
    );
    clear_default_runtime_path(
        &mut config.storage.execution_run_ledger_path,
        common::config::DEFAULT_EXECUTION_RUN_LEDGER_FILE,
    );
    clear_default_runtime_path(
        &mut config.storage.onchain_execution_run_ledger_path,
        common::config::DEFAULT_ONCHAIN_EXECUTION_RUN_LEDGER_FILE,
    );
    clear_default_runtime_path(
        &mut config.storage.onchain_replenishment_ledger_path,
        common::config::DEFAULT_ONCHAIN_REPLENISHMENT_LEDGER_FILE,
    );
    clear_default_runtime_path(
        &mut config.storage.onchain_cross_chain_ledger_path,
        common::config::DEFAULT_ONCHAIN_CROSS_CHAIN_LEDGER_FILE,
    );
    clear_default_runtime_path(
        &mut config.storage.execution_ledger_path,
        common::config::DEFAULT_EXECUTION_LEDGER_FILE,
    );
    clear_default_runtime_path(
        &mut config.storage.order_snapshot_path,
        common::config::DEFAULT_ORDER_SNAPSHOT_FILE,
    );
    clear_default_runtime_path(
        &mut config.storage.close_run_ledger_path,
        common::config::DEFAULT_CLOSE_RUN_LEDGER_FILE,
    );
    clear_default_runtime_path(
        &mut config.storage.automation_config_path,
        common::config::DEFAULT_AUTOMATION_CONFIG_FILE,
    );
    clear_default_runtime_path(
        &mut config.storage.webhook_outbox_path,
        common::config::DEFAULT_WEBHOOK_OUTBOX_FILE,
    );
    clear_default_runtime_path(
        &mut config.storage.market_subscriptions_path,
        common::config::DEFAULT_MARKET_SUBSCRIPTIONS_FILE,
    );
}

#[cfg(not(test))]
fn isolate_default_runtime_paths_for_tests(_: &mut AppConfig) {}

#[cfg(test)]
fn clear_default_runtime_path(path: &mut Option<String>, default_file: &str) {
    if path
        .as_deref()
        .is_some_and(|value| value.trim() == default_file)
    {
        *path = None;
    }
}

fn close_runs_from_replay(runs: Vec<CloseRun>) -> Arc<DashMap<String, CloseRun>> {
    let map = Arc::new(DashMap::new());
    for run in runs {
        map.insert(run.id.clone(), run);
    }
    map
}

fn execution_runs_from_replay(runs: Vec<ExecutionRun>) -> Arc<DashMap<String, ExecutionRun>> {
    let map = Arc::new(DashMap::new());
    for run in runs {
        map.insert(run.run_id.clone(), run);
    }
    map
}

fn action_runs_from_replay(runs: Vec<ActionRun>) -> Arc<DashMap<String, ActionRun>> {
    let map = Arc::new(DashMap::new());
    for run in runs {
        map.insert(run.id.clone(), run);
    }
    map
}

fn load_action_runs_from_audit(config: &AppConfig) -> Result<Arc<DashMap<String, ActionRun>>> {
    let replay = audit::replay_action_runs(config.security.audit_log_path.as_deref())?;
    Ok(action_runs_from_replay(replay))
}

fn apply_sql_run_finality_replay(
    execution_runs: &DashMap<String, ExecutionRun>,
    close_runs: &DashMap<String, CloseRun>,
    events: &[trading::SqlRunFinalityReplayEvent],
) {
    for event in events {
        match event.run_kind.as_str() {
            "execution_run" => apply_execution_run_finality_replay(execution_runs, event),
            "close_run" => apply_close_run_finality_replay(close_runs, event),
            other => tracing::warn!(
                run_kind = other,
                event_id = %event.event_id,
                "skipping unknown SQL run finality replay kind"
            ),
        }
    }
}

fn apply_execution_run_finality_replay(
    execution_runs: &DashMap<String, ExecutionRun>,
    event: &trading::SqlRunFinalityReplayEvent,
) {
    match serde_json::from_value::<ExecutionRun>(event.payload.clone()) {
        Ok(run) => upsert_newer_execution_run(execution_runs, run),
        Err(error) => tracing::warn!(
            event_id = %event.event_id,
            run_id = %event.run_id,
            %error,
            "skipping malformed execution run finality payload"
        ),
    }
}

fn apply_close_run_finality_replay(
    close_runs: &DashMap<String, CloseRun>,
    event: &trading::SqlRunFinalityReplayEvent,
) {
    match serde_json::from_value::<CloseRun>(event.payload.clone()) {
        Ok(run) => upsert_newer_close_run(close_runs, run),
        Err(error) => tracing::warn!(
            event_id = %event.event_id,
            run_id = %event.run_id,
            %error,
            "skipping malformed close run finality payload"
        ),
    }
}

fn upsert_newer_execution_run(execution_runs: &DashMap<String, ExecutionRun>, run: ExecutionRun) {
    let should_insert = execution_runs
        .get(&run.run_id)
        .is_none_or(|existing| run.updated_at_ms >= existing.updated_at_ms);
    if should_insert {
        execution_runs.insert(run.run_id.clone(), run);
    }
}

fn upsert_newer_close_run(close_runs: &DashMap<String, CloseRun>, run: CloseRun) {
    let should_insert = close_runs
        .get(&run.id)
        .is_none_or(|existing| run.updated_at_ms >= existing.updated_at_ms);
    if should_insert {
        close_runs.insert(run.id.clone(), run);
    }
}

fn new_trading_service(config: &AppConfig, sql_ledger: trading::SqlLedgerInit) -> TradingService {
    let execution_ledger_path = config
        .storage
        .execution_ledger_path
        .as_deref()
        .map(PathBuf::from);
    let order_snapshot_path = config
        .storage
        .order_snapshot_path
        .as_deref()
        .map(PathBuf::from);
    TradingService::new_mock_with_storage_paths_and_sql(
        execution_ledger_path,
        order_snapshot_path,
        sql_ledger,
    )
}

fn new_arbitrage_engine(
    funding_diff_stats_snapshot: Arc<RefreshingSnapshot<Vec<FundingDiffStatsRow>>>,
    market_data: Arc<MarketDataCache>,
    config: ArbitrageConfig,
    app_config: &AppConfig,
) -> Arc<ArbitrageEngineV3> {
    let data_source = Arc::new(crate::data_source::OpportunitySnapshotSource::new(
        funding_diff_stats_snapshot,
        market_data,
    ));
    Arc::new(ArbitrageEngineV3::new(
        config,
        data_source,
        app_config.arbitrage.total_capital_usd,
        app_config.arbitrage.risk_tolerance,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::{
        AlertChannel, AlertDeliveryState, AlertDeliveryStatus, AlertRule, AutomatedArbitrageConfig,
        AutomationRuntimeState, CloseRunScope, CloseRunStatus, ExecutionEnvironment,
        ExecutionRunLeg, ExecutionRunState, HedgeLegRole, LiveOrderState, WatchlistItem,
        WatchlistItemRuntime, WatchlistPersistence,
    };

    #[tokio::test]
    async fn app_state_restores_live_automation_config_paused() -> Result<()> {
        let path = temp_automation_config_path();
        let replay = AutomationConfigStore::load(Some(path.clone()));
        replay.store.persist(&AutomatedArbitrageConfig {
            enabled: true,
            paused: false,
            environment: ExecutionEnvironment::Live,
            capital_usd: 10.0,
            min_depth_usd: 10.0,
            cooldown_secs: 30,
            ..AutomatedArbitrageConfig::default()
        })?;
        let mut config = AppConfig::default();
        config.storage.automation_config_path = Some(path.display().to_string());

        let state = AppState::new(config).await?;
        let status = state.automation().snapshot();

        assert!(status.config.enabled);
        assert!(status.config.paused);
        assert_eq!(status.config.environment, ExecutionEnvironment::Live);
        assert_eq!(status.config.capital_usd, 10.0);
        assert_eq!(status.config.min_depth_usd, 10.0);
        assert_eq!(status.state, AutomationRuntimeState::Paused);
        let _ = std::fs::remove_file(path);
        Ok(())
    }

    #[tokio::test]
    async fn app_state_restores_trading_runtime_risk_and_protection() -> Result<()> {
        let ledger_path = temp_execution_ledger_path();
        let runtime_path = ledger_path.with_extension("trading-runtime.json");
        let store = TradingRuntimeConfigStore::load(Some(runtime_path.clone())).store;
        let mut risk = crate::services::risk_config::snapshot(&trading::RiskConfig::default());
        risk.max_order_notional = 20.0;
        risk.max_open_orders = 4;
        risk.max_hedge_imbalance_pct = 0.01;
        risk.allowed_exchanges = vec!["bitget".to_owned(), "hyperliquid".to_owned()];
        risk.protected_positions = vec![protected_binance_btc_position()];
        store.persist("mock", &risk)?;
        let mut config = AppConfig::default();
        config.storage.execution_ledger_path = Some(ledger_path.display().to_string());

        let state = AppState::new(config).await?;
        let restored =
            crate::services::risk_config::snapshot(&state.trading_service().risk_config());

        assert_eq!(state.trading_service().adapter_name(), "mock");
        assert_eq!(restored.max_order_notional, 20.0);
        assert_eq!(restored.max_open_orders, 4);
        assert_eq!(restored.max_hedge_imbalance_pct, 0.01);
        assert_eq!(
            restored.allowed_exchanges,
            vec!["bitget".to_owned(), "hyperliquid".to_owned()]
        );
        assert_eq!(
            restored.protected_positions,
            vec![protected_binance_btc_position()]
        );
        let _ = std::fs::remove_file(runtime_path);
        let _ = std::fs::remove_file(ledger_path);
        Ok(())
    }

    #[tokio::test]
    async fn app_state_restores_watchlist_alert_snapshot_and_active_cooldown() -> Result<()> {
        let path = temp_watchlist_alert_path();
        let mut config = AppConfig::default();
        config.api_surface.watchlist_alerts = true;
        config.storage.watchlist_alerts_path = Some(path.display().to_string());
        let state = AppState::new(config.clone()).await?;
        let watchlist = crate::services::watchlist_alerts::create_watchlist_item(
            &state,
            WatchlistItem {
                id: 0,
                symbol: "BTC-USDT".to_owned(),
                venue_long: Some("binance".to_owned()),
                venue_short: Some("okx".to_owned()),
                min_net_yield: Some(0.1),
                min_volume_24h: Some(1_000.0),
                enabled: true,
                created_at_ms: 0,
                persistence: WatchlistPersistence::default(),
                runtime: WatchlistItemRuntime::default(),
            },
            "api-token:operator:test",
        )
        .await?;
        let watchlist_id = watchlist
            .item
            .as_ref()
            .map(|item| item.id)
            .ok_or_else(|| anyhow::anyhow!("persisted watchlist item missing"))?;
        let alert = crate::services::watchlist_alerts::create_alert_rule(
            &state,
            AlertRule {
                id: 0,
                watchlist_id,
                channel: AlertChannel::Toast,
                cooldown_secs: 300,
                enabled: true,
                created_at_ms: 0,
                persistence: WatchlistPersistence::default(),
                delivery: AlertDeliveryState::configured(&AlertChannel::Toast),
                runtime: shared_types::AlertRuleRuntime::default(),
            },
            "api-token:operator:test",
        )
        .await?;
        let rule_id = alert
            .rule
            .as_ref()
            .map(|rule| rule.id)
            .ok_or_else(|| anyhow::anyhow!("persisted alert rule missing"))?;
        let deadline = common::time::now_ms() + 60_000;
        {
            let _serial = state.watchlist_alert_mutation_lock().lock().await;
            let watchlist = state.watchlist().read().await;
            let mut rules = state.alert_rules().write().await;
            let rule = rules
                .iter_mut()
                .find(|rule| rule.id == rule_id)
                .ok_or_else(|| anyhow::anyhow!("alert rule missing before replay"))?;
            rule.delivery.last_delivery_status = AlertDeliveryStatus::Queued;
            rule.delivery.last_fired_at_ms = Some(common::time::now_ms());
            rule.runtime.last_triggered_at_ms = rule.delivery.last_fired_at_ms;
            rule.runtime.next_eligible_at_ms = Some(deadline);
            state
                .watchlist_alert_store()
                .persist_snapshot(&watchlist, &rules)
                .await
                .map_err(anyhow::Error::msg)?;
        }
        drop(state);

        let restored = AppState::new(config).await?;
        assert_eq!(restored.watchlist().read().await.len(), 1);
        assert_eq!(restored.alert_rules().read().await.len(), 1);
        assert_eq!(
            restored
                .watchlist()
                .read()
                .await
                .first()
                .map(|item| item.persistence.created_by.as_str()),
            Some("api-token:operator:test")
        );
        assert_eq!(
            restored.alert_cooldowns().get(&rule_id).map(|row| *row),
            Some(deadline)
        );
        assert_eq!(
            restored.watchlist_alert_store().health().status,
            shared_types::WatchlistStorageStatus::Ready
        );
        let _ = std::fs::remove_file(path);
        Ok(())
    }

    #[tokio::test]
    async fn app_state_replays_execution_run_snapshots() -> Result<()> {
        let path = temp_execution_run_path();
        let run = execution_run("run-replay", ExecutionRunState::Hedged);
        std::fs::write(&path, format!("{}\n", serde_json::to_string(&run)?))?;
        let mut config = AppConfig::default();
        config.storage.execution_run_ledger_path = Some(path.display().to_string());

        let state = AppState::new(config).await?;

        assert_eq!(
            state
                .execution_runs()
                .get("run-replay")
                .map(|run| run.state),
            Some(ExecutionRunState::Hedged)
        );
        let _ = std::fs::remove_file(path);
        Ok(())
    }

    #[tokio::test]
    async fn app_state_recovers_interrupted_action_run_from_durable_audit_snapshot() -> Result<()> {
        let path = temp_action_run_audit_path();
        let action_run = ActionRun {
            id: "act-restart".to_owned(),
            kind: shared_types::ActionRunKind::TradingOrderSubmit,
            status: shared_types::ActionRunStatus::Accepted,
            actor: "api-token:operator:0123456789abcdef".to_owned(),
            target: Some("client-restart".to_owned()),
            request_id: Some("req-restart".to_owned()),
            idempotency_key: Some("client-restart".to_owned()),
            message: "accepted".to_owned(),
            problem: None,
            result: None,
            mutation: None,
            started_at_ms: 1,
            updated_at_ms: 2,
        };
        std::fs::write(
            &path,
            format!(
                "{}\n",
                serde_json::json!({ "detail": { "actionRun": action_run } })
            ),
        )?;
        let mut config = AppConfig::default();
        config.security.audit_log_path = Some(path.display().to_string());

        let state = AppState::new(config).await?;
        let recovered = state
            .action_runs()
            .get("act-restart")
            .map(|entry| entry.value().clone())
            .ok_or_else(|| anyhow::anyhow!("recovered action run missing"))?;

        assert_eq!(recovered.status, shared_types::ActionRunStatus::Failed);
        assert_eq!(
            recovered
                .problem
                .as_ref()
                .map(|problem| problem.code.as_str()),
            Some(shared_types::problem::codes::ACTION_RUN_REPLAY_UNAVAILABLE)
        );
        assert!(recovered.result.is_none());
        let _ = std::fs::remove_file(path);
        Ok(())
    }

    #[tokio::test]
    async fn app_state_configures_execution_ledger_storage() -> Result<()> {
        let path = temp_execution_ledger_path();
        let order_path = temp_order_snapshot_path();
        let mut config = AppConfig::default();
        config.storage.execution_ledger_path = Some(path.display().to_string());
        config.storage.order_snapshot_path = Some(order_path.display().to_string());

        let state = AppState::new(config).await?;
        let snapshot = state.trading_service().execution_ledger_storage_snapshot();

        assert!(snapshot.configured);
        assert_eq!(snapshot.path, Some(path.display().to_string()));
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(order_path);
        Ok(())
    }

    #[tokio::test]
    async fn app_state_test_default_does_not_use_shared_runtime_ledgers() -> Result<()> {
        let state = AppState::new(AppConfig::default()).await?;
        let execution = state.trading_service().execution_ledger_storage_snapshot();
        let orders = state.trading_service().order_snapshot_storage_snapshot();

        assert!(!execution.configured);
        assert_eq!(execution.path, None);
        assert!(!orders.configured);
        assert_eq!(orders.path, None);
        Ok(())
    }

    #[tokio::test]
    async fn app_state_records_unconfigured_trading_sql_ledger_health() -> Result<()> {
        let state = AppState::new(AppConfig::default()).await?;
        let health = state.trading_sql_ledger_health();

        assert!(!health.configured);
        assert!(!health.applied);
        assert_eq!(health.migration_id, trading::SQL_LEDGER_MIGRATION_ID);
        Ok(())
    }

    #[tokio::test]
    async fn app_state_replays_order_snapshots() -> Result<()> {
        let path = temp_order_snapshot_path();
        let mut config = AppConfig::default();
        config.storage.execution_ledger_path = None;
        config.storage.order_snapshot_path = Some(path.display().to_string());
        let state = AppState::new(config.clone()).await?;
        let intent = shared_types::OrderIntent {
            id: "order-replay".to_owned(),
            source: shared_types::OrderSource::Manual,
            strategy: None,
            mode: shared_types::ExecutionMode::DryRun,
            exchange: "mock".to_owned(),
            symbol: "BTC".to_owned(),
            side: shared_types::OrderSide::Buy,
            order_type: shared_types::OrderType::Limit,
            quantity: 1.0,
            price: Some(10.0),
            slippage_tolerance_bps: None,
            reduce_only: false,
            time_in_force: shared_types::TimeInForce::Ioc,
            post_only: false,
            margin_mode: shared_types::MarginMode::Cross,
            leverage: 1.0,
            client_order_id: "client-replay".to_owned(),
            client_order_id_policy: None,
            created_at_ms: 1,
        };
        state.trading_service().submit(intent).await?;

        let restored = AppState::new(config).await?;

        assert!(restored
            .trading_service()
            .list_orders()
            .iter()
            .any(|order| order.intent.id == "order-replay"));
        let _ = std::fs::remove_file(path);
        Ok(())
    }

    #[test]
    fn sql_run_finality_replay_overlays_newer_execution_run() -> Result<()> {
        let mut older = execution_run("run-sql", ExecutionRunState::SecondLegSubmitted);
        older.updated_at_ms = 10;
        let mut newer = execution_run("run-sql", ExecutionRunState::Hedged);
        newer.status_reason = "sql finality".to_owned();
        newer.updated_at_ms = 20;
        let execution_runs = execution_runs_from_replay(vec![older]);
        let close_runs = close_runs_from_replay(Vec::new());
        let event = sql_run_finality_event(
            "execution_run",
            "run-sql",
            "hedged",
            serde_json::to_value(newer)?,
            20,
        );

        apply_sql_run_finality_replay(&execution_runs, &close_runs, &[event]);

        let run = execution_runs
            .get("run-sql")
            .ok_or_else(|| anyhow::anyhow!("execution run overlay missing"))?;
        assert_eq!(run.state, ExecutionRunState::Hedged);
        assert_eq!(run.status_reason, "sql finality");
        Ok(())
    }

    #[test]
    fn sql_run_finality_replay_does_not_overwrite_newer_close_run() -> Result<()> {
        let mut newer = close_run("close-sql", CloseRunStatus::Submitted);
        newer.updated_at_ms = 30;
        let mut older = close_run("close-sql", CloseRunStatus::Succeeded);
        older.message = "stale sql finality".to_owned();
        older.updated_at_ms = 20;
        let execution_runs = execution_runs_from_replay(Vec::new());
        let close_runs = close_runs_from_replay(vec![newer]);
        let event = sql_run_finality_event(
            "close_run",
            "close-sql",
            "succeeded",
            serde_json::to_value(older)?,
            20,
        );

        apply_sql_run_finality_replay(&execution_runs, &close_runs, &[event]);

        let run = close_runs
            .get("close-sql")
            .ok_or_else(|| anyhow::anyhow!("close run overlay missing"))?;
        assert_eq!(run.status, CloseRunStatus::Submitted);
        assert_ne!(run.message, "stale sql finality");
        Ok(())
    }

    fn execution_run(id: &str, state: ExecutionRunState) -> ExecutionRun {
        ExecutionRun {
            run_id: id.to_owned(),
            ticket_id: "ticket-1".to_owned(),
            opportunity_id: "opp-1".to_owned(),
            state,
            long_leg: execution_run_leg(HedgeLegRole::Long),
            short_leg: execution_run_leg(HedgeLegRole::Short),
            net_exposure_usd: 0.0,
            cost_reconciliation: None,
            valuation_problem: None,
            unwind_problem: None,
            finality_problem: None,
            finality_checked_at_ms: None,
            evidence: Default::default(),
            recovery_action: None,
            status_reason: "replayed".to_owned(),
            created_at_ms: 1,
            updated_at_ms: 2,
        }
    }

    fn close_run(id: &str, status: CloseRunStatus) -> CloseRun {
        CloseRun {
            id: id.to_owned(),
            scope: CloseRunScope::Single,
            status,
            action_run_id: Some("act-1".to_owned()),
            request_id: Some("req-1".to_owned()),
            idempotency_key: None,
            snapshot_version: "pos-1".to_owned(),
            expected_leg_count: 0,
            reason: None,
            legs: Vec::new(),
            submitted_order_count: 0,
            failed_leg_count: 0,
            naked_exposure_usd: 0.0,
            message: "seed".to_owned(),
            problem: None,
            finality_problem: None,
            finality_checked_at_ms: None,
            unwind_plan: None,
            cost_events: Vec::new(),
            cost_reconciliation: None,
            started_at_ms: 1,
            updated_at_ms: 1,
        }
    }

    fn sql_run_finality_event(
        run_kind: &str,
        run_id: &str,
        state: &str,
        payload: serde_json::Value,
        occurred_at_ms: i64,
    ) -> trading::SqlRunFinalityReplayEvent {
        trading::SqlRunFinalityReplayEvent {
            event_id: format!("evt-{run_kind}-{run_id}-{occurred_at_ms}"),
            run_kind: run_kind.to_owned(),
            run_id: run_id.to_owned(),
            source_event_id: None,
            source_order_event_id: None,
            source: "order_query".to_owned(),
            state: state.to_owned(),
            payload,
            payload_hash: "hash".to_owned(),
            schema_version: trading::SQL_LEDGER_SCHEMA_VERSION as i32,
            occurred_at_ms,
            captured_at_ms: occurred_at_ms,
        }
    }

    fn execution_run_leg(role: HedgeLegRole) -> ExecutionRunLeg {
        ExecutionRunLeg {
            role,
            exchange: "paper".to_owned(),
            symbol: "BTC-USDT".to_owned(),
            order_ids: Vec::new(),
            identity: None,
            finality_source: None,
            confirmed_filled_at_ms: None,
            state: LiveOrderState::Filled,
            target_quantity: 1.0,
            filled_quantity: Some(1.0),
            target_notional_usd: 100.0,
            filled_notional_usd: Some(100.0),
            filled_fee: Some(0.1),
        }
    }

    fn temp_execution_run_path() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "crossline-state-execution-run-{}-{}.jsonl",
            std::process::id(),
            common::time::now_ms()
        ))
    }

    fn temp_action_run_audit_path() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "crossline-state-action-run-audit-{}-{}.jsonl",
            std::process::id(),
            common::time::now_ms()
        ))
    }

    fn temp_execution_ledger_path() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "crossline-state-execution-ledger-{}-{}.jsonl",
            std::process::id(),
            common::time::now_ms()
        ))
    }

    fn temp_order_snapshot_path() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "crossline-state-order-snapshot-{}-{}.jsonl",
            std::process::id(),
            common::time::now_ms()
        ))
    }

    fn temp_watchlist_alert_path() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "crossline-state-watchlist-alerts-{}-{}.sqlite",
            std::process::id(),
            common::time::now_ms()
        ))
    }

    fn temp_automation_config_path() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "crossline-state-automation-config-{}-{}.json",
            std::process::id(),
            common::time::now_ms()
        ))
    }

    fn protected_binance_btc_position() -> shared_types::ProtectedPositionFingerprint {
        shared_types::ProtectedPositionFingerprint {
            venue: "binance".to_owned(),
            canonical_symbol: "btc".to_owned(),
            native_symbol: "btcusdt".to_owned(),
            side: "long".to_owned(),
            quantity: 0.232,
            entry_price: 64_456.2,
            position_mode: Some("both".to_owned()),
            opening_identity: "user-preexisting-binance-btc-long".to_owned(),
            source: "operator_verified_account_position".to_owned(),
            captured_at_ms: 42,
        }
    }
}
