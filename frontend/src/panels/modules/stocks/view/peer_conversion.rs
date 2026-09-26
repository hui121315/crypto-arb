use super::*;

pub(super) fn panel(data: StockData, p: &StockPeerPlan) -> impl IntoView {
    let gap = p.peer_conversion_gap().ok();
    let present = gap.is_some() || !p.conversions.is_empty();
    let dust = gap.is_some_and(|n| {
        n.is_sign_positive()
            && p.conversions.last().is_some_and(|c| {
                c.market
                    .is_fresh_at(data.clock.get_untracked(), c.draft.metadata_max_age_ms())
                    && c.market
                        .min_notional
                        .and_then(|m| stock_exact_decimal(&m.to_string()).ok())
                        .is_some_and(|minimum| n < minimum)
            })
    });
    let remainder = gap.map(|n| {
        format!(
            "保留原币余款 {} {} · 低于上次核实的最低交易额，不按 1:1 计入 USDC 盈亏",
            n.normalize(),
            p.terms.draft.quote_asset
        )
    });
    let build = p.clone();
    let enabled = p.clone();
    let limit = RwSignal::new(String::new());
    present.then(||view!{<section class="stock-peer-receipts stock-peer-conversion" aria-label="原币换汇">
        <h4>"原币换汇"</h4>
        {dust.then(||view!{<p class="stock-rfq-note">{remainder}</p>})}
        {gap.filter(|_|!dust).map(|n|{
            let label=if n.is_sign_positive(){"最低收到 USDC"}else{"最多支出 USDC"};
            let quote=p.terms.draft.quote_asset.clone();
            view!{<p class="stock-rfq-note">{format!("原交易现金差额 {} {} · {}",n.normalize(),quote,if n.is_sign_positive(){"余款换为 USDC"}else{"用 USDC 补回原币缺口"})}</p>
                <form class="stock-peer-recovery-build" on:submit=move |ev|{
                    ev.prevent_default();
                    if stock_peer_recovery_limit(&limit.get_untracked()).is_some() && !data.peers.plans.pending.get_untracked(){
                        data.peers.plans.conversion_build.run(StockPeerConversionRequest{plan_id:build.plan_id.clone(),revision:build.revision,usdc_limit:limit.get_untracked()});
                    }
                }>
                    <label>{label}<input type="text" inputmode="decimal" autocomplete="off" placeholder="0.00" aria-label=label prop:value=move ||limit.get() on:input=move |ev|limit.set(event_target_value(&ev))/></label>
                    <button type="submit" class="row-action" disabled=move ||{data.peers.plans.pending.get() || stock_peer_recovery_limit(&limit.get()).is_none() || !enabled.peer_conversion_available(data.clock.get())}>"获取换汇报价"</button>
                </form>}
        })}
        {p.conversions.iter().cloned().enumerate().map(|(i,c)|record(data,p,i,c)).collect_view()}
    </section>})
}

fn record(
    data: StockData,
    p: &StockPeerPlan,
    index: usize,
    c: StockPeerConversion,
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
    let submitted = c.order.is_some();
    let cancelled = c.cancelled_at_ms.is_some();
    let complete = c.order.as_ref().is_some_and(|o| o.receipt_complete());
    let actual = c.native_cash_changes().ok();
    let phase = if cancelled {
        "已取消 · 未提交"
    } else if c.order.as_ref().is_some_and(|o| o.evidence_conflict) {
        "处理结果冲突 · 待核对"
    } else if c.cash_changes().is_ok() {
        if c.order
            .as_ref()
            .is_some_and(|o| o.phase == StockCexOrderPhase::Filled)
        {
            "换汇成交与费用已核实"
        } else {
            "订单已终止 · 未换汇"
        }
    } else if complete {
        "实际收支不符 · 待核对"
    } else if submitted {
        "换汇处理结果待核对 · 不重发"
    } else {
        "待确认 · 未提交"
    };
    let end = c.valid_until_ms;
    let next_check = c.history.next_check_at_ms;
    let current = index + 1 == p.conversions.len();
    let confirm = RwSignal::new(false);
    let buy = c.buy_usdc();
    let quote = c.draft.quote_asset.clone();
    let problem = c
        .order
        .as_ref()
        .and_then(|o| o.problem.clone())
        .or(c.history.problem.clone())
        .or_else(|| complete.then(|| c.cash_changes().err()).flatten());
    let limit_label = if buy {
        "最低收到 USDC"
    } else {
        "最多支出 USDC"
    };
    let fees = c
        .order
        .as_ref()
        .map(|o| {
            o.fills
                .iter()
                .map(|f| {
                    let amount = f
                        .fees
                        .as_ref()
                        .map(|fees| {
                            if fees.is_empty() {
                                "明确零费用".to_owned()
                            } else {
                                fees.iter()
                                    .map(|fee| format!("{} {}", fee.quantity, fee.asset))
                                    .collect::<Vec<_>>()
                                    .join(" + ")
                            }
                        })
                        .unwrap_or_else(|| "费用未返回".into());
                    format!("{}：{amount}", f.execution_id)
                })
                .collect::<Vec<_>>()
                .join("；")
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "尚无逐笔成交".into());
    view! {<section class="stock-peer-recovery-record" aria-label=format!("换汇 {}",index+1)>
        <header><strong>{format!("换汇 {} · {}",index+1,if buy{format!("{quote} → USDC")}else{format!("USDC → {quote}")})}</strong><span>{phase}</span></header>
        <dl class="stock-peer-plan-summary">
            <div><dt>{limit_label}</dt><dd>{c.request.usdc_limit}</dd></div>
            <div><dt>"换汇数量 / USDC"</dt><dd>{c.draft.quantity}</dd></div>
            <div><dt>"原币净收支预算"</dt><dd>{format!("{} {quote}",c.quote_change)}</dd></div>
            <div><dt>"其中手续费预算"</dt><dd>{format!("{} {quote}",c.fee_quote)}</dd></div>
        </dl>
        {actual.map(|flows|view!{<dl class="stock-peer-plan-summary">{flows.into_iter().map(|(asset,delta)|view!{<div><dt>{format!("实际 {asset} 收支")}</dt><dd>{delta}</dd></div>}).collect_view()}</dl>})}
        <p class="stock-rfq-note">"按实际成交与原币手续费核账；原币零头单独保留。换汇完成不等于库存补齐或利润结算。"</p>
        {problem.map(|s|view!{<p class="stock-rfq-note">{s}</p>})}
        {(!submitted && !cancelled && current).then(||view!{<div class="stock-peer-execution-actions">
            <label><input type="checkbox" prop:checked=move ||confirm.get() disabled=move ||{data.clock.get()>=end || data.peers.plans.pending.get()} on:change=move |ev|confirm.set(event_target_checked(&ev))/>"确认本次真实换汇"</label>
            <button type="button" class="row-action stock-peer-conversion-submit" disabled=move ||{!confirm.get() || data.clock.get()>=end || data.peers.plans.pending.get()} on:click=move |_|{if confirm.get_untracked(){confirm.set(false);data.peers.plans.conversion_submit.run(submit.clone());}}>"提交换汇"</button>
            <button type="button" class="row-action" disabled=move ||data.peers.plans.pending.get() on:click=move |_|data.peers.plans.conversion_cancel.run(cancel.clone())>"取消换汇报价"</button>
            <span>{move ||if data.clock.get()>=end{"报价已过期 · 不能提交"}else{"报价有效"}}</span>
        </div>})}
        {(submitted && !complete).then(||view!{<button type="button" class="row-action" disabled=move ||{data.peers.plans.pending.get() || data.clock.get()<next_check} on:click=move |_|data.peers.plans.conversion_recheck.run(action.clone())>"核对原换汇订单"</button>})}
        <details><summary>"换汇原始依据"</summary><dl class="stock-plan-evidence">
            <div><dt>"原币现金差额"</dt><dd>{format!("{} {quote}",c.native_gap)}</dd></div>
            <div><dt>"原生市场 / 限价"</dt><dd>{format!("{} / {}",c.draft.request.selection.native_symbol,c.draft.limit_price)}</dd></div>
            <div><dt>"逐笔原币手续费"</dt><dd>{fees}</dd></div>
            <div><dt>"原订单标识"</dt><dd>{c.order.map(|o|o.client_order_id).unwrap_or_else(||"尚未提交".into())}</dd></div>
        </dl></details>
    </section>}
}
