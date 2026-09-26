use super::*;

pub(super) fn confirmation(
    data: StockData,
    plan: &StockExecutionPlan,
    action: StockExecutionAction,
    expiry: i64,
) -> impl IntoView {
    let confirmed = RwSignal::new(false);
    let original = plan.clone();
    let market_problem = Memo::new(move |_| {
        data.market.with(|m| match m.value() {
            Some(s) => {
                if !original.terms.conversion_costs.is_empty() {
                    if let Some(problem) = &s.exchange_conversion_problem {return Some(problem.clone());}
                    if let Err(problem) = original.check_conversion_sources(&s.exchange_conversions) {return Some(problem);}
                }
                if action == StockExecutionAction::Pair {
                    original.submission_market_check(s, data.clock.get()).err()
                } else {None}
            },
            None => Some("等待股票行情连接".into()),
        })
    });
    let request = StockPlanExecutionRequest {
        plan_id: plan.plan_id.clone(),
        revision: plan.revision,
        action,
        confirm_live: true,
    };
    let (title, button, warning) = match action {
        StockExecutionAction::Pair => (
            "执行两腿",
            "提交两腿",
            "两腿不是原子成交，可能单边失败；金额以本计划备款为限。",
        ),
        StockExecutionAction::NativeTopup { .. } => (
            "执行 SOL 补回",
            "提交 SOL 补回",
            "使用上列 USDC 投入补回原生 SOL，费用计入本计划收支。",
        ),
        StockExecutionAction::Recovery { .. } => (
            "执行股票补偿",
            "提交补偿",
            "仅处理上列股票差额；这是新交易，仍可能失败并产生费用。",
        ),
    };
    view! {<details class="stock-execution-confirmation">
        <summary>{title}</summary>
        <p class="stock-rfq-note">{warning}</p>
        <div class="stock-monitor-control">
            <label><input type="checkbox" checked=move ||confirmed.get() prop:checked=move ||confirmed.get()
                disabled={move ||data.preflight.pending.get() || data.clock.get()>=expiry || market_problem.get().is_some()}
                on:change=move |ev|confirmed.set(event_target_checked(&ev))/><span>"确认本次实盘资金操作"</span></label>
            <button type="button" class="row-action" disabled={move ||!confirmed.get() ||data.preflight.pending.get() ||data.clock.get()>=expiry || market_problem.get().is_some()}
                on:click=move |_|{
                    if confirmed.get_untracked() && data.clock.get_untracked()<expiry && !data.preflight.pending.get_untracked() && market_problem.get_untracked().is_none() {
                        confirmed.set(false);
                        data.preflight.execute.run(request.clone());
                    }
                }>{button}</button>
            <span role="status">{move ||if data.clock.get()>=expiry {"报价已过期，请重新构建".into()}
                else if let Some(reason)=market_problem.get() {reason}
                else {format!("原报价剩余 {:.1}s · 提交时复核备款",(expiry-data.clock.get()) as f64/1000.0)}}</span>
        </div>
    </details>}
}
