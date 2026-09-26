use super::*;

pub(super) fn callbacks(
    market: RwSignal<LoadState<StockMarketSnapshot>>,
    notice: Notice,
    pending: RwSignal<bool>,
    selected_plan: RwSignal<Option<String>>,
) -> (
    Callback<StockFundingPlanRequest>,
    Callback<StockPlanRevisionRequest>,
) {
    let attempt = StoredValue::new(None::<StockFundingPlanRequest>);
    let scope = StockSource::new(market, move || {
        pending.set(false);
        attempt.set_value(None);
        notice.set(None);
    });
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
        let source = scope.capture();
        let client = source.client();
        spawn_local(async move {
            let Some(result) = scope.snapshot(&source, client.build_stock_funding_plan(&request)).await else { return; };
            match result {
                Ok(snapshot) => {
                    attempt.try_set_value(None);
                    if let Some(plan)=snapshot.funding_plans.iter().find(|p|p.request.request_id==request.request_id) {
                        selected_plan.try_set(Some(plan.plan_id.clone()));
                    }
                    let failed = snapshot.funding_plans.iter().find(|p|p.request.request_id == request.request_id)
                        .is_some_and(|p|p.phase == StockFundingPlanPhase::TransferFailed);
                    let message = snapshot
                        .funding_plans
                        .iter()
                        .find(|p| p.request.request_id == request.request_id)
                        .map(
                            |p| match p.phase_at(crate::panels::modules::timestamp::now_ms()) {
                                StockFundingPlanPhase::Reserved => "补充余额计划已保存并预留，未转账",
                                StockFundingPlanPhase::Cancelled => {
                                    "原补充余额计划已取消，没有重新预留"
                                }
                                StockFundingPlanPhase::Expired => "原补充余额计划已到期，没有重新预留",
                                StockFundingPlanPhase::Withdrawing | StockFundingPlanPhase::Received => "原补充余额已提交，请查看原提现和到账记录，没有再次提现",
                                StockFundingPlanPhase::Transferring | StockFundingPlanPhase::DepositPending => "原链上补充余额已提交，请核对原交易与 Backpack 入账，没有重新转账",
                                StockFundingPlanPhase::Deposited => "原补充余额已确认入账，没有重新转账",
                                StockFundingPlanPhase::TransferFailed => "原补充余额转账失败，网络费已记录，没有重新转账",
                            },
                        )
                        .unwrap_or("原补充余额请求已核对，请查看补充余额记录");
                    apply_snapshot(market, snapshot);
                    if failed { notice.try_set(Some(message.into())); } else { notice.inform(message); }
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
        let source = scope.capture();
        let client = source.client();
        spawn_local(async move {
            let Some(result) = scope.snapshot(&source, client.cancel_stock_funding_plan(&request)).await else { return; };
            match result {
                Ok(snapshot) => {
                    apply_snapshot(market, snapshot);
                    notice.inform("补充余额预留已取消，未转账");
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
        && a.source_plan == b.source_plan
        && a.funding_asset == b.funding_asset
        && a.direction == b.direction
        && a.target == b.target
        && a.wallet_address == b.wallet_address
        && a.preflight_at_ms == b.preflight_at_ms
}
