use super::*;
mod plan;

#[derive(Clone, Copy)]
pub(in crate::panels::modules::stocks) struct PeerData {
    pub plans: plan::PeerPlanData,
    pub venue: RwSignal<String>,
    pub product: RwSignal<StockPeerProduct>,
    pub search: RwSignal<String>,
    pub catalog: RwSignal<Option<StockPeerCatalog>>,
    pub loading: RwSignal<bool>,
    pub pending: RwSignal<bool>,
    pub problem: RwSignal<Option<String>>,
    pub refresh: Callback<()>,
    pub select: Callback<Option<StockPeerSelection>>,
    pub checking: RwSignal<bool>,
    pub preflight: Callback<Option<String>>,
    pub funding_checking: RwSignal<bool>,
    pub funding: Callback<()>,
    pub order_checking: RwSignal<bool>,
    pub order_check: Callback<StockChainDirection>,
}

impl PeerData {
    pub(in crate::panels::modules::stocks) fn defaults() -> Self {
        Self {
            plans: plan::PeerPlanData::defaults(),
            venue: RwSignal::new("kraken".into()),
            product: RwSignal::new(StockPeerProduct::Spot),
            search: RwSignal::new(String::new()),
            catalog: RwSignal::new(None),
            loading: RwSignal::new(false),
            pending: RwSignal::new(false),
            problem: RwSignal::new(None),
            refresh: Callback::new(|_| {}),
            select: Callback::new(|_| {}),
            checking: RwSignal::new(false),
            preflight: Callback::new(|_| {}),
            funding_checking: RwSignal::new(false),
            funding: Callback::new(|_| {}),
            order_checking: RwSignal::new(false),
            order_check: Callback::new(|_| {}),
        }
    }
}

pub(super) fn use_peers(market: RwSignal<LoadState<StockMarketSnapshot>>) -> PeerData {
    let client = use_global().client;
    let mut data = PeerData::defaults();
    data.plans = plan::use_plans(market);
    let serial = StoredValue::new(0_u64);
    data.refresh = Callback::new({
        let client = client.clone();
        move |_| {
            let request = StockPeerCatalogRequest {
                venue: data.venue.get_untracked(),
                product: data.product.get_untracked(),
                search: data.search.get_untracked(),
            };
            serial.update_value(|s| *s += 1);
            let current = serial.get_value();
            data.loading.set(true);
            data.problem.set(None);
            data.catalog.set(None);
            let client = client.clone();
            spawn_local(async move {
                let result = client.stock_peer_markets(&request).await;
                if serial.try_get_value() != Some(current) {
                    return;
                }
                match result {
                    Ok(c) => {
                        data.catalog.try_set(Some(c));
                    }
                    Err(e) => {
                        data.problem.try_set(Some(e.problem.message));
                    }
                }
                data.loading.try_set(false);
            });
        }
    });
    let funding_client=client.clone();
    data.funding=Callback::new(move |_| {
        if data.funding_checking.get_untracked() || data.checking.get_untracked() || data.order_checking.get_untracked() {return;}
        let request=market.with_untracked(|m|m.value().and_then(|s|Some(StockPeerFundingRequest{
            asset:s.security.as_ref()?.asset.clone(),selection:s.peer.as_ref()?.selection.clone()})));
        let Some(request)=request else {return};
        data.funding_checking.set(true);data.problem.set(None);
        let client=funding_client.clone();
        spawn_local(async move {
            let result=client.stock_peer_funding(&request).await;
            if market.try_with_untracked(|m|m.value().is_some_and(|s|s.security.as_ref().is_some_and(|s|s.asset==request.asset)
                && s.peer.as_ref().is_some_and(|p|p.selection==request.selection))).unwrap_or(false) {
                match result {Ok(s)=>apply_snapshot(market,s),Err(e)=>{data.problem.try_set(Some(e.problem.message));}}
            }
            data.funding_checking.try_set(false);
        });
    });
    let peer_client=client.clone();
    data.preflight=Callback::new(move |wallet_address| {
        if data.checking.get_untracked() ||data.funding_checking.get_untracked() || data.order_checking.get_untracked() {return;}
        let request=market.with_untracked(|m|m.value().and_then(|s|Some(StockPeerPreflightRequest {
            asset:s.security.as_ref()?.asset.clone(),selection:s.peer.as_ref()?.selection.clone(),wallet_address})));
        let Some(request)=request else {return};
        data.checking.set(true);data.problem.set(None);
        let client=peer_client.clone();
        spawn_local(async move {
            let result=client.stock_peer_preflight(&request).await;
            if market.try_with_untracked(|m|m.value().is_some_and(|s|s.security.as_ref().is_some_and(|s|s.asset==request.asset)
                && s.peer.as_ref().is_some_and(|p|p.selection==request.selection))).unwrap_or(false) {
                match result {Ok(s)=>apply_snapshot(market,s),Err(e)=>{data.problem.try_set(Some(e.problem.message));}}
            }
            data.checking.try_set(false);
        });
    });
    let order_client=client.clone();
    data.order_check=Callback::new(move |direction| {
        if data.order_checking.get_untracked() || data.checking.get_untracked() || data.funding_checking.get_untracked() || data.pending.get_untracked() {return;}
        let request=market.with_untracked(|m|m.value().and_then(|s|Some(StockPeerOrderCheckRequest {
            asset:s.security.as_ref()?.asset.clone(),selection:s.peer.as_ref()?.selection.clone(),direction})));
        let Some(request)=request else {return};
        data.order_checking.set(true);data.problem.set(None);
        let client=order_client.clone();
        spawn_local(async move {
            let result=client.stock_peer_order_check(&request).await;
            if market.try_with_untracked(|m|m.value().is_some_and(|s|s.security.as_ref().is_some_and(|s|s.asset==request.asset)
                && s.peer.as_ref().is_some_and(|p|p.selection==request.selection))).unwrap_or(false) {
                match result {Ok(s)=>apply_snapshot(market,s),Err(e)=>{data.problem.try_set(Some(e.problem.message));}}
            }
            data.order_checking.try_set(false);
        });
    });
    data.select = Callback::new(move |selection| {
        if data.pending.get_untracked() {
            return;
        }
        let Some(asset) = market.with_untracked(|m| {
            m.value()
                .and_then(|s| s.security.as_ref().map(|s| s.asset.clone()))
        }) else {
            return;
        };
        data.pending.set(true);
        data.problem.set(None);
        let client = client.clone();
        spawn_local(async move {
            let result = client
                .watch_stock_peer(&StockPeerWatchRequest {
                    asset: asset.clone(),
                    selection,
                })
                .await;
            if market
                .try_with_untracked(|m| {
                    m.value()
                        .and_then(|s| s.security.as_ref())
                        .is_some_and(|s| s.asset == asset)
                })
                .unwrap_or(false)
            {
                match result {
                    Ok(s) => apply_snapshot(market, s),
                    Err(e) => {
                        data.problem.try_set(Some(e.problem.message));
                    }
                }
            }
            data.pending.try_set(false);
        });
    });
    let asset = Memo::new(move |_| {
        market.with(|m| {
            m.value()
                .and_then(|s| s.security.as_ref())
                .map(|s| (s.asset.clone(), s.ticker.clone()))
        })
    });
    Effect::new(move |_| {
        if let Some((_, ticker)) = asset.get() {
            let selected = market.with_untracked(|m| {
                m.value()
                    .and_then(|s| s.peer.as_ref().map(|p| p.selection.clone()))
            });
            if let Some(selected) = selected {
                data.venue.set(selected.venue);
                data.product.set(selected.product);
            }
            data.search.set(ticker);
            data.refresh.run(());
        }
    });
    data
}
