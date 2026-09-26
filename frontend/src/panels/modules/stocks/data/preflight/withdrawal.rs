use super::*;

pub(super) fn callbacks(
    market: RwSignal<LoadState<StockMarketSnapshot>>,
    notice: Notice,
    pending: RwSignal<bool>,
) -> (
    Callback<StockFundingSubmitRequest>,
    Callback<StockPlanCancelRequest>,
    Callback<StockPlanRevisionRequest>,
) {
    let scope = StockSource::new(market, move || {
        pending.set(false);
        notice.set(None);
    });
    let submit = Callback::new(move |request: StockFundingSubmitRequest| {
        if pending.get_untracked() || !request.confirm_live {
            return;
        }
        pending.set(true);
        notice.set(None);
        let source = scope.capture();
        let client = source.client();
        spawn_local(async move {
            let Some(result) = scope.snapshot(&source, client.submit_stock_funding(&request)).await else { return; };
            match result {
                Ok(s) => {
                    apply_snapshot(market, s);
                    notice.inform("原补充余额状态已更新；链上确认与交易所入账分别核对");
                }
                Err(e) => {
                    notice.try_set(Some(e.problem.message));
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
        notice.set(None);
        let source = scope.capture();
        let client = source.client();
        spawn_local(async move {
            let Some(result) = scope.snapshot(&source, client.recheck_stock_funding(&request)).await else { return; };
            match result {
                Ok(s) => apply_snapshot(market, s),
                Err(e) => {
                    notice.try_set(Some(e.problem.message));
                }
            }
            pending.try_set(false);
        });
    });
    let prepare=Callback::new(move|request:StockPlanRevisionRequest|{
        if pending.get_untracked(){return;}
        pending.set(true);notice.set(None);
        let source=scope.capture();let client=source.client();
        spawn_local(async move{
            let Some(result)=scope.snapshot(&source,client.prepare_stock_funding_transfer(&request)).await else{return;};
            match result {
                Ok(s)=>{apply_snapshot(market,s);notice.inform("原转账与网络费已核算，尚未签名或转账");},
                Err(e)=>{notice.try_set(Some(e.problem.message));},
            }
            pending.try_set(false);
        });
    });
    (submit, recheck, prepare)
}
