use crate::{
    api::ws::{start_stock_stream_with_state, WsChannelState},
    state::{
        context::use_global,
        load_state::LoadState,
        module_runtime::{ModuleRuntimeState, ModuleRuntimeStatus},
    },
};
use leptos::{prelude::*, task::spawn_local};
use shared_types::stocks::*;
use crate::panels::shared::operation_journal::OperationJournal;
pub(in crate::panels::modules::stocks) mod preflight;
mod rfq;
mod alerts;
mod peers;
mod batch;
mod notice;
mod monitor;
mod source;
use source::StockSource;
pub(super) use notice::Notice;
pub(super) use batch::BatchData;
pub(super) use peers::PeerData;
pub(super) use alerts::AlertData;
pub(super) use preflight::PreflightData;
pub(super) use rfq::RfqData;

#[derive(Clone, Copy)]
pub(in crate::panels) struct StocksRuntime {
    pub(super) market: RwSignal<LoadState<StockMarketSnapshot>>,
    catalog_status: RwSignal<ModuleRuntimeState>,
    batch: batch::BatchRuntime,
    monitor: OperationJournal,
    plan: OperationJournal,
    peer_plan: OperationJournal,
}
pub(in crate::panels) fn create_stocks_runtime() -> StocksRuntime {
    let market = RwSignal::new(LoadState::Loading);
    StocksRuntime {
        market,
        catalog_status: RwSignal::new(ModuleRuntimeState::ready()),
        batch: batch::BatchRuntime::new(market),
        monitor: OperationJournal::new("stocks-monitor"),
        plan: OperationJournal::new("stocks-plan"),
        peer_plan: OperationJournal::new("stocks-peer-plan"),
    }
}
impl StocksRuntime {
    pub(in crate::panels) fn module_runtime_state(self) -> ModuleRuntimeState {
        let operation = if self.batch.journal.busy.get() {
            ModuleRuntimeState::from_action_state(&shared_types::ActionState::pending("正在保存股票批量监控"))
        } else if self.batch.journal.locked() {
            ModuleRuntimeState::from_action_state(&shared_types::ActionState::accepted("股票批量监控保存待核对"))
        } else { ModuleRuntimeState::ready() };
        let monitor = if self.monitor.busy.get() {
            ModuleRuntimeState::from_action_state(&shared_types::ActionState::pending("正在保存单股监控"))
        } else if self.monitor.locked() {
            ModuleRuntimeState::from_action_state(&shared_types::ActionState::accepted("单股监控修改待核对"))
        } else { ModuleRuntimeState::ready() };
        let plan = if self.plan.busy.get() {
            ModuleRuntimeState::from_action_state(&shared_types::ActionState::pending("正在构建股票计划"))
        } else if self.plan.locked() {
            ModuleRuntimeState::from_action_state(&shared_types::ActionState::accepted("股票计划构建待核对"))
        } else { ModuleRuntimeState::ready() };
        let peer_plan = if self.peer_plan.busy.get() {
            ModuleRuntimeState::from_action_state(&shared_types::ActionState::pending("正在核对股票双边计划"))
        } else if self.peer_plan.locked() {
            ModuleRuntimeState::from_action_state(&shared_types::ActionState::accepted("股票双边计划构建待核对"))
        } else { ModuleRuntimeState::ready() };
        let funds = self.market.with(|m|m.value().map(|s| {
            let problem=s.plan_problem.as_ref().or(s.peer_plan_problem.as_ref())
                .or(s.funding_problem.as_ref()).or(s.stablecoin_problem.as_ref()).or(s.exchange_conversion_problem.as_ref());
            if let Some(problem)=problem {
                ModuleRuntimeState::from_problem(Some(shared_types::ApiProblem::new("STOCK_FUNDS_READ_FAILED",problem.clone())))
            } else if let Some((label,_))=super::readiness::funds_status(s,super::super::timestamp::now_ms()) {
                ModuleRuntimeState::from_action_state(&shared_types::ActionState::accepted(label))
            } else { ModuleRuntimeState::ready() }
        }).unwrap_or_else(ModuleRuntimeState::ready));
        ModuleRuntimeState::combine([operation, monitor, plan, peer_plan, funds, self.market_runtime_state(), self.catalog_status.get()])
    }

    fn market_runtime_state(self) -> ModuleRuntimeState {
        if let Some(batch)=self.market.with(|v|v.value()
            .filter(|s|s.security.is_none() &&s.batch.request.as_ref().is_some_and(|r|r.enabled))
            .map(|s|s.batch.clone())) {
            let mut state=ModuleRuntimeState::from_load_state(&self.market.get());
            if state.status==ModuleRuntimeStatus::Ready {
                let now=super::super::timestamp::now_ms();
                state.status=if batch.problem.is_some() ||batch.waiting_for_viewers {ModuleRuntimeStatus::Stale}
                    else if batch.rows.is_empty() {ModuleRuntimeStatus::Loading}
                    else if batch.rows.iter().all(|r|r.token_price(true,now).is_some() &&r.token_price(false,now).is_some()) {ModuleRuntimeStatus::Ready}
                    else if batch.running &&batch.completed_rounds==0 {ModuleRuntimeStatus::Loading}
                    else {ModuleRuntimeStatus::Stale};
            }
            return state;
        }
        if self
            .market
            .with(|v| v.value().is_some_and(|s| s.security.is_none()))
        {
            ModuleRuntimeState::setup_required()
        } else {
            let mut state = ModuleRuntimeState::from_load_state(&self.market.get());
            if state.status == ModuleRuntimeStatus::Ready
                && self.market.with(|v| {
                    v.value().is_some_and(|s| {
                        !s.connected
                            || s.problem.is_some()
                            || (s.monitor.enabled && s.monitor.phase == StockMonitorPhase::Backoff)
                    })
                })
            {
                state.status = ModuleRuntimeStatus::Stale;
            }
            if state.status == ModuleRuntimeStatus::Ready
                && self.market.with(|v| {
                    v.value()
                        .is_some_and(|s| s.books.is_empty() && s.reference.is_none())
                })
            {
                state.status = ModuleRuntimeStatus::Loading;
            }
            state
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct StockData {
    pub batch: BatchData,
    pub section: RwSignal<u8>,
    pub peers: PeerData,
    pub market: RwSignal<LoadState<StockMarketSnapshot>>,
    pub catalog: RwSignal<LoadState<StockCatalog>>,
    pub search: RwSignal<String>,
    pub page: RwSignal<usize>,
    pub pending: RwSignal<bool>,
    pub notice: Notice,
    pub clock: RwSignal<i64>,
    pub watch: Callback<Option<String>>,
    pub refresh: Callback<()>,
    pub budget: RwSignal<String>,
    pub keyed: RwSignal<bool>,
    pub quote_pending: RwSignal<bool>,
    pub quote: Callback<String>,
    pub monitor_pending: RwSignal<bool>,
    pub monitor: Callback<(String, bool)>,
    pub monitor_journal: OperationJournal,
    pub monitor_recheck: Callback<()>,
    pub rfq: RfqData,
    pub preflight: PreflightData,
    pub alerts: AlertData,
}

impl StockData {
    pub(super) fn quote_draft_problem(self) -> Option<&'static str> {
        if self.quote_pending.get() || self.monitor_pending.get() {
            return Some("正在更新询价，等待当前参数的完整结果");
        }
        self.market.with(|m| m.value().map(|s|
            super::readiness::quote_draft_problem(s, &self.budget.get(), self.keyed.get()))
            .unwrap_or(Some("股票行情尚未就绪")))
    }
}

pub(super) fn use_data(runtime: StocksRuntime) -> StockData {
    let client = use_global().client;
    let market = runtime.market;
    let catalog = RwSignal::new(LoadState::Loading);
    Effect::new(move |_| {
        runtime.catalog_status.set(catalog.with(ModuleRuntimeState::from_load_state));
    });
    let pending = RwSignal::new(false);
    let loading_catalog = RwSignal::new(false);
    let notice = Notice::new();
    let batch=batch::use_batch(runtime.batch,market,catalog);
    let transport = RwSignal::new(WsChannelState::new("stocks"));
    let handle = start_stock_stream_with_state(
        transport,
        move |snapshot| {
            apply_snapshot(market, snapshot);
        },
        move |problem| {
            market.try_update(|v| v.apply_result(Err(problem)));
        },
    );
    on_cleanup(move || handle.cancel());
    let clock = RwSignal::new(super::super::timestamp::now_ms());
    if let Ok(handle) = set_interval_with_handle(
        move || {
            clock.try_set(super::super::timestamp::now_ms());
        },
        std::time::Duration::from_secs(1),
    ) {
        on_cleanup(move || handle.clear());
    }
    let refresh = Callback::new({
        move |_| {
            if loading_catalog.get_untracked() {
                return;
            }
            loading_catalog.set(true);
            let client = batch.journal.client();
            let connection = batch.journal.connection.get_untracked();
            spawn_local(async move {
                let result = client.stock_catalog().await.map_err(|e| e.problem);
                if batch.journal.connection.try_get_untracked() != Some(connection) { return; }
                catalog.try_update(|v| v.apply_result(result));
                loading_catalog.try_set(false);
            });
        }
    });
    Effect::new(move |_| {
        batch.journal.connection.track();
        catalog.set(LoadState::Loading);
        loading_catalog.set(false);
        refresh.run(());
    });
    let budget = RwSignal::new("10".to_owned());
    let keyed = RwSignal::new(false);
    let quote_pending = RwSignal::new(false);
    let wallet = RwSignal::new(String::new());
    let selection = RwSignal::new(0_u64);
    let peers = peers::use_peers(market, budget, keyed, wallet, selection, runtime.peer_plan);
    let alerts = AlertData::new(market);
    let applied = StoredValue::new(None::<StockQuoteRequest>);
    Effect::new(move |_| {
        let incoming = market.with(|m| m.value().and_then(|s| s.monitor.request.clone()));
        if applied.with_value(|old| old != &incoming) {
            if let Some(request) = incoming.as_ref() {
                budget.set(request.budget_usdc.clone());
                keyed.set(request.keyed);
            }
            applied.set_value(incoming);
        }
    });
    let monitoring = monitor::use_monitor(runtime.monitor, market, pending, quote_pending, budget, keyed, alerts, notice);
    let monitor_pending = monitoring.pending;
    let quote = Callback::new({
        move |asset: String| {
            if quote_pending.get_untracked() || pending.get_untracked() || monitor_pending.get_untracked() {
                return;
            }
            quote_pending.set(true);
            notice.set(None);
            let client = batch.journal.client();
            let connection = batch.journal.connection.get_untracked();
            let selected = selection.get_untracked();
            let request = StockQuoteRequest {
                asset,
                budget_usdc: budget.get_untracked(),
                keyed: keyed.get_untracked(),
            };
            spawn_local(async move {
                let result = client.quote_stock(&request).await;
                if selection.try_get_untracked() != Some(selected)
                    || batch.journal.connection.try_get_untracked() != Some(connection) { return; }
                let same_asset = market
                    .try_with(|s| {
                        s.value()
                            .and_then(|m| m.security.as_ref())
                            .is_some_and(|s| s.asset == request.asset)
                    })
                    .unwrap_or(false);
                if same_asset {
                    match result {
                        Ok(snapshot) => {
                            if snapshot.security.as_ref().is_some_and(|s|s.asset == request.asset)
                                && snapshot.comparison.as_ref().is_some_and(|q|q.asset == request.asset
                                    && q.budget_usdc == request.budget_usdc && q.keyed == request.keyed) {
                                apply_snapshot(market, snapshot);
                            } else {
                                notice.try_set(Some("询价回复与当前股票或参数不匹配，已保留原状态".into()));
                            }
                        }
                        Err(error) => {
                            notice.try_set(Some(error.problem.message));
                        }
                    }
                }
                quote_pending.try_set(false);
            });
        }
    });
    let rfq = rfq::use_rfq(client.clone(), market, notice);
    let section = RwSignal::new(6);
    let preflight = preflight::use_preflight(market, notice, budget, keyed, wallet, section,
        runtime.plan, selection, pending, quote_pending, monitor_pending);
    Effect::new(move |_| {
        batch.journal.connection.track();
        pending.set(false);
        quote_pending.set(false);
        selection.update(|value|*value=value.wrapping_add(1));
    });
    let watch = Callback::new(move |asset: Option<String>| {
        if pending.get_untracked() {
            return;
        }
        pending.set(true);
        selection.update(|value|*value=value.wrapping_add(1));
        quote_pending.set(false);
        notice.set(None);
        let client = batch.journal.client();
        let connection = batch.journal.connection.get_untracked();
        let original_draft = (budget.get_untracked(), keyed.get_untracked());
        let selection_changed = market.with_untracked(|m|
            m.value().and_then(|s| s.security.as_ref()).map(|s| &s.asset) != asset.as_ref());
        spawn_local(async move {
            let result = client.watch_stock(asset.clone()).await;
            if pending.try_get_untracked() != Some(true)
                || batch.journal.connection.try_get_untracked() != Some(connection) { return; }
            match result {
                Ok(snapshot) => {
                    let matches = snapshot.security.as_ref().map(|s| &s.asset) == asset.as_ref();
                    // A newer WS frame for the requested stock can arrive before its HTTP receipt.
                    let (compatible, inherited) = market.try_with_untracked(|m| {
                        let latest = m.value().filter(|s| s.observed_at_ms > snapshot.observed_at_ms)
                            .unwrap_or(&snapshot);
                        let compatible = latest.security.as_ref().map(|s| &s.asset) == asset.as_ref();
                        let inherited = asset.as_ref().and_then(|asset| latest.batch.request.as_ref()
                            .filter(|r| r.assets.contains(asset) && latest.monitor.request.is_none())
                            .map(|r| (r.budget_usdc.clone(), r.keyed)));
                        (compatible, inherited)
                    }).unwrap_or((false, None));
                    if matches && compatible {
                        if selection_changed && (budget.get_untracked(), keyed.get_untracked()) == original_draft {
                            if let Some((amount, source)) = inherited {
                                budget.set(amount);
                                keyed.set(source);
                            }
                        }
                        apply_snapshot(market, snapshot);
                    } else {
                        notice.try_set(Some("股票选择处理结果与当前状态不一致，保留当前详情与询价参数".into()));
                    }
                }
                Err(error) => {
                    notice.try_set(Some(error.problem.message));
                }
            }
            pending.try_set(false);
        });
    });
    StockData {
        batch,
        section,
        peers,
        market,
        catalog,
        search: RwSignal::new(String::new()),
        page: RwSignal::new(0),
        pending,
        notice,
        clock,
        watch,
        refresh,
        budget,
        keyed,
        quote_pending,
        quote,
        monitor_pending,
        monitor: monitoring.apply,
        monitor_journal: runtime.monitor,
        monitor_recheck: monitoring.recheck,
        rfq,
        preflight,
        alerts,
    }
}

fn apply_snapshot(market: RwSignal<LoadState<StockMarketSnapshot>>, snapshot: StockMarketSnapshot) {
    market.try_update(|state| {
        if state
            .value()
            .is_none_or(|current| current.observed_at_ms <= snapshot.observed_at_ms)
        {
            state.apply_result(Ok(snapshot));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stock_snapshot_does_not_restore_a_stopped_watch_from_delayed_frames() {
        Owner::new().with(|| {
            let market = RwSignal::new(LoadState::Ready(StockMarketSnapshot {
                observed_at_ms: 200,
                ..Default::default()
            }));
            apply_snapshot(
                market,
                StockMarketSnapshot {
                    connected: true,
                    observed_at_ms: 100,
                    ..Default::default()
                },
            );
            assert!(!market.get_untracked().value().unwrap().connected);
            apply_snapshot(
                market,
                StockMarketSnapshot {
                    connected: true,
                    observed_at_ms: 201,
                    ..Default::default()
                },
            );
            assert!(market.get_untracked().value().unwrap().connected);
        });
    }
}
