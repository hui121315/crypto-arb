use super::*;

#[derive(Clone)]
pub(in crate::panels::modules::stocks) enum ConversionAction {
    Build,
    Cancel(StockPlanRevisionRequest),
    Submit(StockStablecoinSubmitRequest),
    Recheck(StockPlanCancelRequest),
}
#[derive(Clone, Copy)]
pub(in crate::panels::modules::stocks) struct ConversionData {
    pub input: RwSignal<String>,
    pub minimum: RwSignal<String>,
    pub problem: RwSignal<Option<String>>,
    pub sizing: RwSignal<Option<StockExchangeConversionSizing>>,
    pub size: Callback<()>,
    pub run: Callback<ConversionAction>,
}
pub(super) fn use_conversion(
    market: RwSignal<LoadState<StockMarketSnapshot>>,
    pending: RwSignal<bool>,
) -> ConversionData {
    let input = RwSignal::new(String::new());
    let minimum = RwSignal::new(String::new());
    let problem = RwSignal::new(None);
    let sizing = RwSignal::new(None);
    let previous = StoredValue::new(None::<StockExchangeConversionRequest>);
    let scope = StockSource::new(market, move || {
        pending.set(false);
        previous.set_value(None);
        sizing.set(None);
        problem.set(None);
        input.set(String::new());
        minimum.set(String::new());
    });
    let size = Callback::new(move |()| {
        if pending.get_untracked() {
            return;
        }
        let request = StockExchangeConversionSizingRequest {
            minimum_usdc: minimum.get_untracked().trim().into(),
        };
        if let Err(e) = request.minimum() {
            problem.set(Some(e));
            return;
        }
        let selected = market.with_untracked(|m| {
            m.value()
                .and_then(|s| s.security.as_ref())
                .map(|s| s.asset.clone())
        });
        let original_input = input.get_untracked();
        pending.set(true);
        problem.set(None);
        sizing.set(None);
        let source = scope.capture();
        let client = source.client();
        spawn_local(async move {
            let result = client.size_stock_exchange_conversion(&request).await;
            if !scope.current(&source) { return; }
            let unchanged = minimum
                .try_with(|s| s.trim() == request.minimum_usdc)
                .unwrap_or(false)
                && input.try_with(|s| s == &original_input).unwrap_or(false)
                && market
                    .try_with(|m| {
                        m.value()
                            .and_then(|s| s.security.as_ref())
                            .map(|s| s.asset.clone())
                            == selected
                    })
                    .unwrap_or(false);
            if unchanged {
                match result {
                    Ok(result) if result.request == request => {
                        input.try_set(result.input_usdt.clone());
                        sizing.try_set(Some(result));
                        previous.try_set_value(None);
                    }
                    Ok(_) => { problem.try_set(Some("兑换投入试算回复与当前参数不一致，未修改投入金额".into())); }
                    Err(e) => {
                        problem.try_set(Some(e.problem.message));
                    }
                }
            }
            pending.try_set(false);
        });
    });
    let run = Callback::new(move |action: ConversionAction| {
        if pending.get_untracked() {
            return;
        }
        let mut build = None;
        if matches!(action, ConversionAction::Build) {
            let input = input.get_untracked().trim().to_owned();
            let minimum = minimum.get_untracked().trim().to_owned();
            let now = super::super::super::super::timestamp::now_ms();
            if sizing.get_untracked().is_some_and(|s| {
                s.input_usdt == input
                    && s.request.minimum_usdc == minimum
                    && (now < s.checked_at_ms || now >= s.valid_until_ms)
            }) {
                problem.set(Some("试算已过期，请重新计算投入".into()));
                return;
            }
            let r = previous
                .get_value()
                .filter(|r| r.input_usdt == input && r.minimum_usdc == minimum)
                .unwrap_or_else(|| StockExchangeConversionRequest {
                    request_id: crate::api::rest::MutationRequestContext::new_idempotent_attempt(
                        "stock-cex-convert",
                    )
                    .request_id()
                    .into(),
                    input_usdt: input,
                    minimum_usdc: minimum,
                });
            if let Err(e) = r.amounts() {
                problem.set(Some(e));
                return;
            }
            previous.set_value(Some(r.clone()));
            build = Some(r);
        }
        pending.set(true);
        problem.set(None);
        let source = scope.capture();
        let client = source.client();
        spawn_local(async move {
            let Some(result) = scope.snapshot(&source, async {
                match action {
                ConversionAction::Build => {
                    client
                        .build_stock_exchange_conversion(&build.expect("build request validated"))
                        .await
                }
                ConversionAction::Cancel(r) => client.cancel_stock_exchange_conversion(&r).await,
                ConversionAction::Submit(r) => client.submit_stock_exchange_conversion(&r).await,
                ConversionAction::Recheck(r) => client.recheck_stock_exchange_conversion(&r).await,
                }
            }).await else { return; };
            match result {
                Ok(s) => {
                    previous.try_set_value(None);
                    apply_snapshot(market, s);
                }
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
    });
    ConversionData {
        input,
        minimum,
        problem,
        sizing,
        size,
        run,
    }
}
#[cfg(test)]
impl ConversionData {
    pub(super) fn fixture() -> Self {
        Self {
            input: RwSignal::new(String::new()),
            minimum: RwSignal::new(String::new()),
            problem: RwSignal::new(None),
            sizing: RwSignal::new(None),
            size: Callback::new(|_| {}),
            run: Callback::new(|_| {}),
        }
    }
}
