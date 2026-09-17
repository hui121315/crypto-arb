use super::*;

pub(super) fn panel(data: StockData, p: &StockPeerPlan) -> impl IntoView {
    let target = p.peer_recovery_target().ok();
    let build = p.clone();
    let limit = RwSignal::new(String::new());
    let build_enabled = p.clone();
    let present = target.is_some() || !p.recoveries.is_empty();
    present.then(||view!{<section class="stock-peer-receipts stock-peer-recovery" aria-label="股票差额补偿">
        <h4>"股票差额补偿"</h4>
        {target.map(|t|{
            let label=if t.direction==StockChainDirection::Buy{"最多支出 USDC"}else{"最低收到 USDC"};
            view!{<form class="stock-peer-recovery-build" on:submit=move |ev|{
                ev.prevent_default();
                if stock_peer_recovery_limit(&limit.get_untracked()).is_some() && !data.peers.plans.pending.get_untracked() {
                    data.peers.plans.recovery_build.run(StockPeerRecoveryRequest{plan_id:build.plan_id.clone(),revision:build.revision,usdc_limit:limit.get_untracked()});
                }
            }>
                <label>{label}<input type="text" inputmode="decimal" autocomplete="off" placeholder="0.00" aria-label=label
                    prop:value=move ||limit.get() on:input=move |ev|limit.set(event_target_value(&ev))/></label>
                <button type="submit" class="row-action" disabled=move ||data.peers.plans.pending.get() || stock_peer_recovery_limit(&limit.get()).is_none() || !build_enabled.peer_recovery_available(data.clock.get())>"获取补偿报价"</button>
                <p class="stock-rfq-note">"限额包含本次 SOL 补回预算，不是 USD/USDC 合计盈亏。"</p>
            </form>}
        })}
        {p.recoveries.iter().cloned().enumerate().map(|(i,r)|record(data,p,i,r)).collect_view()}
    </section>})
}

fn record(data: StockData, p: &StockPeerPlan, index: usize, r: StockPeerRecovery) -> impl IntoView {
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
    let submitted = r.submission.is_some();
    let cancelled = r.cancelled_at_ms.is_some();
    let receipt = r.submission.as_ref().and_then(|s| s.receipt.as_ref());
    let finality = receipt.is_some();
    let phase = if cancelled {
        "已取消 · 未提交"
    } else if let Some(receipt) = receipt {
        if receipt.succeeded && receipt.within_plan {
            "补偿到账已核实"
        } else {
            "补偿失败或到账不符 · 查看实际收支"
        }
    } else if submitted {
        "提交结果待核对 · 不重发"
    } else {
        "待确认 · 未提交"
    };
    let end = r.cost.valid_until_ms;
    let next_check = r.submission.as_ref().map_or(0, |s| s.next_recheck_at_ms);
    let confirm = RwSignal::new(false);
    let current = index + 1 == p.recoveries.len();
    let signed = |n: Option<String>, unit: &str| {
        n.map(|s| format!("{s} {unit}"))
            .unwrap_or_else(|| "待核对".into())
    };
    let net = r.conservative_usdc(r.prepared_at_ms);
    view! {<section class="stock-peer-recovery-record" aria-label=format!("补偿 {}",index+1)>
        <header><strong>{format!("补偿 {} · {}",index+1,if r.target.direction==StockChainDirection::Buy{"补买股票"}else{"卖出多余股票"})}</strong><span>{phase}</span></header>
        <dl class="stock-peer-plan-summary">
            <div><dt>"链上代币数量"</dt><dd>{stock_chain_quantity(&r.target.stock_raw,r.cost.mint.decimals)}</dd></div>
            <div><dt>"本次 USDC 限额"</dt><dd>{r.usdc_limit}</dd></div>
            <div><dt>"报价净收支 · 含 SOL 补回预算"</dt><dd>{signed(net,"USDC")}</dd></div>
            <div><dt>"钱包 SOL 支出上限"</dt><dd>{signed(r.cost.wallet_debit_lamports.as_deref().and_then(|n|stock_chain_quantity(n,9)),"SOL")}</dd></div>
        </dl>
        {r.submission.as_ref().and_then(|s|s.problem.clone()).map(|s|view!{<p class="stock-rfq-note">{s}</p>})}
        {(!submitted && !cancelled && current).then(||view!{<div class="stock-peer-execution-actions">
            <label><input type="checkbox" prop:checked=move ||confirm.get() disabled=move ||{data.clock.get()>=end || data.peers.plans.pending.get()}
                on:change=move |ev|confirm.set(event_target_checked(&ev))/>"确认本次真实补偿"</label>
            <button type="button" class="row-action stock-peer-recovery-submit" disabled=move ||{!confirm.get() || data.clock.get()>=end || data.peers.plans.pending.get()}
                on:click=move |_|{if confirm.get_untracked(){confirm.set(false);data.peers.plans.recovery_submit.run(submit.clone());}}>"提交补偿"</button>
            <button type="button" class="row-action" disabled=move ||data.peers.plans.pending.get() on:click=move |_|data.peers.plans.recovery_cancel.run(cancel.clone())>"取消补偿预留"</button>
            <span>{move ||if data.clock.get()>=end{"报价已过期 · 不能提交"}else{"报价有效"}}</span>
        </div>})}
        {(submitted && !finality).then(||view!{<button type="button" class="row-action" disabled=move ||{data.peers.plans.pending.get() || data.clock.get()<next_check}
            on:click=move |_|data.peers.plans.recovery_recheck.run(action.clone())>"核对原补偿交易"</button>})}
        <details><summary>"补偿原始依据"</summary><dl class="stock-plan-evidence">
            <div><dt>"原股票差额 / 股"</dt><dd>{r.target.stock_shares}</dd></div>
            <div><dt>"钱包"</dt><dd>{r.cost.wallet_address}</dd></div>
            <div><dt>"原补偿交易"</dt><dd>{r.submission.and_then(|s|s.transaction_id).unwrap_or_else(||if submitted{"按原钱包签名核对".into()}else{"尚未提交".into()})}</dd></div>
        </dl></details>
    </section>}
}
