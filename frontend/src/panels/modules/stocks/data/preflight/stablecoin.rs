use super::*;

#[derive(Clone, Copy)]
pub(in crate::panels::modules::stocks) struct StablecoinData {
    pub input: RwSignal<String>,
    pub target: RwSignal<String>,
    pub preview: RwSignal<Option<StockStablecoinPreview>>,
    pub problem: RwSignal<Option<String>>,
    pub restoring: RwSignal<bool>,
    pub restored: RwSignal<bool>,
    pub restore_problem: RwSignal<Option<String>>,
    pub refresh: Callback<()>,
    pub read: Callback<(String, bool)>,
    pub save: Callback<StockStablecoinPreview>,
    pub cancel: Callback<StockPlanRevisionRequest>,
    pub submit: Callback<StockStablecoinSubmitRequest>,
    pub recheck: Callback<StockPlanCancelRequest>,
    pub prepare_topup: Callback<StockPlanRevisionRequest>,
    pub submit_topup: Callback<StockStablecoinTopupSubmitRequest>,
    pub cancel_topup: Callback<StockTopupRecheckRequest>,
    pub recheck_topup: Callback<StockTopupRecheckRequest>,
    pub plans: Memo<Vec<StockStablecoinPlan>>,
    pub store_problem: Memo<Option<String>>,
}

pub(super) fn use_stablecoin(
    market: RwSignal<LoadState<StockMarketSnapshot>>,
    wallet: RwSignal<String>,
    pending: RwSignal<bool>,
) -> StablecoinData {
    let input = RwSignal::new(String::new());
    let target = RwSignal::new(String::new());
    let preview = RwSignal::new(None);
    let problem = RwSignal::new(None);
    let restoring = RwSignal::new(false);
    let restored = RwSignal::new(false);
    let restore_problem = RwSignal::new(None);
    let attempt = StoredValue::new(None::<StockStablecoinPlanRequest>);
    let scope = StockSource::new(market, move || {
        pending.set(false);
        preview.set(None);
        problem.set(None);
        restoring.set(false);
        restored.set(false);
        restore_problem.set(None);
        input.set(String::new());
        target.set(String::new());
        attempt.set_value(None);
    });
    let plans = Memo::new(move |_| {
        market.with(|m| {
            m.value()
                .map(|s| s.stablecoin_plans.clone())
                .unwrap_or_default()
        })
    });
    let store_problem =
        Memo::new(move |_| market.with(|m| m.value().and_then(|s| s.stablecoin_problem.clone())));
    let refresh = Callback::new(move |()| {
        if restoring.get_untracked() { return; }
        restoring.set(true);
        restore_problem.set(None);
        let source = scope.capture();
        let client = source.client();
        spawn_local(async move {
        let Some(result) = scope.snapshot(&source, client.stock_stablecoin_plans()).await else { return; };
        match result {
            Ok(snapshot) => {
                apply_snapshot(market, snapshot);
                restored.try_set(true);
            },
            Err(e) => {
                restore_problem.try_set(Some(format!("兑换记录读取失败：{}", e.problem.message)));
            }
        }
        restoring.try_set(false);
        });
    });
    Effect::new(move |_| { scope.track(); refresh.run(()); });
    let read = Callback::new(move |(asset, keyed): (String, bool)| {
        if pending.get_untracked() {
            return;
        }
        let request = StockStablecoinRequest {
            asset,
            wallet_address: wallet.get_untracked().trim().into(),
            input_usdt: input.get_untracked().trim().into(),
            target_usdc: target.get_untracked().trim().into(),
            keyed,
        };
        if let Err(e) = request.amounts_raw() {
            problem.set(Some(e));
            return;
        }
        pending.set(true);
        preview.set(None);
        problem.set(None);
        let source = scope.capture();
        let client = source.client();
        spawn_local(async move {
            let result = client.preview_stock_stablecoin(&request).await;
            if !scope.current(&source) { return; }
            if market
                .try_with(|m| {
                    m.value()
                        .and_then(|s| s.security.as_ref())
                        .is_some_and(|s| s.asset == request.asset)
                })
                .unwrap_or(false)
                && wallet
                    .try_with(|s| s.trim() == request.wallet_address)
                    .unwrap_or(false)
                && input
                    .try_with(|s| s.trim() == request.input_usdt)
                    .unwrap_or(false)
                && target
                    .try_with(|s| s.trim() == request.target_usdc)
                    .unwrap_or(false)
            {
                match result {
                    Ok(p) if p.request == request => {
                        preview.try_set(Some(p));
                    }
                    Ok(_) => { problem.try_set(Some("兑换试算回复与当前参数不一致，未采用该报价".into())); }
                    Err(e) => {
                        problem.try_set(Some(e.problem.message));
                    }
                }
            }
            pending.try_set(false);
        });
    });
    let save = Callback::new(move |p: StockStablecoinPreview| {
        if pending.get_untracked()
            || !p.can_reserve(super::super::super::super::timestamp::now_ms())
            || p.request.wallet_address != wallet.get_untracked().trim()
            || p.request.input_usdt != input.get_untracked().trim()
            || p.request.target_usdc != target.get_untracked().trim()
        {
            return;
        }
        let Some(cost) = p.cost.as_ref() else {
            return;
        };
        let previous = attempt.get_value().filter(|r| {
            r.conversion == p.request
                && r.preview_at_ms == p.checked_at_ms
                && r.transaction_fingerprint == cost.transaction_fingerprint
        });
        let request = previous.unwrap_or_else(|| StockStablecoinPlanRequest {
            request_id: crate::api::rest::MutationRequestContext::new_idempotent_attempt(
                "stock-stablecoin",
            )
            .request_id()
            .into(),
            conversion: p.request,
            preview_at_ms: p.checked_at_ms,
            transaction_fingerprint: cost.transaction_fingerprint.clone(),
        });
        attempt.set_value(Some(request.clone()));
        pending.set(true);
        problem.set(None);
        let source = scope.capture();
        let client = source.client();
        spawn_local(async move {
            let Some(result) = scope.snapshot(&source, client.build_stock_stablecoin_plan(&request)).await else { return; };
            match result {
                Ok(s) => {
                    attempt.try_set_value(None);
                    apply_snapshot(market, s);
                }
                Err(e) => {
                    problem.try_set(Some(e.problem.message));
                }
            }
            pending.try_set(false);
        });
    });
    let cancel = Callback::new(move |request: StockPlanRevisionRequest| {
        if pending.get_untracked() {
            return;
        }
        pending.set(true);
        problem.set(None);
        let source = scope.capture();
        let client = source.client();
        spawn_local(async move {
            let Some(result) = scope.snapshot(&source, client.cancel_stock_stablecoin_plan(&request)).await else { return; };
            match result {
                Ok(s) => apply_snapshot(market, s),
                Err(e) => {
                    problem.try_set(Some(e.problem.message));
                }
            }
            pending.try_set(false);
        });
    });
    let submit = Callback::new(move |request: StockStablecoinSubmitRequest| {
        if pending.get_untracked() || !request.confirm_live {
            return;
        }
        pending.set(true);
        problem.set(None);
        let source = scope.capture();
        let client = source.client();
        spawn_local(async move {
            let Some(result) = scope.snapshot(&source, client.submit_stock_stablecoin(&request)).await else { return; };
            match result {
                Ok(s) => apply_snapshot(market, s),
                Err(e) => {
                    problem.try_set(Some(format!(
                        "{}；请查看原计划，不要重新建单",
                        e.problem.message
                    )));
                    // A timed-out response may have a persisted intent; refresh before exposing actions.
                    let Some(result) = scope.snapshot(&source, client.stock_stablecoin_plans()).await else { return; };
                    if let Ok(s) = result {
                        apply_snapshot(market, s);
                    }
                }
            }
            pending.try_set(false);
        });
    });
    let recheck = Callback::new(move |request: StockPlanCancelRequest| {
        if pending.get_untracked() {
            return;
        }
        pending.set(true);
        problem.set(None);
        let source = scope.capture();
        let client = source.client();
        spawn_local(async move {
            let Some(result) = scope.snapshot(&source, client.recheck_stock_stablecoin(&request)).await else { return; };
            match result {
                Ok(s) => apply_snapshot(market, s),
                Err(e) => {
                    problem.try_set(Some(e.problem.message));
                }
            }
            pending.try_set(false);
        });
    });
    let prepare_topup = Callback::new(move |request: StockPlanRevisionRequest| {
        run_topup(pending, problem, market, scope, move |client| async move {
            client.prepare_stock_stablecoin_topup(&request).await
        });
    });
    let submit_topup = Callback::new(move |request: StockStablecoinTopupSubmitRequest| {
        if !request.confirm_live {
            return;
        }
        run_topup(pending, problem, market, scope, move |client| async move {
            client.submit_stock_stablecoin_topup(&request).await
        });
    });
    let cancel_topup = Callback::new(move |request: StockTopupRecheckRequest| {
        run_topup(pending, problem, market, scope, move |client| async move {
            client.cancel_stock_stablecoin_topup(&request).await
        });
    });
    let recheck_topup = Callback::new(move |request: StockTopupRecheckRequest| {
        run_topup(pending, problem, market, scope, move |client| async move {
            client.recheck_stock_stablecoin_topup(&request).await
        });
    });
    StablecoinData {
        restoring,
        restored,
        restore_problem,
        refresh,
        input,
        target,
        preview,
        problem,
        read,
        save,
        cancel,
        submit,
        recheck,
        prepare_topup,
        submit_topup,
        cancel_topup,
        recheck_topup,
        plans,
        store_problem,
    }
}

fn run_topup<F: std::future::Future<Output = Result<StockMarketSnapshot, crate::api::rest::ApiError>> + 'static>(
    pending: RwSignal<bool>,
    problem: RwSignal<Option<String>>,
    market: RwSignal<LoadState<StockMarketSnapshot>>,
    scope: StockSource,
    work: impl FnOnce(crate::api::rest::ApiClient) -> F + 'static,
) {
    if pending.get_untracked() {
        return;
    }
    pending.set(true);
    problem.set(None);
    let source = scope.capture();
    let client = source.client();
    spawn_local(async move {
        let Some(result) = scope.snapshot(&source, work(client.clone())).await else { return; };
        match result {
            Ok(s) => apply_snapshot(market, s),
            Err(e) => {
                problem.try_set(Some(e.problem.message));
                let Some(result) = scope.snapshot(&source, client.stock_stablecoin_plans()).await else { return; };
                if let Ok(s) = result {
                    apply_snapshot(market, s);
                }
            }
        }
        pending.try_set(false);
    });
}

#[cfg(test)]
impl StablecoinData {
    pub(super) fn fixture() -> Self {
        Self {
            restoring: RwSignal::new(false),
            restored: RwSignal::new(false),
            restore_problem: RwSignal::new(None),
            refresh: Callback::new(|_| {}),
            input: RwSignal::new(String::new()),
            target: RwSignal::new(String::new()),
            preview: RwSignal::new(None),
            problem: RwSignal::new(None),
            read: Callback::new(|_| {}),
            save: Callback::new(|_| {}),
            cancel: Callback::new(|_| {}),
            submit: Callback::new(|_| {}),
            recheck: Callback::new(|_| {}),
            prepare_topup: Callback::new(|_| {}),
            submit_topup: Callback::new(|_| {}),
            cancel_topup: Callback::new(|_| {}),
            recheck_topup: Callback::new(|_| {}),
            plans: Memo::new(|_| Vec::new()),
            store_problem: Memo::new(|_| None),
        }
    }
}
