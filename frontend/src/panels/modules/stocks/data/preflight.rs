use super::*;
mod build;
mod funding;
mod withdrawal;
mod stablecoin;
pub(in crate::panels::modules::stocks) mod exchange_conversion;

#[derive(Clone, Copy)]
pub(in crate::panels::modules::stocks) struct PreflightData {
    pub stablecoin: stablecoin::StablecoinData,
    pub conversion: exchange_conversion::ConversionData,
    pub wallet: RwSignal<String>,
    pub pending: RwSignal<bool>,
    pub conversion_cost_ids: RwSignal<Vec<String>>,
    pub selected_plan: RwSignal<Option<String>>,
    pub selected_funding_plan: RwSignal<Option<String>>,
    pub read: Callback<String>,
    pub restock: Callback<StockPreflightRequest>,
    pub deposit_address: Callback<String>,
    pub funding_build: Callback<StockFundingPlanRequest>,
    pub funding_cancel: Callback<StockPlanRevisionRequest>,
    pub funding_submit: Callback<StockFundingSubmitRequest>,
    pub funding_recheck: Callback<StockPlanCancelRequest>,
    pub funding_prepare_transfer: Callback<StockPlanRevisionRequest>,
    pub build: Callback<(String, StockChainDirection)>,
    pub build_journal: OperationJournal,
    pub build_recheck: Callback<()>,
    pub cancel: Callback<String>,
    pub recheck: Callback<String>,
    pub settle: Callback<StockPlanRevisionRequest>,
    pub topup: Callback<StockPlanRevisionRequest>,
    pub recheck_topup: Callback<StockTopupRecheckRequest>,
    pub execute: Callback<StockPlanExecutionRequest>,
    pub recovery: Callback<StockRecoveryBuildRequest>,
    pub cancel_recovery: Callback<StockRecoveryActionRequest>,
    pub recheck_recovery: Callback<StockRecoveryActionRequest>,
}

pub(super) fn use_preflight(
    market: RwSignal<LoadState<StockMarketSnapshot>>,
    notice: Notice,
    budget: RwSignal<String>,
    keyed: RwSignal<bool>,
    wallet: RwSignal<String>,
    section: RwSignal<u8>,
    build_journal: OperationJournal,
    selection: RwSignal<u64>,
    selecting: RwSignal<bool>,
    quoting: RwSignal<bool>,
    monitoring: RwSignal<bool>,
) -> PreflightData {
    let pending = RwSignal::new(false);
    let reading = RwSignal::new(false);
    let read_scope = crate::state::read_scope::ReadScope::new(move || {
        if reading.get_untracked() { pending.set(false); reading.set(false); }
    });
    let conversion_cost_ids = RwSignal::new(Vec::<String>::new());
    let selected_plan = RwSignal::new(None);
    let selected_funding_plan = RwSignal::new(None);
    let stablecoin = stablecoin::use_stablecoin(market, wallet, pending);
    let conversion = exchange_conversion::use_conversion(market, pending);
    let (funding_build, funding_cancel) = funding::callbacks(market, notice, pending, selected_funding_plan);
    let (funding_submit, funding_recheck, funding_prepare_transfer) = withdrawal::callbacks(market, notice, pending);
    let action_scope = StockSource::new(market, move || {
        pending.set(false);
        notice.set(None);
        conversion_cost_ids.set(Vec::new());
        selected_plan.set(None);
        selected_funding_plan.set(None);
        wallet.set(String::new());
    });
    let restock = Callback::new(move |request: StockPreflightRequest| {
        if pending.get_untracked() ||build_journal.locked() ||selecting.get_untracked() {
            return;
        }
        pending.set(true);
        reading.set(true);
        notice.set(None);
        let source = read_scope.capture();
        let client = source.client();
        let selected = selection.get_untracked();
        let asset = request.asset.clone();
        let owner = request.wallet_address.clone().unwrap_or_default();
        spawn_local(async move {
            let result = client.preflight_stock(&request).await;
            if !read_scope.accepts(&source) { return; }
            if selection.try_get_untracked()==Some(selected) && wallet.try_get_untracked().is_some_and(|w| w.trim() == owner.trim()) && market
                .try_with(|m| {
                    m.value()
                        .and_then(|s| s.security.as_ref())
                        .is_some_and(|s| s.asset == asset)
                })
                .unwrap_or(false)
            {
                match result {
                    Ok(snapshot) if snapshot.security.as_ref().is_some_and(|s|s.asset==asset)
                        &&snapshot.preflight.as_ref().is_some_and(|p|p.asset==asset
                            &&p.wallet_address.as_deref().unwrap_or_default().trim()==owner.trim()
                            &&p.source_plan==request.source_plan) => apply_snapshot(market, snapshot),
                    Ok(_) => {notice.try_set(Some("库存交易检查回复与当前股票或钱包不一致，已保留原数据".into()));}
                    Err(e) => {
                        notice.try_set(Some(e.problem.message));
                    }
                }
            }
            pending.try_set(false);
            reading.try_set(false);
        });
    });
    let read = Callback::new(move |asset: String| restock.run(StockPreflightRequest {
        source_plan: None, asset, wallet_address: Some(wallet.get_untracked()),
    }));
    let deposit_address = Callback::new(move |asset:String| {
        if pending.get_untracked() ||build_journal.locked() ||selecting.get_untracked() {return;}
        pending.set(true);reading.set(true);notice.set(None);
        let source=read_scope.capture();
        let client=source.client();
        let selected=selection.get_untracked();
        spawn_local(async move {
            let result=client.stock_deposit_address(&StockDepositAddressRequest{asset:asset.clone()}).await;
            if !read_scope.accepts(&source) {return;}
            if selection.try_get_untracked()==Some(selected) && market.try_with(|m|m.value().and_then(|s|s.security.as_ref()).is_some_and(|s|s.asset==asset)).unwrap_or(false) {
                match result {
                    Ok(s) if s.security.as_ref().is_some_and(|s|s.asset==asset)
                        &&s.deposit_address.as_ref().is_some_and(|a|a.asset==asset)=>apply_snapshot(market,s),
                    Ok(_)=>{notice.try_set(Some("充币地址回复与所选股票不一致，已保留原数据".into()));},
                    Err(e)=>{notice.try_set(Some(e.problem.message));}
                }
            }
            pending.try_set(false);
            reading.try_set(false);
        });
    });
    let (build,build_recheck)=build::callbacks(build::BuildInput {
        journal:build_journal,market,notice,wallet,budget,keyed,costs:conversion_cost_ids,
        section,selection,pending,selecting,quoting,monitoring,selected_plan,
    });
    let cancel = Callback::new(move |id: String| {
        if pending.get_untracked() {
            return;
        }
        pending.set(true);
        notice.set(None);
        let source = action_scope.capture();
        let client = source.client();
        spawn_local(async move {
            let Some(result) = action_scope.snapshot(&source, client.cancel_stock_plan(&id)).await else { return; };
            match result {
                Ok(snapshot) => {
                    apply_snapshot(market, snapshot);
                }
                Err(e) => {
                    notice.try_set(Some(e.problem.message));
                }
            }
            pending.try_set(false);
        });
    });
    let recheck = Callback::new(move |id:String| {
        if pending.get_untracked() {return;}
        pending.set(true);notice.set(None);
        let source=action_scope.capture();
        let client=source.client();
        spawn_local(async move {
            let Some(result)=action_scope.snapshot(&source,client.recheck_stock_order(&id)).await else {return;};
            match result {
                Ok(snapshot)=>apply_snapshot(market,snapshot),
                Err(e)=>{notice.try_set(Some(e.problem.message));},
            }
            pending.try_set(false);
        });
    });
    let settle = Callback::new(move |request: StockPlanRevisionRequest| {
        if pending.get_untracked() { return; }
        pending.set(true); notice.set(None);
        let source = action_scope.capture();
        let client = source.client();
        spawn_local(async move {
            let Some(result) = action_scope.snapshot(&source, client.settle_stock_plan(&request)).await else { return; };
            match result {
                Ok(snapshot) => apply_snapshot(market, snapshot),
                Err(e) => { notice.try_set(Some(e.problem.message)); }
            }
            pending.try_set(false);
        });
    });
    let topup = Callback::new(move |request: StockPlanRevisionRequest| {
        if pending.get_untracked() { return; }
        pending.set(true); notice.set(None);
        let source = action_scope.capture();
        let client = source.client();
        spawn_local(async move {
            let Some(result) = action_scope.snapshot(&source, client.prepare_stock_topup(&request)).await else { return; };
            match result {
                Ok(snapshot) => apply_snapshot(market, snapshot),
                Err(e) => { notice.try_set(Some(e.problem.message)); }
            }
            pending.try_set(false);
        });
    });
    let recheck_topup = Callback::new(move |request: StockTopupRecheckRequest| {
        if pending.get_untracked() { return; }
        pending.set(true); notice.set(None);
        let source = action_scope.capture();
        let client = source.client();
        spawn_local(async move {
            let Some(result) = action_scope.snapshot(&source, client.recheck_stock_topup(&request)).await else { return; };
            match result {
                Ok(snapshot) => apply_snapshot(market, snapshot),
                Err(e) => { notice.try_set(Some(e.problem.message)); }
            }
            pending.try_set(false);
        });
    });
    let execute = Callback::new(move |request: StockPlanExecutionRequest| {
        if pending.get_untracked() || !request.confirm_live { return; }
        pending.set(true); notice.set(None);
        let source = action_scope.capture();
        let client = source.client();
        spawn_local(async move {
            let Some(result) = action_scope.snapshot(&source, client.execute_stock_plan(&request)).await else { return; };
            match result {
                Ok(snapshot) => {
                    apply_snapshot(market, snapshot);
                    notice.inform("原计划提交状态已更新，成交结果以两腿处理结果为准");
                }
                Err(e) => { notice.try_set(Some(e.problem.message)); }
            }
            pending.try_set(false);
        });
    });
    let recovery = Callback::new(move |request:StockRecoveryBuildRequest| {
        if pending.get_untracked() {return;}
        pending.set(true);notice.set(None);
        let source=action_scope.capture();
        let client=source.client();
        spawn_local(async move {
            let Some(result)=action_scope.snapshot(&source,client.prepare_stock_recovery(&request)).await else {return;};
            match result {
                Ok(snapshot)=>apply_snapshot(market,snapshot),
                Err(e)=>{notice.try_set(Some(e.problem.message));},
            }
            pending.try_set(false);
        });
    });
    let cancel_recovery = Callback::new(move |request:StockRecoveryActionRequest| {
        if pending.get_untracked() {return;}
        pending.set(true);notice.set(None);
        let source=action_scope.capture();
        let client=source.client();
        spawn_local(async move {
            let Some(result)=action_scope.snapshot(&source,client.cancel_stock_recovery(&request)).await else {return;};
            match result {
                Ok(snapshot)=>apply_snapshot(market,snapshot),
                Err(e)=>{notice.try_set(Some(e.problem.message));},
            }
            pending.try_set(false);
        });
    });
    let recheck_recovery = Callback::new(move |request:StockRecoveryActionRequest| {
        if pending.get_untracked() {return;}
        pending.set(true);notice.set(None);
        let source=action_scope.capture();
        let client=source.client();
        spawn_local(async move {
            let Some(result)=action_scope.snapshot(&source,client.recheck_stock_recovery(&request)).await else {return;};
            match result {
                Ok(snapshot)=>apply_snapshot(market,snapshot),
                Err(e)=>{notice.try_set(Some(e.problem.message));},
            }
            pending.try_set(false);
        });
    });
    PreflightData {
        selected_plan,
        selected_funding_plan,
        restock,
        conversion_cost_ids,
        conversion,
        stablecoin,
        funding_prepare_transfer,
        funding_submit,
        funding_recheck,
        funding_build,
        funding_cancel,
        deposit_address,
        recovery,
        cancel_recovery,
        recheck_recovery,
        wallet,
        pending,
        read,
        build,
        build_journal,
        build_recheck,
        cancel,
        recheck,
        settle,
        topup,
        recheck_topup,
        execute,
    }
}

#[cfg(test)]
impl PreflightData {
    pub(in crate::panels::modules::stocks) fn fixture() -> Self {
        Self {
            selected_plan: RwSignal::new(None),
            selected_funding_plan: RwSignal::new(None),
            restock: Callback::new(|_| {}),
            conversion_cost_ids: RwSignal::new(vec![]),
            stablecoin: stablecoin::StablecoinData::fixture(),
            conversion: exchange_conversion::ConversionData::fixture(),
            funding_prepare_transfer: Callback::new(|_| {}),
            funding_submit: Callback::new(|_| {}),
            funding_recheck: Callback::new(|_| {}),
            funding_build: Callback::new(|_| {}),
            funding_cancel: Callback::new(|_| {}),
            deposit_address: Callback::new(|_| {}),
            wallet: RwSignal::new(String::new()),
            pending: RwSignal::new(false),
            read: Callback::new(|_| {}),
            build: Callback::new(|_| {}),
            build_journal: OperationJournal::fixture("stocks-plan"),
            build_recheck: Callback::new(|_| {}),
            cancel: Callback::new(|_| {}),
            recheck: Callback::new(|_| {}),
            settle: Callback::new(|_| {}),
            topup: Callback::new(|_| {}),
            recheck_topup: Callback::new(|_| {}),
            execute: Callback::new(|_| {}),
            recovery: Callback::new(|_| {}),
            cancel_recovery: Callback::new(|_| {}),
            recheck_recovery: Callback::new(|_| {}),
        }
    }
}
