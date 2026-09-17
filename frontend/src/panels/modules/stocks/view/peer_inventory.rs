use super::*;

pub(super) fn panel(data: StockData, p: &StockPeerPlan) -> impl IntoView {
    let gap = p.peer_inventory_gap().ok();
    let present = gap.is_some() || !p.inventory_orders.is_empty();
    let build = p.clone();
    let enabled = p.clone();
    let limit = RwSignal::new(String::new());
    present.then(|| view! {<section class="stock-peer-receipts stock-peer-inventory" aria-label="交易所库存恢复">
        <h4>"交易所库存恢复"</h4>
        {gap.map(|n| {
            let buying = n.is_sign_negative();
            let label = format!("{} {}",if buying {"最多支出"}else{"最低收到"},p.terms.draft.quote_asset);
            let input_label = label.clone();
            view! {<p class="stock-rfq-note">{format!("Kraken 待{} {} 股 · 链上库存另行恢复",if buying {"补买"}else{"卖回"},n.abs().normalize())}</p>
                <form class="stock-peer-recovery-build" on:submit=move |ev|{
                    ev.prevent_default();
                    if stock_peer_recovery_limit(&limit.get_untracked()).is_some() && !data.peers.plans.pending.get_untracked(){
                        data.peers.plans.inventory_build.run(StockPeerInventoryRequest{plan_id:build.plan_id.clone(),revision:build.revision,quote_limit:limit.get_untracked()});
                    }
                }>
                    <label>{label}<input type="text" inputmode="decimal" autocomplete="off" placeholder="0.00" aria-label=input_label prop:value=move ||limit.get() on:input=move |ev|limit.set(event_target_value(&ev))/></label>
                    <button type="submit" class="row-action" disabled=move ||{data.peers.plans.pending.get() || stock_peer_recovery_limit(&limit.get()).is_none() || !enabled.peer_inventory_available(data.clock.get())}>"获取库存恢复报价"</button>
                </form>
            }
        })}
        <p class="stock-rfq-note">"分步恢复存在单边价格风险，额外手续费计入实际收支；不是再次套利或跨发行方互充。"</p>
        {p.inventory_orders.iter().cloned().enumerate().map(|(i,r)|record(data,p,i,r)).collect_view()}
    </section>})
}

fn record(
    data: StockData,
    p: &StockPeerPlan,
    index: usize,
    r: StockPeerInventory,
) -> impl IntoView {
    let action = StockRecoveryActionRequest {
        plan_id: p.plan_id.clone(),
        revision: p.revision,
        index,
    };
    let cancel = action.clone();
    let submit = StockPeerRecoverySubmitRequest {
        plan_id: p.plan_id.clone(),
        revision: p.revision,
        index,
        confirm_live: true,
    };
    let buying = r.buying();
    let submitted = r.order.is_some();
    let cancelled = r.cancelled_at_ms.is_some();
    let complete = r.order.as_ref().is_some_and(|o| o.receipt_complete());
    let actual = r.observed_changes().ok();
    let status = if cancelled {
        "已取消 · 未提交"
    } else if r.actual_changes().is_ok() {
        if r.order
            .as_ref()
            .is_some_and(|o| o.phase == StockCexOrderPhase::Filled)
        {
            "库存订单成交与费用已核实"
        } else {
            "订单已终止 · 库存未变"
        }
    } else if submitted {
        "原订单或费用待核对 · 不重发"
    } else {
        "待确认 · 未提交"
    };
    let problem = r
        .order
        .as_ref()
        .and_then(|o| o.problem.clone())
        .or(r.history.problem.clone())
        .or_else(|| complete.then(|| r.actual_changes().err()).flatten());
    let end = r.valid_until_ms;
    let next_check = r.history.next_check_at_ms;
    let current = index + 1 == p.inventory_orders.len();
    let confirm = RwSignal::new(false);
    let quote = r.draft.quote_asset.clone();
    let fees = r.order.as_ref().map(|o| {
        o.fills
            .iter()
            .map(|f| {
                let fees = f
                    .fees
                    .as_ref()
                    .map(|v| {
                        if v.is_empty() {
                            "明确零费用".into()
                        } else {
                            v.iter()
                                .map(|v| format!("{} {}", v.quantity, v.asset))
                                .collect::<Vec<_>>()
                                .join(" + ")
                        }
                    })
                    .unwrap_or_else(|| "费用未知".into());
                format!("{}：{fees}", f.execution_id)
            })
            .collect::<Vec<_>>()
            .join("；")
    });
    view! {<section class="stock-peer-recovery-record" aria-label=format!("库存恢复 {}",index+1)>
        <header><strong>{format!("库存恢复 {} · {}",index+1,if buying{"补买"}else{"卖回"})}</strong><span>{status}</span></header>
        <dl class="stock-peer-plan-summary">
            <div><dt>"股票数量 / 股"</dt><dd>{r.draft.quantity}</dd></div>
            <div><dt>{format!("{} / {quote}",if buying{"最多支出"}else{"最低收到"})}</dt><dd>{r.request.quote_limit}</dd></div>
            <div><dt>{format!("净收支预算 / {quote}")}</dt><dd>{r.quote_change}</dd></div>
            <div><dt>{format!("手续费预算 / {quote}")}</dt><dd>{r.fee_quote}</dd></div>
        </dl>
        {actual.map(|c|view!{<dl class="stock-peer-plan-summary"><div><dt>"实际股票变动 / 股"</dt><dd>{c.equity_shares_change}</dd></div><div><dt>{format!("实际收支 / {}",c.quote_asset)}</dt><dd>{c.quote_change}</dd></div></dl>})}
        {problem.map(|e|view!{<p class="stock-rfq-note">{e}</p>})}
        {(!submitted && !cancelled && current).then(||view!{<div class="stock-peer-execution-actions">
            <label><input type="checkbox" prop:checked=move ||confirm.get() disabled=move ||{data.clock.get()>=end || data.peers.plans.pending.get()} on:change=move |ev|confirm.set(event_target_checked(&ev))/>"确认本次真实库存恢复"</label>
            <button type="button" class="row-action stock-peer-inventory-submit" disabled=move ||{!confirm.get() || data.clock.get()>=end || data.peers.plans.pending.get()} on:click=move |_|{if confirm.get_untracked(){confirm.set(false);data.peers.plans.inventory_submit.run(submit.clone());}}>"提交库存恢复"</button>
            <button type="button" class="row-action" disabled=move ||data.peers.plans.pending.get() on:click=move |_|data.peers.plans.inventory_cancel.run(cancel.clone())>"取消报价"</button>
            <span>{move ||if data.clock.get()>=end{"报价已过期 · 不能提交"}else{"报价有效"}}</span>
        </div>})}
        {(submitted && !complete).then(||view!{<button type="button" class="row-action" disabled=move ||{data.peers.plans.pending.get() || data.clock.get()<next_check} on:click=move |_|data.peers.plans.inventory_recheck.run(action.clone())>"核对原库存订单"</button>})}
        <details><summary>"库存恢复原始依据"</summary><dl class="stock-plan-evidence">
            <div><dt>"交易所股票差额 / 股"</dt><dd>{r.stock_gap}</dd></div>
            <div><dt>"原生市场 / 限价"</dt><dd>{format!("{} / {}",r.draft.request.selection.native_symbol,r.draft.limit_price)}</dd></div>
            <div><dt>"实际原币手续费"</dt><dd>{fees.unwrap_or_else(||"尚无成交".into())}</dd></div>
            <div><dt>"原订单标识"</dt><dd>{r.order.map(|o|o.client_order_id).unwrap_or_else(||"尚未提交".into())}</dd></div>
        </dl></details>
    </section>}
}
