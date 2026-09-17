use super::*;

fn direction_name(d: StockChainDirection) -> &'static str {
    if d == StockChainDirection::Buy {
        "链买 / Kraken 卖"
    } else {
        "Kraken 买 / 链卖"
    }
}

pub(super) fn builder(data: StockData) -> impl IntoView {
    let reserved = Memo::new(move |_| {
        data.market.with(|m| {
            m.value()
                .is_some_and(|s| s.peer_plans.iter().any(|p| p.holds_funds(data.clock.get())))
        })
    });
    let eligible = Memo::new(move |_| {
        data.market.with(|m| {
            m.value().and_then(|s| s.peer.as_ref()).is_some_and(|p| {
                p.selection.venue == "kraken"
                    && p.selection.product == StockPeerProduct::Spot
                    && p.share_unit_verified
            })
        })
    });
    let busy = move || {
        reserved.get()
            || data.peers.plans.pending.get()
            || data.peers.checking.get()
            || data.peers.order_checking.get()
            || data.peers.funding_checking.get()
            || data.peers.pending.get()
            || data.preflight.pending.get()
            || data.quote_pending.get()
    };
    view! { {move ||eligible.get().then(||view!{
        <section class="stock-peer-plan-builder" aria-label="Kraken 双边计划构建">
            <header><h4>"双边计划"</h4><span>"先预留 · 确认后双边提交"</span></header>
            <label class="stock-peer-plan-wallet">"Solana 钱包"
                <input type="text" autocomplete="off" spellcheck="false" placeholder="钱包公开地址" prop:value=move ||data.preflight.wallet.get() value=move ||data.preflight.wallet.get()
                    on:input=move |ev|data.preflight.wallet.set(event_target_value(&ev))/>
            </label>
            {move ||reserved.get().then(||view!{<p class="stock-rfq-note" role="status">"已有双边资金占用，请查看原计划状态"</p>})}
            <div class="stock-directions">{[StockChainDirection::Buy,StockChainDirection::Sell].into_iter().map(|direction| {
                let draft=Memo::new(move |_|data.market.with(|m| {
                    let s=m.value().ok_or_else(||"等待行情".to_owned())?;
                    prepare_peer_order_check(s,StockPeerOrderCheckRequest{asset:s.security.as_ref().ok_or("请选择股票")?.asset.clone(),
                        selection:s.peer.as_ref().ok_or("请选择市场")?.selection.clone(),direction},data.clock.get())
                }));
                view!{<section class="stock-direction"><header><h4>{direction_name(direction)}</h4></header>
                    {move ||draft.get().ok().map(|d|view!{<dl class="stock-direction-values">
                        <div><dt>"股票数量"</dt><dd>{format!("{} 股",d.quantity)}</dd></div>
                        <div><dt>"Kraken 限价"</dt><dd>{format!("{} {}",d.limit_price,d.quote_asset)}</dd></div>
                    </dl>})}
                    {move ||draft.get().err().map(|e|view!{<p class="stock-rfq-note">{e}</p>})}
                    <button type="button" class="row-action" disabled=move ||busy() ||draft.get().is_err() ||data.preflight.wallet.get().trim().is_empty()
                        on:click=move |_|data.peers.plans.build.run((data.preflight.wallet.get_untracked(),direction))>
                        {move ||if data.peers.plans.pending.get(){"正在核对计划…"}else{"保存双边计划"}}
                    </button>
                </section>}
            }).collect_view()}</div>
        </section>
    })} }
}

pub(super) fn history(data: StockData) -> impl IntoView {
    let rows = Memo::new(move |_| {
        data.market
            .with(|m| m.value().map(|s| s.peer_plans.clone()).unwrap_or_default())
    });
    let problem = Memo::new(move |_| {
        data.peers.plans.problem.get().or_else(|| {
            data.market
                .with(|m| m.value().and_then(|s| s.peer_plan_problem.clone()))
        })
    });
    view! {<section class="stock-section stock-peer-plans" aria-label="Kraken 双边计划记录" hidden=move ||rows.get().is_empty() && problem.get().is_none()>
        <header><h3>"Kraken 双边计划记录"</h3>
            <button type="button" class="row-action" disabled=move ||data.peers.plans.pending.get() on:click=move |_|data.peers.plans.refresh.run(())>"核对记录"</button>
        </header>
        {move ||problem.get().map(|e|view!{<p class="stock-problem" role="alert">{e}</p>})}
        <For each=move ||rows.get() key=|p|(p.plan_id.clone(),p.revision) children=move |p|record(data,p)/>
    </section>}
}

fn record(data: StockData, p: StockPeerPlan) -> impl IntoView {
    let end = p.terms.reserved_until_ms;
    let market_end = p.terms.market_valid_until_ms;
    let phase = p.phase;
    let submitted = phase == StockPeerPlanPhase::SubmissionUnknown;
    let confirmed = RwSignal::new(false);
    let execution = StockPeerExecutionRequest {
        plan_id: p.plan_id.clone(),
        revision: p.revision,
        confirm_live: true,
    };
    let recheck = StockPlanRevisionRequest {
        plan_id: p.plan_id.clone(),
        revision: p.revision,
    };
    let proof = receipts(&p);
    let recovery = super::peer_recovery::panel(data, &p);
    let conversion = super::peer_conversion::panel(data, &p);
    let inventory = super::peer_inventory::panel(data, &p);
    let native_topup = super::peer_native_topup::panel(data, &p);
    let accounting = data
        .market
        .with_untracked(|m| {
            m.value().and_then(|s| {
                s.peer_accounting
                    .iter()
                    .find(|a| a.plan_id == p.plan_id && a.source_revision == p.revision)
                    .cloned()
            })
        })
        .unwrap_or_else(|| p.accounting());
    let actual = submitted.then(|| actual_accounting(&p, accounting));
    let revision = StockPlanRevisionRequest {
        plan_id: p.plan_id.clone(),
        revision: p.revision,
    };
    let not_cancelled = phase == StockPeerPlanPhase::Reserved;
    let reserve_state = move || {
        if submitted {
            "提交已记录 · 资金保持占用".into()
        } else if !not_cancelled {
            "已取消 · 未下单".into()
        } else if data.clock.get() >= end {
            "预留已到期 · 未下单".into()
        } else {
            format!("已预留 · {}s", (end - data.clock.get() + 999) / 1000)
        }
    };
    let quote = p.terms.draft.quote_asset.clone();
    let min_out = if p.request.direction == StockChainDirection::Buy {
        format!(
            "{} 股",
            shared_types::stocks::comparison::shares(
                &p.terms.basis.chain_cost.quote.minimum_output_raw,
                &p.terms.basis.chain_cost.mint
            )
            .map(|n| n.normalize().to_string())
            .unwrap_or_else(|| "未知".into())
        )
    } else {
        format!(
            "{} USDC",
            stock_chain_quantity(&p.terms.basis.chain_cost.quote.minimum_output_raw, 6)
                .unwrap_or_else(|| "未知".into())
        )
    };
    let mint = p.terms.basis.chain_cost.mint.address.clone();
    let ticker = p.terms.basis.security.ticker.clone();
    view! {<article class="stock-plan-record stock-peer-plan-record">
        <header><div><strong>{format!("{} · {}",p.request.selection.native_symbol,direction_name(p.request.direction))}</strong><p class="stock-plan-phase">{reserve_state}</p></div>
            {(!submitted).then(||view!{<button type="button" class="row-action" disabled=move ||{!not_cancelled ||data.clock.get()>=end ||data.peers.plans.pending.get()}
                on:click=move |_|data.peers.plans.cancel.run(revision.clone())>"取消预留"</button>})}
        </header>
        <dl class="stock-peer-plan-summary">
            <div><dt>"费用后差额估算"</dt><dd>{comparison::quantity(Some(p.terms.after_known_costs_usdc.clone()))}" USDC"</dd></div>
            <div><dt>"原报价状态"</dt><dd>{move ||if submitted{"已冻结 · 仅核对原交易"}else if data.clock.get()>=market_end{"已过期，需重建"}else{"有效，尚未提交"}}</dd></div>
            <div><dt>"股票 / 原生限价"</dt><dd>{format!("{} 股 / {} {}",p.terms.draft.quantity,p.terms.draft.limit_price,quote)}</dd></div>
            <div><dt>"链上最低到账"</dt><dd>{min_out}</dd></div>
        </dl>
        {proof}
        {actual}
        {recovery}
        {inventory}
        {conversion}
        {native_topup}
        {submitted.then(||view!{<div class="stock-peer-execution-actions"><button type="button" class="row-action" disabled=move ||data.peers.plans.pending.get()
            on:click=move |_|data.peers.plans.recheck.run(recheck.clone())>"核对原双边交易"</button><span>"未核账前不释放占用；不自动重发"</span></div>})}
        {not_cancelled.then(||view!{
            <div class="stock-peer-execution-actions">
                <label><input type="checkbox" prop:checked=move ||confirmed.get() disabled=move ||{data.clock.get()>=market_end ||data.peers.plans.pending.get()}
                    on:change=move |ev|confirmed.set(event_target_checked(&ev))/>"确认本次真实双边交易"</label>
                <button type="button" class="row-action stock-peer-submit" disabled=move ||{!confirmed.get() ||data.clock.get()>=market_end ||data.peers.plans.pending.get()}
                    on:click=move |_|{if confirmed.get_untracked(){confirmed.set(false);data.peers.plans.execute.run(execution.clone());}}>"提交双边计划"</button>
            </div>})}
        <h4>"资金预留"</h4>
        <div class="stock-peer-plan-allocations">{p.terms.allocations.into_iter().map(|a|{
            let label=if a.location=="Solana" && a.asset==mint {format!("{ticker} 股票代币")}else{a.asset.clone()};
            view!{<div class="stock-plan-allocation"><span title=a.asset>{format!("{} · {}",a.location,label)}</span><strong>{a.quantity}</strong></div>}
        }).collect_view()}</div>
        <details><summary>"原始依据与限制"</summary><dl class="stock-plan-evidence">
            <div><dt>"计划编号"</dt><dd>{p.plan_id}</dd></div>
            <div><dt>"钱包"</dt><dd>{p.request.wallet_address}</dd></div>
            <div><dt>"链上股票合约"</dt><dd>{p.terms.basis.chain_cost.mint.address}</dd></div>
            <div><dt>"原始差额估值 / USDC"</dt><dd>{p.terms.after_known_costs_usdc}</dd></div>
            <div><dt>"Kraken 交易费用预算"</dt><dd>{format!("{} {}",p.terms.cex_fee_quote,p.terms.draft.quote_asset)}</dd></div>
            <div><dt>"最小到账取整余量"</dt><dd>{format!("{} 股",p.terms.remainder_shares)}</dd></div>
            <div><dt>"币种与资金路径"</dt><dd>"原生币种分别预留；换汇以实际回执为准，不按一比一计价。不同发行方股票不能直接互充，不是已锁定利润。"</dd></div>
        </dl></details>
    </article>}
}

fn receipts(p: &StockPeerPlan) -> impl IntoView {
    p.cex_order.as_ref().map(|cex| {
    let chain = p.chain_submission.as_ref();
    let cex_state = if cex.evidence_conflict {
        "回执冲突 · 待核对"
    } else if cex.rejection_proven() {
        "已拒绝 · 未成交"
    } else if cex.receipt_complete() {
        if cex.phase == StockCexOrderPhase::Filled {
            "已成交 · 费用已核实"
        } else {
            "订单已终止 · 核对剩余敞口"
        }
    } else if !cex.fills.is_empty() {
        "已收到成交 · 费用或终态待核对"
    } else if cex.submission_ack.as_ref().is_some_and(|a| a.accepted) {
        "已接收 · 尚无成交回执"
    } else {
        "结果未明 · 只查询原订单"
    };
    let chain_state = match chain.and_then(|r| r.receipt.as_ref()) {
        Some(r) if !r.succeeded => "链上失败 · 需核对补偿",
        Some(r) if !r.within_plan => "已执行 · 到账不满足原计划",
        Some(_) => "链上回执已确认",
        None if chain.is_some_and(|r| r.provider_acknowledged) => "Provider 已接收 · 等待链上回执",
        None => "结果未明 · 只查询原交易",
    };
    let problems = [
        cex.problem.clone(),
        chain.and_then(|r| r.problem.clone()),
        p.execution_problem.clone(),
        p.cex_history.problem.clone(),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    view!{<section class="stock-peer-receipts" aria-label="双边原始回执">
        <h4>"双边原始回执"</h4><dl class="stock-peer-plan-summary">
            <div><dt>"Kraken"</dt><dd>{cex_state}</dd></div><div><dt>"Solana"</dt><dd>{chain_state}</dd></div>
        </dl>
        {problems.into_iter().map(|e|view!{<p class="stock-rfq-note">{e}</p>}).collect_view()}
        <details><summary>"原订单与链上流水"</summary><dl class="stock-plan-evidence">
            <div><dt>"Kraken 客户端编号"</dt><dd>{cex.client_order_id.clone()}</dd></div>
            <div><dt>"Kraken 订单编号"</dt><dd>{cex.order_id.clone().unwrap_or_else(||"等待原订单回执".into())}</dd></div>
            <div><dt>"历史回补"</dt><dd>{if p.cex_history.attempts == 0 {"尚未补查 · 实时回执优先".into()} else if cex.receipt_complete() && p.cex_history.problem.is_none() {format!("已核齐 · 查询 {} 次",p.cex_history.attempts)} else if p.cex_history.attempts >= 6 {format!("自动回补已暂停 · 查询 {} 次，可手动核对原订单",p.cex_history.attempts)} else {format!("等待核齐 · 查询 {} 次，不重发订单",p.cex_history.attempts)}}</dd></div>
            <div><dt>"原链上交易"</dt><dd>{chain.and_then(|r|r.transaction_id.clone()).unwrap_or_else(||"待按原钱包签名核对".into())}</dd></div>
        </dl></details>
    </section>}
    })
}

fn actual_accounting(p: &StockPeerPlan, a: StockPeerAccounting) -> impl IntoView {
    let state = match a.status {
        StockAccountingStatus::AwaitingReceipts => "收支待核齐",
        StockAccountingStatus::NeedsReview => "存在差额或疑点 · 未结算",
        StockAccountingStatus::LegsReconciled => "原交易收支已核齐 · 未结算",
    };
    let value = |n: Option<String>, unit: &str| {
        n.map(|s| format!("{s} {unit}"))
            .unwrap_or_else(|| "待核对".into())
    };
    let mint = p.terms.basis.chain_cost.mint.address.clone();
    let ticker = p.terms.basis.security.ticker.clone();
    let network_label = if p.recoveries.iter().any(|r| r.submission.is_some()) {
        "网络费 · 含补偿交易"
    } else {
        match p.chain_submission.as_ref().and_then(|s| s.receipt.as_ref()) {
            Some(r) if r.fee_payer != p.request.wallet_address => "网络费 · 其他地址支付",
            Some(_) => "网络费 · 已含于钱包变化",
            None => "网络手续费",
        }
    };
    view! {<section class="stock-peer-receipts stock-peer-accounting" aria-label="双边实际收支">
        <h4>"实际收支"</h4><p class="stock-plan-phase">{state}</p>
        <dl class="stock-peer-plan-summary">
            <div><dt>"股票份额净变化"</dt><dd>{value(a.net_stock_shares,"股")}</dd></div>
            <div><dt>"Kraken 原币手续费"</dt><dd>{value(a.cex_fee_quote,&a.quote_asset)}</dd></div>
            <div><dt>"钱包 SOL 实际变化"</dt><dd>{value(a.wallet_sol_change,"SOL")}</dd></div>
            <div><dt>{network_label}</dt><dd>{value(a.network_fee_sol,"SOL")}</dd></div>
        </dl>
        <dl class="stock-peer-plan-summary stock-peer-native-cash">{a.cash_totals.into_iter().map(|(asset,n)|view!{
            <div><dt>{format!("{asset} 已核实费后收支")}</dt><dd>{format!("{n} {asset}")}</dd></div>
        }).collect_view()}</dl>
        {a.recovery_target.map(|t|view!{<p class="stock-rfq-note stock-peer-recovery-target">{format!("补偿参考：{} {} 个链上股票代币；需重新报价和确认",if t.direction==StockChainDirection::Buy{"补买"}else{"卖出多余的"},stock_chain_quantity(&t.stock_raw,p.terms.basis.chain_cost.mint.decimals).unwrap_or_else(||"待核对".into()))}</p>})}
        {a.problems.into_iter().map(|p|view!{<p class="stock-rfq-note">{p}</p>}).collect_view()}
        {(!a.remaining.is_empty()).then(||view!{<ul class="stock-peer-remaining">{a.remaining.into_iter().map(|p|view!{<li>{p}</li>}).collect_view()}</ul>})}
        <details><summary>"资产位置与原始数量"</summary><div class="stock-peer-plan-allocations">
            {a.movements.into_iter().map(|m|{
                let label=if m.asset==mint {format!("{ticker} 链上代币")}else{m.asset.clone()};
                view!{<div class="stock-plan-allocation"><span title=m.asset>{format!("{} · {}",m.location,label)}</span><strong>{m.quantity}</strong></div>}
            }).collect_view()}
        </div></details>
    </section>}
}
