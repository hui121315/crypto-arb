use super::*;

#[derive(Clone, Copy)]
pub(in crate::panels::modules::stocks) struct PeerPlanData {
    pub pending: RwSignal<bool>,
    pub problem: RwSignal<Option<String>>,
    pub build: Callback<(String, StockChainDirection)>,
    pub cancel: Callback<StockPlanRevisionRequest>,
    pub refresh: Callback<()>,
    pub execute: Callback<StockPeerExecutionRequest>,
    pub recheck: Callback<StockPlanRevisionRequest>,
    pub recovery_build: Callback<StockPeerRecoveryRequest>,
    pub recovery_cancel: Callback<StockRecoveryActionRequest>,
    pub recovery_submit: Callback<StockPeerRecoverySubmitRequest>,
    pub recovery_recheck: Callback<StockRecoveryActionRequest>,
    pub conversion_build: Callback<StockPeerConversionRequest>,
    pub conversion_cancel: Callback<StockRecoveryActionRequest>,
    pub conversion_submit: Callback<StockPeerRecoverySubmitRequest>,
    pub conversion_recheck: Callback<StockRecoveryActionRequest>,
    pub inventory_build: Callback<StockPeerInventoryRequest>,
    pub inventory_cancel: Callback<StockRecoveryActionRequest>,
    pub inventory_submit: Callback<StockPeerRecoverySubmitRequest>,
    pub inventory_recheck: Callback<StockRecoveryActionRequest>,
    pub native_build: Callback<StockPeerNativeTopupRequest>,
    pub native_cancel: Callback<StockRecoveryActionRequest>,
    pub native_submit: Callback<StockPeerRecoverySubmitRequest>,
    pub native_recheck: Callback<StockRecoveryActionRequest>,
}
impl PeerPlanData {
    pub(super) fn defaults() -> Self {
        Self {
            pending: RwSignal::new(false),
            problem: RwSignal::new(None),
            build: Callback::new(|_| {}),
            cancel: Callback::new(|_| {}),
            refresh: Callback::new(|_| {}),
            execute: Callback::new(|_| {}),
            recheck: Callback::new(|_| {}),
            recovery_build: Callback::new(|_| {}),
            recovery_cancel: Callback::new(|_| {}),
            recovery_submit: Callback::new(|_| {}),
            recovery_recheck: Callback::new(|_| {}),
            conversion_build: Callback::new(|_| {}),
            conversion_cancel: Callback::new(|_| {}),
            conversion_submit: Callback::new(|_| {}),
            conversion_recheck: Callback::new(|_| {}),
            inventory_build: Callback::new(|_| {}),
            inventory_cancel: Callback::new(|_| {}),
            inventory_submit: Callback::new(|_| {}),
            inventory_recheck: Callback::new(|_| {}),
            native_build: Callback::new(|_| {}),
            native_cancel: Callback::new(|_| {}),
            native_submit: Callback::new(|_| {}),
            native_recheck: Callback::new(|_| {}),
        }
    }
}
pub(super) fn use_plans(market: RwSignal<LoadState<StockMarketSnapshot>>) -> PeerPlanData {
    let client = use_global().client;
    let mut data = PeerPlanData::defaults();
    let attempt = StoredValue::new(None::<StockPeerPlanRequest>);
    macro_rules! recovery_action {
        ($field:ident,$ty:ty,$method:ident) => {{
            let client = client.clone();
            data.$field = Callback::new(move |r: $ty| {
                if data.pending.get_untracked() {
                    return;
                }
                data.pending.set(true);
                data.problem.set(None);
                let client = client.clone();
                spawn_local(async move {
                    match client.$method(&r).await {
                        Ok(s) => apply_snapshot(market, s),
                        Err(e) => {
                            data.problem.try_set(Some(e.problem.message));
                            if let Ok(s) = client.stock_peer_plans().await {
                                apply_snapshot(market, s);
                            }
                        }
                    }
                    data.pending.try_set(false);
                });
            });
        }};
    }
    recovery_action!(
        recovery_build,
        StockPeerRecoveryRequest,
        prepare_stock_peer_recovery
    );
    recovery_action!(
        recovery_cancel,
        StockRecoveryActionRequest,
        cancel_stock_peer_recovery
    );
    recovery_action!(
        recovery_submit,
        StockPeerRecoverySubmitRequest,
        submit_stock_peer_recovery
    );
    recovery_action!(
        recovery_recheck,
        StockRecoveryActionRequest,
        recheck_stock_peer_recovery
    );
    recovery_action!(
        conversion_build,
        StockPeerConversionRequest,
        prepare_stock_peer_conversion
    );
    recovery_action!(
        conversion_cancel,
        StockRecoveryActionRequest,
        cancel_stock_peer_conversion
    );
    recovery_action!(
        conversion_submit,
        StockPeerRecoverySubmitRequest,
        submit_stock_peer_conversion
    );
    recovery_action!(
        conversion_recheck,
        StockRecoveryActionRequest,
        recheck_stock_peer_conversion
    );
    recovery_action!(native_build,StockPeerNativeTopupRequest,prepare_stock_peer_native_topup);
    recovery_action!(inventory_build,StockPeerInventoryRequest,prepare_stock_peer_inventory);
    recovery_action!(inventory_cancel,StockRecoveryActionRequest,cancel_stock_peer_inventory);
    recovery_action!(inventory_submit,StockPeerRecoverySubmitRequest,submit_stock_peer_inventory);
    recovery_action!(inventory_recheck,StockRecoveryActionRequest,recheck_stock_peer_inventory);
    recovery_action!(native_cancel,StockRecoveryActionRequest,cancel_stock_peer_native_topup);
    recovery_action!(native_submit,StockPeerRecoverySubmitRequest,submit_stock_peer_native_topup);
    recovery_action!(native_recheck,StockRecoveryActionRequest,recheck_stock_peer_native_topup);
    data.execute = Callback::new({
        let client = client.clone();
        move |request: StockPeerExecutionRequest| {
            if data.pending.get_untracked() || !request.confirm_live {
                return;
            }
            data.pending.set(true);
            data.problem.set(None);
            let client = client.clone();
            spawn_local(async move {
                match client.execute_stock_peer_plan(&request).await {
                    Ok(s) => apply_snapshot(market, s),
                    Err(e) => {
                        data.problem.try_set(Some(e.problem.message));
                        // A missing response must not leave a stale "submit" button.
                        if let Ok(s) = client.stock_peer_plans().await {
                            apply_snapshot(market, s);
                        }
                    }
                }
                data.pending.try_set(false);
            });
        }
    });
    data.recheck = Callback::new({
        let client = client.clone();
        move |request: StockPlanRevisionRequest| {
            if data.pending.get_untracked() {
                return;
            }
            data.pending.set(true);
            data.problem.set(None);
            let client = client.clone();
            spawn_local(async move {
                match client.recheck_stock_peer_plan(&request).await {
                    Ok(s) => apply_snapshot(market, s),
                    Err(e) => {
                        data.problem.try_set(Some(e.problem.message));
                    }
                }
                data.pending.try_set(false);
            });
        }
    });
    data.build = Callback::new({
        let client = client.clone();
        move |(wallet, direction): (String, StockChainDirection)| {
            if data.pending.get_untracked() {
                return;
            }
            let Some(mut request) = market.with_untracked(|m| {
                m.value().and_then(|s| {
                    let c = s.comparison.as_ref()?;
                    Some(StockPeerPlanRequest {
                        request_id:
                            crate::api::rest::MutationRequestContext::new_idempotent_attempt(
                                "stock-peer-plan",
                            )
                            .request_id()
                            .into(),
                        asset: c.asset.clone(),
                        selection: s.peer.as_ref()?.selection.clone(),
                        direction,
                        wallet_address: wallet.trim().into(),
                        input_raw: direction.quote(c)?.input_raw.clone(),
                        keyed: c.keyed,
                    })
                })
            }) else {
                return;
            };
            if let Some(old) = attempt.get_value() {
                let mut same = request.clone();
                same.request_id = old.request_id.clone();
                if old == same {
                    request = old;
                }
            }
            if let Err(e) = request.validate() {
                data.problem.set(Some(e));
                return;
            }
            attempt.set_value(Some(request.clone()));
            data.pending.set(true);
            data.problem.set(None);
            let client = client.clone();
            spawn_local(async move {
                match client.build_stock_peer_plan(&request).await {
                    Ok(s) => {
                        attempt.try_set_value(None);
                        apply_snapshot(market, s);
                    }
                    Err(e) => {
                        data.problem.try_set(Some(e.problem.message));
                    }
                }
                data.pending.try_set(false);
            });
        }
    });
    data.cancel = Callback::new({
        let client = client.clone();
        move |request: StockPlanRevisionRequest| {
            if data.pending.get_untracked() {
                return;
            }
            data.pending.set(true);
            data.problem.set(None);
            let client = client.clone();
            spawn_local(async move {
                match client.cancel_stock_peer_plan(&request).await {
                    Ok(s) => apply_snapshot(market, s),
                    Err(e) => {
                        data.problem.try_set(Some(e.problem.message));
                    }
                }
                data.pending.try_set(false);
            });
        }
    });
    data.refresh = Callback::new(move |_| {
        if data.pending.get_untracked() {
            return;
        }
        data.pending.set(true);
        data.problem.set(None);
        let client = client.clone();
        spawn_local(async move {
            match client.stock_peer_plans().await {
                Ok(s) => apply_snapshot(market, s),
                Err(e) => {
                    data.problem.try_set(Some(e.problem.message));
                }
            }
            data.pending.try_set(false);
        });
    });
    data
}
