use super::*;
mod build;

#[derive(Clone, Copy)]
pub(in crate::panels::modules::stocks) struct PeerPlanData {
    pub journal: OperationJournal,
    pub build_recheck: Callback<()>,
    pub pending: RwSignal<bool>,
    pub selected_plan: RwSignal<Option<String>>,
    pub problem: RwSignal<Option<String>>,
    pub build: Callback<(String, StockChainDirection)>,
    pub cancel: Callback<StockPlanRevisionRequest>,
    pub refresh: Callback<()>,
    pub execute: Callback<StockPeerExecutionRequest>,
    pub recheck: Callback<StockPlanRevisionRequest>,
    pub settle: Callback<StockPlanRevisionRequest>,
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
    pub(super) fn defaults(journal: OperationJournal) -> Self {
        Self {
            journal,
            build_recheck: Callback::new(|_| {}),
            pending: RwSignal::new(false),
            selected_plan: RwSignal::new(None),
            problem: RwSignal::new(None),
            build: Callback::new(|_| {}),
            cancel: Callback::new(|_| {}),
            refresh: Callback::new(|_| {}),
            execute: Callback::new(|_| {}),
            recheck: Callback::new(|_| {}),
            settle: Callback::new(|_| {}),
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
pub(super) fn use_plans(market: RwSignal<LoadState<StockMarketSnapshot>>, budget: RwSignal<String>, keyed: RwSignal<bool>, journal: OperationJournal) -> PeerPlanData {
    let mut data = PeerPlanData::defaults(journal);
    let scope = StockSource::new(market, move || {
        data.pending.set(false);
        data.problem.set(None);
        data.selected_plan.set(None);
    });
    macro_rules! recovery_action {
        ($field:ident,$ty:ty,$method:ident) => {{
            data.$field = Callback::new(move |r: $ty| {
                if data.pending.get_untracked() {
                    return;
                }
                data.pending.set(true);
                data.problem.set(None);
                let source = scope.capture();
                let client = source.client();
                spawn_local(async move {
                    let Some(result) = scope.snapshot(&source, client.$method(&r)).await else { return; };
                    match result {
                        Ok(s) => apply_snapshot(market, s),
                        Err(e) => {
                            data.problem.try_set(Some(e.problem.message));
                            let Some(result) = scope.snapshot(&source, client.stock_peer_plans()).await else { return; };
                            if let Ok(s) = result {
                                apply_snapshot(market, s);
                            }
                        }
                    }
                    data.pending.try_set(false);
                });
            });
        }};
    }
    recovery_action!(settle, StockPlanRevisionRequest, settle_stock_peer_plan);
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
        move |request: StockPeerExecutionRequest| {
            if data.pending.get_untracked() || !request.confirm_live {
                return;
            }
            data.pending.set(true);
            data.problem.set(None);
            let source = scope.capture();
            let client = source.client();
            spawn_local(async move {
                let Some(result) = scope.snapshot(&source, client.execute_stock_peer_plan(&request)).await else { return; };
                match result {
                    Ok(s) => apply_snapshot(market, s),
                    Err(e) => {
                        data.problem.try_set(Some(e.problem.message));
                        // A missing response must not leave a stale "submit" button.
                        let Some(result) = scope.snapshot(&source, client.stock_peer_plans()).await else { return; };
                        if let Ok(s) = result {
                            apply_snapshot(market, s);
                        }
                    }
                }
                data.pending.try_set(false);
            });
        }
    });
    data.recheck = Callback::new({
        move |request: StockPlanRevisionRequest| {
            if data.pending.get_untracked() {
                return;
            }
            data.pending.set(true);
            data.problem.set(None);
            let source = scope.capture();
            let client = source.client();
            spawn_local(async move {
                let Some(result) = scope.snapshot(&source, client.recheck_stock_peer_plan(&request)).await else { return; };
                match result {
                    Ok(s) => apply_snapshot(market, s),
                    Err(e) => {
                        data.problem.try_set(Some(e.problem.message));
                    }
                }
                data.pending.try_set(false);
            });
        }
    });
    (data.build, data.build_recheck) = build::callbacks(data, market, budget, keyed);
    data.cancel = Callback::new({
        move |request: StockPlanRevisionRequest| {
            if data.pending.get_untracked() {
                return;
            }
            data.pending.set(true);
            data.problem.set(None);
            let source = scope.capture();
            let client = source.client();
            spawn_local(async move {
                let Some(result) = scope.snapshot(&source, client.cancel_stock_peer_plan(&request)).await else { return; };
                match result {
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
        let source = scope.capture();
        let client = source.client();
        spawn_local(async move {
            let Some(result) = scope.snapshot(&source, client.stock_peer_plans()).await else { return; };
            match result {
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
