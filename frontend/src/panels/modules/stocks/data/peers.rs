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
    pub(super) fn query(self) -> StockPeerCatalogRequest {
        StockPeerCatalogRequest {
            venue: self.venue.get(), product: self.product.get(), search: self.search.get(),
        }
    }

    pub(in crate::panels::modules::stocks) fn current_catalog(self) -> Option<StockPeerCatalog> {
        self.catalog.get().filter(|c| c.request == self.query())
    }

    pub(in crate::panels::modules::stocks) fn defaults(journal: OperationJournal) -> Self {
        Self {
            plans: plan::PeerPlanData::defaults(journal),
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

pub(super) fn use_peers(market: RwSignal<LoadState<StockMarketSnapshot>>, budget: RwSignal<String>, keyed: RwSignal<bool>, wallet: RwSignal<String>, stock_generation: RwSignal<u64>, journal: OperationJournal) -> PeerData {
    let mut data = PeerData::defaults(journal);
    data.plans = plan::use_plans(market, budget, keyed, journal);
    let select_revision = RwSignal::new(0_u64);
    let checks_revision = RwSignal::new(0_u64);
    let scope = crate::state::read_scope::ReadScope::new(move || {
        data.catalog.set(None);
        data.problem.set(None);
        data.loading.set(false);
        data.pending.set(false);
        data.checking.set(false);
        data.funding_checking.set(false);
        data.order_checking.set(false);
        select_revision.update(|n| *n = n.wrapping_add(1));
        checks_revision.update(|n| *n = n.wrapping_add(1));
    });
    let catalog_read = scope.request();
    data.refresh = Callback::new(move |_| {
        let request = untrack(move || data.query());
        let for_read = request.clone();
        data.loading.set(true);
        data.problem.set(None);
        data.catalog.set(None);
        catalog_read.run(move |client| async move {
            crate::state::read_scope::bounded_read(client.stock_peer_markets(&for_read)).await
        }, move |result| {
            if untrack(move || data.query()) == request {
                match result {
                    Ok(c) if c.request == request && c.rows.len() <= c.matched && c.matched <= c.registry_count
                        && c.rows.iter().all(|r| r.venue == request.venue && !r.native_symbol.is_empty()
                            && request.product.matches(r.product_type.as_deref())) => { data.catalog.set(Some(c)); }
                    Ok(_) => data.problem.set(Some("市场目录回复与搜索条件不一致，请重新搜索".into())),
                    Err(e) => data.problem.set(Some(e.message)),
                }
            }
            data.loading.set(false);
        });
    });
    let context = Memo::new(move |_| market.with(|m|m.value().and_then(|s|
        Some((s.security.as_ref()?.asset.clone(),s.peer.as_ref()?.selection.clone())))));
    Effect::new(move |_| {
        context.get();
        stock_generation.track();
        checks_revision.update(|n|*n=n.wrapping_add(1));
        data.problem.set(None);
        data.checking.set(false);
        data.funding_checking.set(false);
        data.order_checking.set(false);
    });
    Effect::new(move |_| {
        stock_generation.track();
        select_revision.update(|n|*n=n.wrapping_add(1));
        data.pending.set(false);
    });
    data.funding=Callback::new(move |_| {
        if data.funding_checking.get_untracked() || data.checking.get_untracked() || data.order_checking.get_untracked() ||data.pending.get_untracked() {return;}
        let request=market.with_untracked(|m|m.value().and_then(|s|Some(StockPeerFundingRequest{
            asset:s.security.as_ref()?.asset.clone(),selection:s.peer.as_ref()?.selection.clone()})));
        let Some(request)=request else {return};
        data.funding_checking.set(true);data.problem.set(None);
        let source=scope.capture();let client=source.client();
        let revision=checks_revision.get_untracked();
        spawn_local(async move {
            let result=client.stock_peer_funding(&request).await;
            if !scope.accepts(&source) || checks_revision.try_get_untracked()!=Some(revision) {return;}
            if market.try_with_untracked(|m|m.value().is_some_and(|s|s.security.as_ref().is_some_and(|s|s.asset==request.asset)
                && s.peer.as_ref().is_some_and(|p|p.selection==request.selection))).unwrap_or(false) {
                match result {
                    Ok(s) if matches_peer(&s,&request.asset,&request.selection)
                        &&s.peer_funding.as_ref().is_some_and(|r|r.asset==request.asset &&r.selection==request.selection)=>apply_snapshot(market,s),
                    Ok(_)=>{data.problem.try_set(Some("充提回复与当前股票或交易对不一致，未采用旧资料".into()));},
                    Err(e)=>{data.problem.try_set(Some(e.problem.message));}
                }
            }
            data.funding_checking.try_set(false);
        });
    });
    data.preflight=Callback::new(move |wallet_address:Option<String>| {
        if data.checking.get_untracked() ||data.funding_checking.get_untracked() || data.order_checking.get_untracked() ||data.pending.get_untracked() {return;}
        let wallet_address=wallet_address.map(|w|w.trim().to_owned()).filter(|w|!w.is_empty());
        let request=market.with_untracked(|m|m.value().and_then(|s|Some(StockPeerPreflightRequest {
            asset:s.security.as_ref()?.asset.clone(),selection:s.peer.as_ref()?.selection.clone(),wallet_address})));
        let Some(request)=request else {return};
        data.checking.set(true);data.problem.set(None);
        let source=scope.capture();let client=source.client();
        let revision=checks_revision.get_untracked();
        spawn_local(async move {
            let result=client.stock_peer_preflight(&request).await;
            if !scope.accepts(&source) || checks_revision.try_get_untracked()!=Some(revision) {return;}
            if market.try_with_untracked(|m|m.value().is_some_and(|s|s.security.as_ref().is_some_and(|s|s.asset==request.asset)
                && s.peer.as_ref().is_some_and(|p|p.selection==request.selection))).unwrap_or(false)
                &&wallet.try_with_untracked(|w|w.trim()==request.wallet_address.as_deref().unwrap_or_default())==Some(true) {
                match result {
                    Ok(s) if matches_peer(&s,&request.asset,&request.selection)
                        &&s.peer_preflight.as_ref().is_some_and(|r|r.asset==request.asset &&r.selection==request.selection
                            &&r.account.as_ref().is_none_or(|a|a.venue==request.selection.venue &&a.native_symbol==request.selection.native_symbol)
                            &&r.wallet.as_ref().is_none_or(|w|Some(w.owner.as_str())==request.wallet_address.as_deref()))=>apply_snapshot(market,s),
                    Ok(_)=>{data.problem.try_set(Some("账户交易检查回复与当前股票、交易对或钱包不一致，未采用旧余额".into()));},
                    Err(e)=>{data.problem.try_set(Some(e.problem.message));}
                }
            }
            data.checking.try_set(false);
        });
    });
    data.order_check=Callback::new(move |direction| {
        if data.order_checking.get_untracked() || data.checking.get_untracked() || data.funding_checking.get_untracked() || data.pending.get_untracked() {return;}
        if let Some(problem) = market.with_untracked(|m|m.value().and_then(|s|
            super::super::readiness::quote_draft_problem(s, &budget.get_untracked(), keyed.get_untracked()))) {
            data.problem.set(Some(problem.into())); return;
        }
        let request=market.with_untracked(|m|m.value().and_then(|s|Some(StockPeerOrderCheckRequest {
            asset:s.security.as_ref()?.asset.clone(),selection:s.peer.as_ref()?.selection.clone(),direction})));
        let Some(request)=request else {return};
        data.order_checking.set(true);data.problem.set(None);
        let source=scope.capture();let client=source.client();
        let revision=checks_revision.get_untracked();
        let inputs=(budget.get_untracked(),keyed.get_untracked());
        spawn_local(async move {
            let result=client.stock_peer_order_check(&request).await;
            if !scope.accepts(&source) || checks_revision.try_get_untracked()!=Some(revision) {return;}
            if market.try_with_untracked(|m|m.value().is_some_and(|s|s.security.as_ref().is_some_and(|s|s.asset==request.asset)
                && s.peer.as_ref().is_some_and(|p|p.selection==request.selection))).unwrap_or(false)
                &&budget.try_get_untracked().zip(keyed.try_get_untracked())==Some(inputs) {
                match result {
                    Ok(s) if matches_peer(&s,&request.asset,&request.selection)
                        &&s.peer_order_checks.iter().any(|r|r.draft.request==request)=>apply_snapshot(market,s),
                    Ok(_)=>{data.problem.try_set(Some("订单验证回复与当前方向或市场不一致，未认定通过".into()));},
                    Err(e)=>{data.problem.try_set(Some(e.problem.message));}
                }
            }
            data.order_checking.try_set(false);
        });
    });
    data.select = Callback::new(move |selection:Option<StockPeerSelection>| {
        if data.pending.get_untracked() {
            return;
        }
        let Some(asset) = market.with_untracked(|m| {
            m.value()
                .and_then(|s| s.security.as_ref().map(|s| s.asset.clone()))
        }) else {
            return;
        };
        if let Some(selected)=selection.as_ref() {
            if untrack(move ||data.current_catalog()).is_none_or(|c|c.request.product!=selected.product
                ||c.rows.iter().all(|r|r.venue!=selected.venue ||r.native_symbol!=selected.native_symbol)) {
                data.problem.set(Some("搜索条件已变化，请重新搜索后选择市场".into()));return;
            }
        }
        data.pending.set(true);
        data.problem.set(None);
        select_revision.update(|n|*n=n.wrapping_add(1));
        let revision=select_revision.get_untracked();
        let source=scope.capture();let client=source.client();
        spawn_local(async move {
            let result = client
                .watch_stock_peer(&StockPeerWatchRequest {
                    asset: asset.clone(),
                    selection:selection.clone(),
                })
                .await;
            if !scope.accepts(&source) ||select_revision.try_get_untracked()!=Some(revision) {return;}
            if market
                .try_with_untracked(|m| {
                    m.value()
                        .and_then(|s| s.security.as_ref())
                        .is_some_and(|s| s.asset == asset)
                })
                .unwrap_or(false)
            {
                match result {
                    Ok(s) if s.security.as_ref().is_some_and(|s|s.asset==asset)
                        &&s.peer.as_ref().map(|p|&p.selection)==selection.as_ref()=>apply_snapshot(market, s),
                    Ok(_) => {data.problem.try_set(Some("市场选择回复与当前选择不一致，已保留原对比".into()));},
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
        scope.track();
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

fn matches_peer(snapshot:&StockMarketSnapshot,asset:&str,selection:&StockPeerSelection)->bool {
    snapshot.security.as_ref().is_some_and(|s|s.asset==asset)
        &&snapshot.peer.as_ref().is_some_and(|p|&p.selection==selection)
}
