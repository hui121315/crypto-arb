use super::*;
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
    pub read: Callback<String>,
    pub deposit_address: Callback<String>,
    pub funding_build: Callback<StockFundingPlanRequest>,
    pub funding_cancel: Callback<StockPlanRevisionRequest>,
    pub funding_submit: Callback<StockFundingSubmitRequest>,
    pub funding_recheck: Callback<StockPlanCancelRequest>,
    pub funding_prepare_transfer: Callback<StockPlanRevisionRequest>,
    pub build: Callback<(String, StockChainDirection)>,
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
    client: crate::api::rest::ApiClient,
    market: RwSignal<LoadState<StockMarketSnapshot>>,
    notice: RwSignal<Option<String>>,
    budget: RwSignal<String>,
    keyed: RwSignal<bool>,
) -> PreflightData {
    let wallet = RwSignal::new(String::new());
    let pending = RwSignal::new(false);
    let stablecoin = stablecoin::use_stablecoin(client.clone(), market, wallet, pending);
    let conversion = exchange_conversion::use_conversion(client.clone(), market, pending);
    let (funding_build, funding_cancel) = funding::callbacks(client.clone(), market, notice, pending);
    let (funding_submit, funding_recheck, funding_prepare_transfer) = withdrawal::callbacks(client.clone(), market, notice, pending);
    let plan_client = client.clone();
    let address_client = client.clone();
    let cancel_client = client.clone();
    let recheck_client = client.clone();
    let settle_client = client.clone();
    let topup_client = client.clone();
    let topup_recheck_client = client.clone();
    let execute_client = client.clone();
    let recovery_client = client.clone();
    let cancel_recovery_client = client.clone();
    let recheck_recovery_client = client.clone();
    let read = Callback::new(move |asset: String| {
        if pending.get_untracked() {
            return;
        }
        pending.set(true);
        notice.set(None);
        let client = client.clone();
        let request = StockPreflightRequest {
            asset: asset.clone(),
            wallet_address: Some(wallet.get_untracked()),
        };
        spawn_local(async move {
            let result = client.preflight_stock(&request).await;
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
                    Err(e) => {
                        notice.try_set(Some(e.problem.message));
                    }
                }
            }
            pending.try_set(false);
        });
    });
    let deposit_address = Callback::new(move |asset:String| {
        if pending.get_untracked() {return;}
        pending.set(true);notice.set(None);
        let client=address_client.clone();
        spawn_local(async move {
            let result=client.stock_deposit_address(&StockDepositAddressRequest{asset:asset.clone()}).await;
            if market.try_with(|m|m.value().and_then(|s|s.security.as_ref()).is_some_and(|s|s.asset==asset)).unwrap_or(false) {
                match result {Ok(s)=>apply_snapshot(market,s),Err(e)=>{notice.try_set(Some(e.problem.message));}}
            }
            pending.try_set(false);
        });
    });
    let attempt = StoredValue::new(None::<StockPlanBuildRequest>);
    let build = Callback::new(move |(asset, direction): (String, StockChainDirection)| {
        if pending.get_untracked() {
            return;
        }
        let problem = market.with_untracked(|m| {
            m.value().map(|s| super::super::readiness::build_block_reason(s, &wallet.get_untracked(),
                &budget.get_untracked(), keyed.get_untracked(), direction, super::super::super::timestamp::now_ms()))
                .unwrap_or(Some("股票行情尚未就绪"))
        });
        if let Some(problem) = problem { notice.set(Some(problem.into())); return; }
        let Some(comparison) = market.with_untracked(|m| {
            m.value()
                .and_then(|s| s.comparison.clone())
        }) else {
            return;
        };
        if comparison.asset != asset { return; }
        let Some(quote) = direction.quote(&comparison) else { return; };
        let input_raw = quote.input_raw.clone();
        let wallet_address = wallet.get_untracked().trim().to_owned();
        let previous = attempt.get_value().filter(|p| {
            p.asset == asset
                && p.direction == direction
                && p.wallet_address == wallet_address
                && p.input_raw == input_raw && p.keyed == comparison.keyed
        });
        let request = previous.unwrap_or_else(|| StockPlanBuildRequest {
            request_id: crate::api::rest::MutationRequestContext::new_idempotent_attempt(
                "stock-plan",
            )
            .request_id()
            .into(),
            asset,
            direction,
            wallet_address,
            input_raw, keyed: comparison.keyed,
        });
        attempt.set_value(Some(request.clone()));
        pending.set(true);
        notice.set(None);
        let client = plan_client.clone();
        spawn_local(async move {
            match client.build_stock_plan(&request).await {
                Ok(snapshot) => {
                    attempt.try_set_value(None);
                    let message = snapshot.plans.iter().find(|p| p.request.request_id == request.request_id)
                        .map(|p| match p.phase_at(super::super::super::timestamp::now_ms()) {
                            StockPlanPhase::Reserved => "计划已保存并预留，尚未下单",
                            StockPlanPhase::Cancelled => "原计划已取消，没有重新预留",
                            StockPlanPhase::Expired => "原计划预留已到期，没有重新预留",
                            StockPlanPhase::SubmissionUnknown => "原计划已提交，继续核对原回执，不重复提交",
                            StockPlanPhase::Settled => "原计划已收尾，没有重新预留",
                        }).unwrap_or("原构建请求已核对，请查看计划记录");
                    apply_snapshot(market, snapshot);
                    notice.try_set(Some(message.into()));
                }
                Err(e) => {
                    notice.try_set(Some(e.problem.message));
                }
            }
            pending.try_set(false);
        });
    });
    let cancel = Callback::new(move |id: String| {
        if pending.get_untracked() {
            return;
        }
        pending.set(true);
        notice.set(None);
        let client = cancel_client.clone();
        spawn_local(async move {
            match client.cancel_stock_plan(&id).await {
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
        let client=recheck_client.clone();
        spawn_local(async move {
            match client.recheck_stock_order(&id).await {
                Ok(snapshot)=>apply_snapshot(market,snapshot),
                Err(e)=>{notice.try_set(Some(e.problem.message));},
            }
            pending.try_set(false);
        });
    });
    let settle = Callback::new(move |request: StockPlanRevisionRequest| {
        if pending.get_untracked() { return; }
        pending.set(true); notice.set(None);
        let client = settle_client.clone();
        spawn_local(async move {
            match client.settle_stock_plan(&request).await {
                Ok(snapshot) => apply_snapshot(market, snapshot),
                Err(e) => { notice.try_set(Some(e.problem.message)); }
            }
            pending.try_set(false);
        });
    });
    let topup = Callback::new(move |request: StockPlanRevisionRequest| {
        if pending.get_untracked() { return; }
        pending.set(true); notice.set(None);
        let client = topup_client.clone();
        spawn_local(async move {
            match client.prepare_stock_topup(&request).await {
                Ok(snapshot) => apply_snapshot(market, snapshot),
                Err(e) => { notice.try_set(Some(e.problem.message)); }
            }
            pending.try_set(false);
        });
    });
    let recheck_topup = Callback::new(move |request: StockTopupRecheckRequest| {
        if pending.get_untracked() { return; }
        pending.set(true); notice.set(None);
        let client = topup_recheck_client.clone();
        spawn_local(async move {
            match client.recheck_stock_topup(&request).await {
                Ok(snapshot) => apply_snapshot(market, snapshot),
                Err(e) => { notice.try_set(Some(e.problem.message)); }
            }
            pending.try_set(false);
        });
    });
    let execute = Callback::new(move |request: StockPlanExecutionRequest| {
        if pending.get_untracked() || !request.confirm_live { return; }
        pending.set(true); notice.set(None);
        let client = execute_client.clone();
        spawn_local(async move {
            match client.execute_stock_plan(&request).await {
                Ok(snapshot) => {
                    apply_snapshot(market, snapshot);
                    notice.try_set(Some("原计划提交状态已更新，成交结果以两腿回执为准".into()));
                }
                Err(e) => { notice.try_set(Some(e.problem.message)); }
            }
            pending.try_set(false);
        });
    });
    let recovery = Callback::new(move |request:StockRecoveryBuildRequest| {
        if pending.get_untracked() {return;}
        pending.set(true);notice.set(None);
        let client=recovery_client.clone();
        spawn_local(async move {
            match client.prepare_stock_recovery(&request).await {
                Ok(snapshot)=>apply_snapshot(market,snapshot),
                Err(e)=>{notice.try_set(Some(e.problem.message));},
            }
            pending.try_set(false);
        });
    });
    let cancel_recovery = Callback::new(move |request:StockRecoveryActionRequest| {
        if pending.get_untracked() {return;}
        pending.set(true);notice.set(None);
        let client=cancel_recovery_client.clone();
        spawn_local(async move {
            match client.cancel_stock_recovery(&request).await {
                Ok(snapshot)=>apply_snapshot(market,snapshot),
                Err(e)=>{notice.try_set(Some(e.problem.message));},
            }
            pending.try_set(false);
        });
    });
    let recheck_recovery = Callback::new(move |request:StockRecoveryActionRequest| {
        if pending.get_untracked() {return;}
        pending.set(true);notice.set(None);
        let client=recheck_recovery_client.clone();
        spawn_local(async move {
            match client.recheck_stock_recovery(&request).await {
                Ok(snapshot)=>apply_snapshot(market,snapshot),
                Err(e)=>{notice.try_set(Some(e.problem.message));},
            }
            pending.try_set(false);
        });
    });
    PreflightData {
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
