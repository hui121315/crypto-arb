use super::*;

pub(super) fn callbacks(
    client: crate::api::rest::ApiClient,
    market: RwSignal<LoadState<StockMarketSnapshot>>,
    notice: RwSignal<Option<String>>,
    pending: RwSignal<bool>,
) -> (
    Callback<StockFundingSubmitRequest>,
    Callback<StockPlanCancelRequest>,
    Callback<StockPlanRevisionRequest>,
) {
    let read_client = client.clone();
    let prepare_client=client.clone();
    let submit = Callback::new(move |request: StockFundingSubmitRequest| {
        if pending.get_untracked() || !request.confirm_live {
            return;
        }
        pending.set(true);
        notice.set(None);
        let client = client.clone();
        spawn_local(async move {
            match client.submit_stock_funding(&request).await {
                Ok(s) => {
                    apply_snapshot(market, s);
                    notice.try_set(Some(
                        "原补库状态已更新；链上确认与交易所入账分别核验".into(),
                    ));
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
        let client = read_client.clone();
        spawn_local(async move {
            match client.recheck_stock_funding(&request).await {
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
        pending.set(true);notice.set(None);let client=prepare_client.clone();
        spawn_local(async move{
            match client.prepare_stock_funding_transfer(&request).await{
                Ok(s)=>{apply_snapshot(market,s);notice.try_set(Some("原转账与网络费已核算，尚未签名或转账".into()));},
                Err(e)=>{notice.try_set(Some(e.problem.message));},
            }
            pending.try_set(false);
        });
    });
    (submit, recheck, prepare)
}
