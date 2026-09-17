use super::*;

pub(super) fn callbacks(
    client: crate::api::rest::ApiClient,
    market: RwSignal<LoadState<StockMarketSnapshot>>,
    notice: RwSignal<Option<String>>,
    pending: RwSignal<bool>,
) -> (
    Callback<StockFundingPlanRequest>,
    Callback<StockPlanRevisionRequest>,
) {
    let cancel_client = client.clone();
    let attempt = StoredValue::new(None::<StockFundingPlanRequest>);
    let build = Callback::new(move |mut request: StockFundingPlanRequest| {
        if pending.get_untracked() {
            return;
        }
        request.wallet_address = request.wallet_address.trim().into();
        request.request_id.clear();
        let previous = attempt.get_value().filter(|p| same_inputs(p, &request));
        let request = previous.unwrap_or_else(|| {
            request.request_id =
                crate::api::rest::MutationRequestContext::new_idempotent_attempt("stock-funding")
                    .request_id()
                    .into();
            request
        });
        attempt.set_value(Some(request.clone()));
        pending.set(true);
        notice.set(None);
        let client = client.clone();
        spawn_local(async move {
            match client.build_stock_funding_plan(&request).await {
                Ok(snapshot) => {
                    attempt.try_set_value(None);
                    let message = snapshot
                        .funding_plans
                        .iter()
                        .find(|p| p.request.request_id == request.request_id)
                        .map(
                            |p| match p.phase_at(crate::panels::modules::timestamp::now_ms()) {
                                StockFundingPlanPhase::Reserved => "补库计划已保存并预留，未转账",
                                StockFundingPlanPhase::Cancelled => {
                                    "原补库计划已取消，没有重新预留"
                                }
                                StockFundingPlanPhase::Expired => "原补库计划已到期，没有重新预留",
                                StockFundingPlanPhase::Withdrawing | StockFundingPlanPhase::Received => "原补库已提交，请查看原提现和到账记录，没有再次提现",
                                StockFundingPlanPhase::Transferring | StockFundingPlanPhase::DepositPending => "原链上补库已提交，请核对原交易与 Backpack 入账，没有重新转账",
                                StockFundingPlanPhase::Deposited => "原补库已确认入账，没有重新转账",
                                StockFundingPlanPhase::TransferFailed => "原补库转账失败，网络费已记录，没有重新转账",
                            },
                        )
                        .unwrap_or("原补库请求已核对，请查看补库记录");
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
    let cancel = Callback::new(move |request: StockPlanRevisionRequest| {
        if pending.get_untracked() {
            return;
        }
        pending.set(true);
        notice.set(None);
        let client = cancel_client.clone();
        spawn_local(async move {
            match client.cancel_stock_funding_plan(&request).await {
                Ok(snapshot) => {
                    apply_snapshot(market, snapshot);
                    notice.try_set(Some("补库预留已取消，未转账".into()));
                }
                Err(e) => {
                    notice.try_set(Some(e.problem.message));
                }
            }
            pending.try_set(false);
        });
    });
    (build, cancel)
}

fn same_inputs(a: &StockFundingPlanRequest, b: &StockFundingPlanRequest) -> bool {
    a.security_asset == b.security_asset
        && a.funding_asset == b.funding_asset
        && a.direction == b.direction
        && a.target == b.target
        && a.wallet_address == b.wallet_address
        && a.preflight_at_ms == b.preflight_at_ms
}
