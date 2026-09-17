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
    pub run: Callback<ConversionAction>,
}
pub(super) fn use_conversion(
    client: crate::api::rest::ApiClient,
    market: RwSignal<LoadState<StockMarketSnapshot>>,
    pending: RwSignal<bool>,
) -> ConversionData {
    let input = RwSignal::new(String::new());
    let minimum = RwSignal::new(String::new());
    let problem = RwSignal::new(None);
    let previous = StoredValue::new(None::<StockExchangeConversionRequest>);
    let run = Callback::new(move |action: ConversionAction| {
        if pending.get_untracked() {
            return;
        }
        let mut build = None;
        if matches!(action, ConversionAction::Build) {
            let input = input.get_untracked().trim().to_owned();
            let minimum = minimum.get_untracked().trim().to_owned();
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
        let client = client.clone();
        spawn_local(async move {
            let result = match action {
                ConversionAction::Build => {
                    client
                        .build_stock_exchange_conversion(&build.expect("build request validated"))
                        .await
                }
                ConversionAction::Cancel(r) => client.cancel_stock_exchange_conversion(&r).await,
                ConversionAction::Submit(r) => client.submit_stock_exchange_conversion(&r).await,
                ConversionAction::Recheck(r) => client.recheck_stock_exchange_conversion(&r).await,
            };
            match result {
                Ok(s) => {
                    previous.try_set_value(None);
                    apply_snapshot(market, s);
                }
                Err(e) => {
                    problem.try_set(Some(e.problem.message));
                    if let Ok(s) = client.stock_stablecoin_plans().await {
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
            run: Callback::new(|_| {}),
        }
    }
}
