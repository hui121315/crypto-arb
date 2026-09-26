use super::*;

pub(super) fn panel(data: StockData, p: &StockPeerPlan) -> impl IntoView {
    let target = p.peer_native_target().ok();
    let limit = RwSignal::new(String::new());
    let build = p.clone();
    let enabled = p.clone();
    (target.is_some() || !p.native_topups.is_empty()).then(||view!{
        <section class="stock-peer-receipts stock-peer-native-topup" aria-label="SOL 费用补回">
            <h4>"SOL 费用补回"</h4>
            {target.map(|(lamports,_)|view!{
                <p class="stock-rfq-note">{format!("待补回 {} SOL",stock_chain_quantity(&lamports.to_string(),9).unwrap_or_default())}</p>
                <form class="stock-peer-recovery-build" on:submit=move |ev|{ev.prevent_default();
                    if !data.peers.plans.pending.get_untracked() && stock_peer_recovery_limit(&limit.get_untracked()).is_some(){
                        data.peers.plans.native_build.run(StockPeerNativeTopupRequest{plan_id:build.plan_id.clone(),revision:build.revision,usdc_limit:limit.get_untracked()});
                    }
                }>
                    <label>"最多支出 USDC"<input type="text" inputmode="decimal" autocomplete="off" placeholder="0.00" aria-label="SOL 补回最多支出 USDC"
                        prop:value=move ||limit.get() on:input=move |ev|limit.set(event_target_value(&ev))/></label>
                    <button type="submit" class="row-action" disabled=move ||data.peers.plans.pending.get() || stock_peer_recovery_limit(&limit.get()).is_none() || !enabled.peer_native_available(data.clock.get())>"获取 SOL 补回报价"</button>
                </form>
            })}
            {p.native_topups.iter().cloned().enumerate().map(|(i,r)|record(data,p,i,r)).collect_view()}
        </section>
    })
}
fn record(
    data: StockData,
    p: &StockPeerPlan,
    index: usize,
    r: StockPeerNativeTopup,
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
    let submitted = r.terms.submission.is_some();
    let cancelled = r.cancelled_at_ms.is_some();
    let finality = r
        .terms
        .submission
        .as_ref()
        .is_some_and(|s| s.receipt.is_some());
    let actual = r.actual_changes(p);
    let observed = r.observed_changes();
    let phase = if cancelled {
        "已取消 · 未提交"
    } else if finality {
        if matches!(&actual,Ok(Some((_,sol,_))) if *sol>0) {
            "SOL 补回到账已核实"
        } else {
            "补回失败或收支不符 · 需核对"
        }
    } else if submitted {
        "提交结果待核对 · 不重发"
    } else {
        "待确认 · 未提交"
    };
    let end = r
        .terms
        .valuation
        .replenishment
        .as_ref()
        .map_or(0, |p| p.valid_until_ms);
    let next = r
        .terms
        .submission
        .as_ref()
        .map_or(0, |s| s.next_recheck_at_ms);
    let confirm = RwSignal::new(false);
    let current = index + 1 == p.native_topups.len();
    let mut prefix = p.clone();
    prefix.native_topups.truncate(index);
    let basis_ok = r.validate(&prefix, p.updated_at_ms).is_ok();
    let quantity = |raw: &str, decimals| {
        stock_chain_quantity(raw, decimals).unwrap_or_else(|| "待核对".into())
    };
    let tx = r
        .terms
        .submission
        .as_ref()
        .and_then(|s| s.transaction_id.clone())
        .unwrap_or_else(|| "尚无最终处理结果".into());
    view! {<section class="stock-peer-recovery-record" aria-label=format!("SOL 补回 {}",index+1)>
        <header><strong>{format!("SOL 补回 {}",index+1)}</strong><span>{phase}</span></header>
        <dl class="stock-peer-plan-summary">
            <div><dt>"本次 USDC 限额"</dt><dd>{r.usdc_limit}</dd></div>
            <div><dt>"报价支出 / USDC"</dt><dd>{quantity(&r.terms.valuation.quote.input_raw,6)}</dd></div>
            <div><dt>"最低净补回 / SOL"</dt><dd>{r.terms.valuation.replenishment.as_ref().map(|p|quantity(&p.minimum_credit_lamports,9))}</dd></div>
            <div><dt>"补回网络费 / SOL"</dt><dd>{r.terms.valuation.replenishment.as_ref().map(|p|quantity(&p.network_fee_lamports,9))}</dd></div>
        </dl>
        {observed.ok().flatten().map(|(cash,sol,_)|view!{<dl class="stock-peer-plan-summary">
            <div><dt>"实际 USDC 收支"</dt><dd>{quantity(&cash.to_string(),6)}</dd></div>
            <div><dt>"实际钱包 SOL 净变化"</dt><dd>{quantity(&sol.to_string(),9)}</dd></div>
        </dl>})}
        {r.terms.submission.as_ref().and_then(|s|s.problem.clone()).map(|s|view!{<p class="stock-rfq-note">{s}</p>})}
        {(!submitted && !cancelled && current).then(||view!{<div class="stock-peer-execution-actions">
            <label><input type="checkbox" prop:checked=move ||confirm.get() disabled=move ||{!basis_ok || data.clock.get()>=end || data.peers.plans.pending.get()}
                on:change=move |ev|confirm.set(event_target_checked(&ev))/>"确认本次真实 SOL 补回"</label>
            <button type="button" class="row-action stock-peer-native-submit" disabled=move ||{!basis_ok || !confirm.get() || data.clock.get()>=end || data.peers.plans.pending.get()}
                on:click=move |_|{if confirm.get_untracked(){confirm.set(false);data.peers.plans.native_submit.run(submit.clone());}}>"提交 SOL 补回"</button>
            <button type="button" class="row-action" disabled=move ||data.peers.plans.pending.get() on:click=move |_|data.peers.plans.native_cancel.run(cancel.clone())>"取消 SOL 补回"</button>
            <span>{move ||if data.clock.get()>=end{"报价已过期 · 不能提交"}else{"报价有效"}}</span>
        </div>})}
        {(submitted && !finality).then(||view!{<button type="button" class="row-action" disabled=move ||{data.peers.plans.pending.get() || data.clock.get()<next}
            on:click=move |_|data.peers.plans.native_recheck.run(action.clone())>"核对原 SOL 补回"</button>})}
        <details><summary>"补回交易记录"</summary><dl class="stock-plan-evidence"><div><dt>"原交易"</dt><dd>{tx}</dd></div></dl></details>
    </section>}
}
