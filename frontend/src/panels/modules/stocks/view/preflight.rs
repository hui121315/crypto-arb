use super::*;

pub(super) fn panel(asset: String, data: StockData) -> impl IntoView {
    let draft = data.preflight;
    let report = Memo::new(move |_| {
        data.market
            .with(|m| m.value().and_then(|s| s.preflight.clone()))
    });
    let current = Memo::new(move |_| {
        if data.quote_draft_problem().is_some() { return false; }
        data.market.with(|m| {
            m.value().is_some_and(|s| super::super::readiness::preflight_current(s,
                &draft.wallet.get(), &data.budget.get(), data.keyed.get(), data.clock.get()))
        })
    });
    let read_asset = asset.clone();
    view! {<section class="stock-section stock-preflight" id="stock-inventory" aria-label="股票库存与成本交易检查">
        <header><h3>"库存与成本"</h3><span>{move ||if draft.pending.get(){"处理中"}else if report.get().is_some_and(|p|p.source_plan.is_some()){"下一笔库存复查"}else if current.get(){"本次交易检查快照"}else if report.with(Option::is_some){"历史快照 · 构建时自动复核"}else{"尚未检查交易"}}</span></header>
        <form class="stock-preflight-form" on:submit=move |ev|{ev.prevent_default();draft.read.run(read_asset.clone());}>
            <label><span>"Solana 钱包地址"</span><input type="text" autocomplete="off" placeholder="公开地址" aria-label="股票套利 Solana 钱包地址"
                value=move ||draft.wallet.get() prop:value=move ||draft.wallet.get() on:input=move |ev|draft.wallet.set(event_target_value(&ev)) disabled=move ||draft.pending.get() ||draft.build_journal.locked()/></label>
            <button type="submit" class="row-action" disabled=move ||draft.pending.get() || data.pending.get() ||draft.build_journal.locked()>{move ||if draft.pending.get(){"正在交易检查…"}else{"检查库存与成本"}}</button>
        </form>
        <p class="stock-rfq-note">"只读交易检查 · 未签名、未下单"</p>
        {move ||report.get().filter(|p|p.source_plan.is_some()).map(|p|super::restock::panel(p,data))}
        {super::funding::address(asset.clone(),data)}
        {super::stablecoin::panel(asset.clone(),data)}
        <Show when=move ||report.get().is_none_or(|p|p.source_plan.is_none())>{super::conversion_costs::selector(data)}</Show>
        {move ||report.get().map(|p|{
            let fee=p.spot_taker_fee_pct.map(|p|format!("{p}%")).unwrap_or_else(||"未读取".into());
            view!{{p.source_plan.is_none().then(||view!{<div class="stock-quote-meta"><span>{format!("账户现货 taker 费率 {fee}")}</span></div>})}
                {p.problems.into_iter().map(|p|view!{<p class="stock-rfq-note" role="status">{p}</p>}).collect_view()}
            }
        })}
        <Show when=move ||report.get().is_none_or(|p|p.source_plan.is_none())>
        <div class="stock-directions">{[StockChainDirection::Buy, StockChainDirection::Sell].into_iter().map(|direction| {
            let asset=asset.clone();
            let funding_asset=asset.clone();
            let row=Memo::new(move |_|report.get().and_then(|p|p.directions.into_iter().find(|r|r.direction==direction.label())));
            let funding=Memo::new(move |_|report.get().and_then(|p|p.funding.into_iter().find(|r|r.direction==direction)));
            let cost=Memo::new(move |_|data.market.with(|m|m.value().and_then(|s|s.chain_costs.iter().find(|c|c.direction==direction).cloned())));
            let cost_current=Memo::new(move |_|data.quote_draft_problem().is_none() &&data.market.with(|m|m.value().is_some_and(|s|
                super::super::readiness::quote_matches_draft(s,&data.budget.get(),data.keyed.get())
                && s.chain_costs.iter().any(|c|c.direction==direction && c.current(s,draft.wallet.get().trim(),data.clock.get())))));
            let build_problem=Memo::new(move |_| {
                if draft.build_journal.locked() { return Some("原构建请求尚未核对，请先查询原处理结果"); }
                if draft.pending.get() || data.pending.get() { return Some("当前操作尚未完成，请稍候"); }
                if data.quote_pending.get() ||data.monitor_pending.get() { return Some("正在更新询价，请等待完整结果"); }
                data.market.with(|m|m.value().map(|s|super::super::readiness::build_block_reason(s,
                    &draft.wallet.get(), &data.budget.get(), data.keyed.get(), direction, data.clock.get()))
                    .unwrap_or(Some("股票行情尚未就绪")))
            });
            view!{<article class="stock-direction">
                <header><h4>{direction.label()}</h4>
                    <div class="stock-cost-actions">
                    <button type="button" class="row-action stock-cost-action" title="读取库存、试算费用并保存本地计划，不提交订单"
                        on:click=move |_|draft.build.run((asset.clone(),direction))
                        disabled=move ||build_problem.get().is_some()>
                        {move ||if draft.pending.get(){"构建中…"}else{"构建并预留"}}</button>
                    </div>
                </header>
                <p class="stock-rfq-note stock-build-status" role="status">{move ||build_problem.get().unwrap_or("可构建 · 费用与库存待复核 · 尚未预留")}</p>
                {move ||cost.get().map(|c| {
                    let passed=c.simulation_passed;
                    let problems=c.problems.clone();
                    let native_budget=c.native_usdc_budget(c.checked_at_ms);
                    let has_native_budget=native_budget.is_some();
                    let zero_native_budget=native_budget.as_deref()==Some("0");
                    let valuation=c.native_valuation.clone();
                    let complete=c.complete_native_usdc_budget(c.checked_at_ms).is_some();
                    let total_required=c.total_native_required_lamports(c.checked_at_ms).map(|n|n.to_string()).or_else(||c.wallet_required_lamports.clone());
                    let live_cost=c.clone();
                    view!{<div class="stock-chain-cost" data-current=move ||cost_current.get().to_string()>
                        <p class="stock-quote-meta">{move ||if !cost_current.get(){"历史费用快照"}else if passed{"RPC 模拟通过 · 未提交"}else{"RPC 模拟未通过"}}</p>
                        <dl class="stock-direction-values">
                            <div><dt>"消息网络费 / SOL"</dt><dd>{sol(c.network_fee_lamports)}</dd></div>
                            <div><dt>"模拟钱包净扣 / SOL"</dt><dd>{sol(c.wallet_debit_lamports)}</dd></div>
                            <div><dt>"保守周转余额 / SOL"</dt><dd>{sol(total_required)}</dd></div>
                            <div><dt>"SOL 补回预算 / USDC"</dt><dd>{native_budget.unwrap_or_else(||"未知".into())}</dd></div>
                        </dl>
                        <details><summary>"费用明细与付款方"</summary><ul>
                            <li>{format!("报价服务 预算估算 {} SOL · 不代替周转余额",sol(c.wallet_budget_lamports))}</li>
                            {c.provider_fees.into_iter().map(|f|{
                            let payer=match f.payer.as_deref(){Some(p) if p==c.wallet_address=>"本钱包".to_owned(),Some(p)=>format!("其他付款方 {}…{}",p.chars().take(4).collect::<String>(),p.chars().rev().take(4).collect::<String>().chars().rev().collect::<String>()),None=>"付款方未知".into()};
                            let kind=match f.kind.as_str(){"signature"=>"基础网络费","priority"=>"优先费与小费","rent"=>"账户租金估算",_=>"其他费用"};
                            view!{<li title=f.payer.unwrap_or_default()>{format!("{kind} {} SOL · {payer}",sol(f.lamports))}</li>}
                        }).collect_view()}</ul></details>
                        {valuation.map(|v|view!{<details><summary>"SOL 补回报价"</summary><ul>
                            <li>{format!("目标 {} SOL · 最低报价到账 {} SOL",sol(Some(v.native_lamports)),sol(Some(v.quote.minimum_output_raw)))}</li>
                            <li>{format!("路由 {} · {}",v.quote.router,if v.replenishment.is_some(){"交易已模拟，未兑换"}else{"仅报价，未兑换"})}</li>
                            {v.replenishment.map(|p|view!{<li>{format!("补仓消息网络费 {} SOL · 保守原生支出 {} SOL",sol(Some(p.network_fee_lamports)),sol(Some(p.wallet_outflow_lamports)))}</li>
                                <li>{format!("扣补仓支出后最低增加 {} SOL · 补仓周转余额 {} SOL",sol(Some(p.minimum_credit_lamports)),sol(Some(p.wallet_required_lamports)))}</li>})}
                        </ul></details>})}
                        {problems.into_iter().map(|p|view!{<p class="stock-rfq-note">{p}</p>}).collect_view()}
                        <p class="stock-rfq-note">{move ||if has_native_budget {
                            if !cost_current.get() || live_cost.native_usdc_budget(data.clock.get()).is_none(){"SOL 补回预算已失效，不计入当前差额。"}
                            else if zero_native_budget {"模拟净扣为零不代表无需备款；周转余额单独核对。"}
                            else if complete {"补仓自身支出已计入，临时退款不抵备款。当前仍是模拟成本，不代表两腿已成交。"}
                            else{"仅补回模拟净扣 SOL；周转余额含临时支出与钱包免租保留额，退款不抵备款。补仓交易费另计。"}
                        }else{"SOL 费用尚未折合 USDC，不能当作零成本；租金不是固定费用。"}}</p>
                    </div>}
                })}
                {move ||row.get().map(|r|view!{
                    <p class="stock-rfq-note" role="status">{move ||if current.get(){"本次交易检查预算 · 不是实际收益"}else{"历史交易检查 · 不代表当前余额与可执行差额"}}</p>
                    <dl class="stock-direction-values"><div><dt>"另计交易所费用 / USDC"</dt><dd>{r.cex_fee_usdc.unwrap_or_else(||"未知".into())}</dd></div>
                        <div><dt>"另计 SOL 补回预算 / USDC"</dt><dd>{r.native_fee_usdc.unwrap_or_else(||"待核实".into())}</dd></div>
                        <div><dt>"已知费用后差额 / USDC"</dt><dd>{r.after_known_costs_usdc.clone().unwrap_or_else(||"—".into())}</dd></div>
                        {move ||(!draft.conversion_cost_ids.with(Vec::is_empty)).then(||{
                            let adjusted=data.market.with(|m|m.value().and_then(|s|
                                s.difference_after_conversion_costs(&draft.conversion_cost_ids.get(),r.after_known_costs_usdc.as_deref()?).ok()))
                                .unwrap_or_else(||"待核实".into());
                            view!{<div><dt>"归集后预算差额 / USDC"</dt><dd>{adjusted}</dd></div>}
                        })}</dl>
                    <p class="stock-rfq-note">{r.fee_basis}</p>
                    <div class="stock-inventory-list">{r.inventory.into_iter().map(|i|view!{<div class="stock-inventory-row">
                        <strong>{format!("{} · {}",i.location,i.asset)}</strong>
                        <span class="stock-inventory-status" data-status=match i.sufficient {Some(true)=>"enough",Some(false)=>"short",None=>"unknown"}>{match i.sufficient {Some(true)=>"数量足够",Some(false)=>"余额不足",None=>"待核实"}}</span>
                        <span>"需要 "{comparison::quantity(Some(i.required.unwrap_or_else(||"待构建".into())))}</span>
                        <span>"可用 "{comparison::quantity(Some(i.available.unwrap_or_else(||"未知".into())))}</span>
                    </div>}).collect_view()}</div>
                    {r.transfer_problem.map(|p|view!{<p class="stock-rfq-note">{p}</p>})}
                    <details class="stock-preflight-blockers"><summary>{format!("待完成 {} 项",r.blockers.len())}</summary><ul>{r.blockers.into_iter().map(|p|view!{<li>{p}</li>}).collect_view()}</ul></details>
                })}
                {move ||funding.get().filter(|f|!f.needs.is_empty()).map(|f|super::funding::needs(f,data,funding_asset.clone(),report.get().map_or(0,|p|p.checked_at_ms),None))}
            </article>}
        }).collect_view()}</div>
        </Show>
    </section>}
}

fn sol(raw: Option<String>) -> String {
    raw.and_then(|r| r.parse::<u128>().ok())
        .map(|n| {
            let value = format!("{}.{:09}", n / 1_000_000_000, n % 1_000_000_000);
            value.trim_end_matches('0').trim_end_matches('.').to_owned()
        })
        .unwrap_or_else(|| "未知".into())
}
