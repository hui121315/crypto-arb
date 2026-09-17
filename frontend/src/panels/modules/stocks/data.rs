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
pub(in crate::panels::modules::stocks) mod preflight;
mod rfq;
mod alerts;
mod peers;
pub(super) use peers::PeerData;
pub(super) use alerts::AlertData;
pub(super) use preflight::PreflightData;
pub(super) use rfq::RfqData;

#[derive(Clone, Copy)]
pub(in crate::panels) struct StocksRuntime {
    pub(super) market: RwSignal<LoadState<StockMarketSnapshot>>,
}
pub(in crate::panels) fn create_stocks_runtime() -> StocksRuntime {
    StocksRuntime {
        market: RwSignal::new(LoadState::Loading),
    }
}
impl StocksRuntime {
    pub(in crate::panels) fn module_runtime_state(self) -> ModuleRuntimeState {
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
    pub peers: PeerData,
    pub market: RwSignal<LoadState<StockMarketSnapshot>>,
    pub catalog: RwSignal<LoadState<StockCatalog>>,
    pub search: RwSignal<String>,
    pub page: RwSignal<usize>,
    pub pending: RwSignal<bool>,
    pub notice: RwSignal<Option<String>>,
    pub clock: RwSignal<i64>,
    pub watch: Callback<Option<String>>,
    pub refresh: Callback<()>,
    pub budget: RwSignal<String>,
    pub keyed: RwSignal<bool>,
    pub quote_pending: RwSignal<bool>,
    pub quote: Callback<String>,
    pub monitor_pending: RwSignal<bool>,
    pub monitor: Callback<(String, bool)>,
    pub rfq: RfqData,
    pub preflight: PreflightData,
    pub alerts: AlertData,
}

pub(super) fn use_data(runtime: StocksRuntime) -> StockData {
    let client = use_global().client;
    let market = runtime.market;
    let peers = peers::use_peers(market);
    let catalog = RwSignal::new(LoadState::Loading);
    let pending = RwSignal::new(false);
    let loading_catalog = RwSignal::new(false);
    let notice = RwSignal::new(None);
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
        let client = client.clone();
        move |_| {
            if loading_catalog.get_untracked() {
                return;
            }
            loading_catalog.set(true);
            let client = client.clone();
            spawn_local(async move {
                let result = client.stock_catalog().await.map_err(|e| e.problem);
                catalog.try_update(|v| v.apply_result(result));
                loading_catalog.try_set(false);
            });
        }
    });
    refresh.run(());
    let budget = RwSignal::new("10".to_owned());
    let keyed = RwSignal::new(false);
    let quote_pending = RwSignal::new(false);
    let monitor_pending = RwSignal::new(false);
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
    let monitor = Callback::new({
        let client = client.clone();
        move |(asset, enabled): (String, bool)| {
            if monitor_pending.get_untracked() || pending.get_untracked() {
                return;
            }
            let alert_config = match alerts.config(enabled) {
                Ok(config) => config,
                Err(e) => { notice.set(Some(e)); return; }
            };
            monitor_pending.set(true);
            notice.set(None);
            let request = StockMonitorRequest {
                enabled,
                alerts: alert_config,
                quote: StockQuoteRequest {
                    asset: asset.clone(),
                    budget_usdc: budget.get_untracked(),
                    keyed: keyed.get_untracked(),
                },
            };
            let client = client.clone();
            spawn_local(async move {
                let result = client.monitor_stock(&request).await;
                if market
                    .try_with(|m| {
                        m.value()
                            .and_then(|s| s.security.as_ref())
                            .is_some_and(|s| s.asset == asset)
                    })
                    .unwrap_or(false)
                {
                    match result {
                        Ok(snapshot) => apply_snapshot(market, snapshot),
                        Err(error) => {
                            notice.try_set(Some(error.problem.message));
                        }
                    }
                }
                monitor_pending.try_set(false);
            });
        }
    });
    let quote = Callback::new({
        let client = client.clone();
        move |asset: String| {
            if quote_pending.get_untracked() || pending.get_untracked() {
                return;
            }
            quote_pending.set(true);
            notice.set(None);
            let client = client.clone();
            let request = StockQuoteRequest {
                asset,
                budget_usdc: budget.get_untracked(),
                keyed: keyed.get_untracked(),
            };
            spawn_local(async move {
                let result = client.quote_stock(&request).await;
                let same_asset = market
                    .try_with(|s| {
                        s.value()
                            .and_then(|m| m.security.as_ref())
                            .is_some_and(|s| s.asset == request.asset)
                    })
                    .unwrap_or(false);
                if same_asset {
                    match result {
                        Ok(snapshot) => apply_snapshot(market, snapshot),
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
    let preflight = preflight::use_preflight(client.clone(), market, notice, budget, keyed);
    let watch = Callback::new(move |asset: Option<String>| {
        if pending.get_untracked() {
            return;
        }
        pending.set(true);
        notice.set(None);
        let client = client.clone();
        spawn_local(async move {
            match client.watch_stock(asset).await {
                Ok(snapshot) => {
                    apply_snapshot(market, snapshot);
                }
                Err(error) => {
                    notice.try_set(Some(error.problem.message));
                }
            }
            pending.try_set(false);
        });
    });
    StockData {
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
        monitor,
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
