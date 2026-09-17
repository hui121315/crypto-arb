use super::*;

pub(super) fn panel(data: StockData, plan: &StockExecutionPlan) -> impl IntoView {
    let target = plan.recovery_target().ok();
    let loss = RwSignal::new(String::new());
    let id = plan.plan_id.clone();
    let revision = plan.revision;
    let last_expiry = plan
        .recoveries
        .last()
        .filter(|r| r.submission.is_none() && r.cancelled_at_ms.is_none())
        .map(|r| r.cost.valid_until_ms)
        .unwrap_or(0);
    let limit_reached = plan.recoveries.len() >= 8;
    let original = plan.clone();
    let rows = plan.recoveries.clone();
    view! {
        {target.map(|t|view!{<div class="stock-order-receipt" aria-label="股票差额处置">
            <strong>"股票差额处置"</strong>
            <dl class="stock-plan-evidence">
                <div><dt>"实际库存合计变化 / 股"</dt><dd>{t.stock_shares}</dd></div>
                <div><dt>"补偿方向"</dt><dd>{if t.direction==StockChainDirection::Buy{"链上补买股票"}else{"链上卖回多余股票"}}</dd></div>
                <div><dt>"链上代币数量"</dt><dd>{stock_chain_quantity(&t.stock_raw,plan.terms.chain_cost.mint.decimals).unwrap_or_else(||"待核实".into())}</dd></div>
            </dl>
            <div class="stock-rfq-form stock-recovery-form">
                <label><span>"整笔损失上限 / USDC"</span><input type="text" inputmode="decimal" placeholder="0.00"
                    prop:value=move ||loss.get() on:input=move |ev|loss.set(event_target_value(&ev))/></label>
                <button type="button" class="row-action" disabled={move ||data.preflight.pending.get() ||limit_reached ||data.clock.get()<last_expiry ||stock_recovery_loss_limit(&loss.get()).is_none()}
                    on:click=move |_|data.preflight.recovery.run(StockRecoveryBuildRequest{plan_id:id.clone(),revision,max_loss_usdc:loss.get_untracked()})>"试算补偿"</button>
            </div>
            <p class="stock-rfq-note">"计入原交易、补偿与 SOL 补回预算；不自动转移两边库存。"</p>
            {limit_reached.then(||view!{<p class="stock-problem">"补偿尝试已达上限，请人工核对，不再生成交易。"</p>})}
        </div>})}
        {rows.into_iter().enumerate().map(move |(index,row)| {
            let req=StockRecoveryActionRequest{plan_id:original.plan_id.clone(),revision:original.revision,index};
            let cancel=req.clone();
            let submitted=row.submission.is_some();
            let cancelled=row.cancelled_at_ms.is_some();
            let receipt=row.submission.as_ref().and_then(|s|s.receipt.as_ref());
            let unresolved=submitted && receipt.is_none();
            let settled=original.phase==StockPlanPhase::Settled;
            let status=match receipt {
                Some(r) if r.succeeded && r.within_plan=>"已成交 · 已计入收支",
                Some(_)=>"未完成 · 已保留费用",
                None if cancelled=>"已取消 · 未提交",
                None if submitted=>"结果待核对 · 不重发",
                _=>"试算已保存 · 未提交",
            };
            let transaction_id=row.submission.as_ref().and_then(|s|s.transaction_id.clone());
            let native=receipt.map(|r|stock_chain_quantity(&r.wallet_native_change_lamports,9).unwrap_or_else(||"未知".into()));
            let expiry=row.cost.valid_until_ms;
            let confirmation=(!submitted && !cancelled && !settled && index+1==original.recoveries.len()).then(||super::execution::confirmation(data,&original,StockExecutionAction::Recovery{index},expiry));
            let cash=if row.target.direction==StockChainDirection::Buy{&row.cost.quote.input_raw}else{&row.cost.quote.minimum_output_raw};
            let cash=stock_chain_quantity(cash,6).unwrap_or_else(||"待核实".into());
            let problem=row.submission.and_then(|s|s.problem);
            view!{<div class="stock-order-receipt" aria-label="股票补偿记录">
                <strong>{format!("补偿 {} · {status}",index+1)}</strong>
                <dl class="stock-plan-evidence">
                    <div><dt>"动作"</dt><dd>{if row.target.direction==StockChainDirection::Buy{"链上补买"}else{"链上卖回"}}</dd></div>
                    <div><dt>{if row.target.direction==StockChainDirection::Buy{"USDC 投入"}else{"最低到账 / USDC"}}</dt><dd>{cash}</dd></div>
                    <div><dt>"整笔保守净变化 / USDC"</dt><dd>{row.minimum_net_usdc}</dd></div>
                    <div><dt>"允许损失上限 / USDC"</dt><dd>{row.max_loss_usdc}</dd></div>
                    {native.map(|n|view!{<div><dt>"本次实际 SOL 变化"</dt><dd>{n}</dd></div>})}
                    {transaction_id.map(|id|view!{<div><dt>"原交易编号"</dt><dd>{id}</dd></div>})}
                </dl>
                {confirmation}
                {(!submitted && !cancelled && !settled).then(||view!{<button type="button" class="row-action" disabled=move ||data.preflight.pending.get()
                    on:click=move |_|data.preflight.cancel_recovery.run(cancel.clone())>"取消本次补偿"</button>})}
                {(unresolved && !settled).then(||view!{<button type="button" class="row-action" disabled=move ||data.preflight.pending.get()
                    on:click=move |_|data.preflight.recheck_recovery.run(req.clone())>"核对原补偿交易"</button>})}
                {problem.map(|p|view!{<p class="stock-rfq-note">{p}</p>})}
            </div>}
        }).collect_view()}
    }
}
