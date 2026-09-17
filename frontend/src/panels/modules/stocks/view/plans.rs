use super::*;

pub(super) fn panel(data: StockData) -> impl IntoView {
    let plans = Memo::new(move |_| {
        data.market
            .with(|m| m.value().map(|s| s.plans.clone()).unwrap_or_default())
    });
    view! {<section class="stock-section stock-plans" aria-label="股票执行计划">
        <header><h3>"执行计划"</h3><span>"备款 · 双腿收支"</span></header>
        {move ||data.market.with(|m|m.value().and_then(|s|s.plan_problem.clone())).map(|p|view!{<p class="stock-problem" role="alert">{p}</p>})}
        {move ||plans.with(Vec::is_empty).then(||view!{<p class="stock-rfq-note">"暂无已保存计划"</p>})}
        <For each=move ||plans.get() key=|p|(p.plan_id.clone(),p.revision) children=move |plan| {
            let cancel_id=plan.plan_id.clone();
            let recheck_id=plan.plan_id.clone();
            let submitted=plan.cex_order.is_some() || plan.rfq_acceptance.is_some() || plan.chain_submission.is_some();
            let recheck_label=if plan.two_leg_started_at_ms.is_some(){"核对两腿回执"}else if plan.chain_submission.is_some(){"核对原链上交易"}else if plan.rfq_acceptance.is_some(){"核对原 RFQ"}else{"核对原订单"};
            let receipt=order_receipt(&plan);
            let rfq=rfq_receipt(&plan);
            let chain=chain_receipt(&plan);
            let accounting=submitted.then(||accounting(&plan));
            let paired=plan.two_leg_started_at_ms.is_some();
            let accounting_status=plan.accounting().status;
            let report=plan.accounting();
            let settled=plan.phase==StockPlanPhase::Settled;
            let can_settle=plan.two_leg_started_at_ms.is_some() && report.can_settle();
            let needs_sol=report.status==StockAccountingStatus::LegsReconciled && report.fee_basis_matched==Some(true) && report.net_sol_change.as_deref().is_some_and(|s|s.starts_with('-'));
            let topup_unresolved=plan.native_topups.iter().any(|t|t.submission.as_ref().is_some_and(|s|s.receipt.is_none()));
            let topup_ready_after=plan.native_topups.last().filter(|t|t.submission.is_none()).and_then(|t|t.valuation.replenishment.as_ref()).map(|p|p.valid_until_ms).unwrap_or(0);
            let topup_limit=plan.native_topups.len()>=8;
            let settle_request=StockPlanRevisionRequest{plan_id:plan.plan_id.clone(),revision:plan.revision};
            let topup_request=settle_request.clone();
            let topups=native_topups(&plan, data);
            let recovery=super::recovery::panel(data,&plan);
            let fee=plan.terms.cex_fee_budget.clone();
            let execution=(!submitted && plan.phase==StockPlanPhase::Reserved && plan.terms.cex_instruction.is_some() && plan.terms.chain_cost.transaction.is_some())
                .then(||super::execution::confirmation(data,&plan,StockExecutionAction::Pair,plan.terms.market_valid_until_ms));
            let state=plan.clone();
            let phase=Memo::new(move |_|state.phase_at(data.clock.get()));
            let valid_until=plan.terms.market_valid_until_ms;
            let reserved_until=plan.terms.reserved_until_ms;
            let instruction=instruction_label(plan.terms.cex_instruction.as_ref());
            view!{<article class="stock-plan-record">
                <header><div><strong>{format!("{} · {}",plan.request.asset,plan.request.direction.label())}</strong>
                    <span class="stock-plan-phase" data-phase=move ||format!("{:?}",phase.get())>{move ||match phase.get(){StockPlanPhase::Reserved=>"已预留 · 未下单",StockPlanPhase::Cancelled=>"已取消",StockPlanPhase::Expired=>"预留已到期",StockPlanPhase::Settled=>"交易已收尾 · 预留已释放",StockPlanPhase::SubmissionUnknown=>match (paired,accounting_status){(true,StockAccountingStatus::LegsReconciled)=>"两腿已核对 · 等待收尾",(true,StockAccountingStatus::NeedsReview)=>"执行需处置 · 保留占用",_=>"提交待核对 · 保留占用"}}}</span></div>
                    <div class="stock-plan-actions">
                    {(submitted && !settled).then(||view!{<button type="button" class="row-action" disabled=move ||data.preflight.pending.get() on:click=move |_|data.preflight.recheck.run(recheck_id.clone())>{recheck_label}</button>})}
                    {(!submitted).then(||view!{<button type="button" class="row-action" disabled=move ||data.preflight.pending.get() || phase.get()!=StockPlanPhase::Reserved on:click=move |_|data.preflight.cancel.run(cancel_id.clone())>"取消预留"</button>})}
                    {(needs_sol && !settled).then(||view!{<button type="button" class="row-action" disabled=move ||data.preflight.pending.get() || topup_unresolved || topup_limit || data.clock.get()<topup_ready_after on:click=move |_|data.preflight.topup.run(topup_request.clone())>"试算 SOL 补回"</button>})}
                    {(can_settle && !settled).then(||view!{<button type="button" class="row-action" disabled=move ||data.preflight.pending.get() on:click=move |_|data.preflight.settle.run(settle_request.clone())>"结束并释放预留"</button>})}
                    </div>
                </header>
                <p class="stock-rfq-note">{move ||if phase.get()==StockPlanPhase::Reserved {format!("备款剩余 {} 秒 · {}",reserved_until.saturating_sub(data.clock.get()).max(0)/1000,if data.clock.get()>=valid_until{"报价已过期，执行前需重新核验"}else{"报价仍有效，尚非执行许可"})}else if settled{"预留已释放 · 收支与备款已归档".into()}else{"保留原计划与资金记录，不自动重发".into()}}</p>
                <div class="stock-inventory-list">{plan.terms.allocations.into_iter().map(|a|view!{<div class="stock-plan-allocation"><span>{format!("{} · {}",a.location,a.asset)}</span><strong>{comparison::quantity(Some(a.quantity))}</strong></div>}).collect_view()}</div>
                {execution}
                {accounting}
                {recovery}
                {topups}
                {receipt}
                {rfq}
                {chain}
                <details><summary>"计划凭据"</summary><dl class="stock-plan-evidence">
                    <div><dt>"计划编号"</dt><dd>{plan.plan_id}</dd></div>
                    <div><dt>"钱包"</dt><dd>{plan.request.wallet_address}</dd></div>
                    <div><dt>"交易所股数"</dt><dd>{plan.terms.cex_shares}</dd></div>
                    <div><dt>"原始指令"</dt><dd>{instruction}</dd></div>
                    <div><dt>"当时已知费用后差额 / USDC"</dt><dd>{plan.terms.after_known_costs_usdc}</dd></div>
                    {fee.map(|f|view!{<div><dt>"交易所费用预算"</dt><dd>{match f.basis {StockCexFeeBasis::OrderBookQuote{..}=>format!("{} USDC · 实扣以回执为准",f.additional_fee),StockCexFeeBasis::RfqIncluded{..}=>"RFQ 报价已含费，不再叠加现货费".into()}}</dd></div>
                        <div><dt>"交易所扣费后 USDC 预估变化"</dt><dd>{f.net_quote_change}</dd></div>})}
                </dl></details>
            </article>}
        }/>
    </section>}
}

fn native_topups(plan: &StockExecutionPlan, data: StockData) -> impl IntoView {
    let id = plan.plan_id.clone();
    let original = plan.clone();
    let settled = plan.phase == StockPlanPhase::Settled;
    plan.native_topups.clone().into_iter().enumerate().map(move |(index,t)| {
        let request=StockTopupRecheckRequest{plan_id:id.clone(),index};
        let submitted=t.submission.is_some();
        let confirmed=t.submission.as_ref().and_then(|s|s.receipt.as_ref());
        let needs_recheck=submitted && confirmed.is_none() && !settled;
        let status=match confirmed {Some(r) if r.succeeded && r.within_plan=>"已补回 · 实际收支已计入",Some(_)=>"补回未完成 · 保留费用",None if submitted=>"补回结果待核对",None=>"试算已保存 · 尚未提交"};
        let expiry=t.valuation.replenishment.as_ref().map(|p|p.valid_until_ms).unwrap_or(0);
        let execution=(!submitted && !settled && index+1==original.native_topups.len()).then(||super::execution::confirmation(data,&original,StockExecutionAction::NativeTopup{index},expiry));
        let problem=t.submission.and_then(|s|s.problem);
        view!{<div class="stock-order-receipt" aria-label="SOL 补回记录">
            <strong>{format!("SOL 补回 {} · {status}",index+1)}</strong>
            <dl class="stock-plan-evidence">
                <div><dt>"补回目标 / SOL"</dt><dd>{stock_chain_quantity(&t.valuation.native_lamports,9).unwrap_or_else(||"待核实".into())}</dd></div>
                <div><dt>"USDC 投入"</dt><dd>{stock_chain_quantity(&t.valuation.quote.input_raw,6).unwrap_or_else(||"待核实".into())}</dd></div>
            </dl>
            {(!submitted).then(||view!{<span class="stock-rfq-note">{move ||if data.clock.get()>=expiry{"试算已过期，需重新构建"}else{"报价有效 · 未签名或发送"}}</span>})}
            {execution}
            {needs_recheck.then(||view!{<button type="button" class="row-action" disabled=move ||data.preflight.pending.get() on:click=move |_|data.preflight.recheck_topup.run(request.clone())>"核对原补回交易"</button>})}
            {problem.map(|p|view!{<p class="stock-rfq-note">{p}</p>})}
        </div>}
    }).collect_view()
}

fn accounting(plan: &StockExecutionPlan) -> impl IntoView {
    let report = plan.accounting();
    let status = match report.status {
        StockAccountingStatus::AwaitingReceipts => "双腿收支待核对",
        StockAccountingStatus::NeedsReview => "双腿存在待处置缺口",
        StockAccountingStatus::LegsReconciled => "两腿原币收支已核对",
    };
    view! {<div class="stock-order-receipt" aria-label="双腿实际收支">
        <strong>{status}</strong>
        <dl class="stock-plan-evidence">
            <div><dt>"USDC 净变化（含补偿与补回）"</dt><dd>{report.net_usdc_change.unwrap_or_else(||"待核实".into())}</dd></div>
            <div><dt>"股票库存合计变化 / 股"</dt><dd>{report.net_stock_shares.unwrap_or_else(||"待核实".into())}</dd></div>
            <div><dt>"钱包 SOL 净变化"</dt><dd>{report.net_sol_change.unwrap_or_else(||"待核实".into())}</dd></div>
        </dl>
        {report.problems.into_iter().map(|p|view!{<p class="stock-rfq-note">{p}</p>}).collect_view()}
        <details><summary>"实际资产位置"</summary><dl class="stock-plan-evidence">
            {report.movements.into_iter().map(|m|view!{<div><dt>{format!("{} · {}",m.location,m.asset)}</dt><dd>{m.quantity}</dd></div>}).collect_view()}
        </dl></details>
    </div>}
}

fn chain_receipt(plan: &StockExecutionPlan) -> impl IntoView {
    let settled = plan.phase == StockPlanPhase::Settled;
    let stock_mint = plan.terms.chain_cost.mint.address.clone();
    let asset = plan.request.asset.clone();
    plan.chain_submission.clone().map(move |row| {
        let status=match &row.receipt {
            Some(r) if !r.succeeded=>"链上已失败 · 保留实际费用",
            Some(r) if r.within_plan=>"链上已最终确认 · 原币收支已核对",
            Some(_)=>"链上已成交 · 实际收支偏离计划",
            None if row.provider_acknowledged=>"Provider 已回复 · 链上结果待核对",
            None=>"链上提交结果未知 · 不重复发送",
        };
        let id=row.transaction_id.clone().unwrap_or_else(||"待原交易回执确认".into());
        view!{<div class="stock-order-receipt" aria-label="链上交易回执">
            <strong>{status}</strong><span class="stock-rfq-note">{if settled {"交易收支已归档 · 预留已释放"}else{"两腿资金闭环尚未完成 · 保留占用"}}</span>
            <dl class="stock-plan-evidence"><div><dt>"链上交易"</dt><dd>{id}</dd></div>
                <div><dt>"原交易核对次数"</dt><dd>{row.recheck_attempts}</dd></div></dl>
            {row.problem.map(|p|view!{<p class="stock-problem" role="status">{p}</p>})}
            {row.receipt.map(move |r|view!{
                <dl class="stock-plan-evidence">
                    <div><dt>"网络费 / SOL"</dt><dd>{stock_chain_quantity(&r.network_fee_lamports,9).unwrap_or_else(||"未知".into())}</dd></div>
                    <div><dt>"钱包 SOL 净变化（已含费用）"</dt><dd>{stock_chain_quantity(&r.wallet_native_change_lamports,9).unwrap_or_else(||"未知".into())}</dd></div>
                    {r.asset_changes.into_iter().map(|a|{let label=if a.mint==stock_mint{format!("{asset} 链上代币净变化")}else if a.mint==shared_types::stocks::comparison::SOLANA_USDC{"USDC 净变化".into()}else{format!("代币 {} 净变化",a.mint)};
                        view!{<div><dt>{label}</dt><dd title=format!("{} · 原始数量 {}",a.mint,a.raw_change)>{stock_chain_quantity(&a.raw_change,a.decimals).unwrap_or_else(||format!("{} 原始单位",a.raw_change))}</dd></div>}
                    }).collect_view()}
                </dl>
                {r.problems.into_iter().map(|p|view!{<p class="stock-problem" role="status">{p}</p>}).collect_view()}
                <details><summary>"链上回执凭据"</summary><dl class="stock-plan-evidence">
                    <div><dt>"确认等级"</dt><dd>"Finalized"</dd></div><div><dt>"区块槽位"</dt><dd>{r.slot}</dd></div>
                    <div><dt>"网络费付款方"</dt><dd>{r.fee_payer}</dd></div>
                </dl></details>
            })}
        </div>}
    })
}

fn order_receipt(plan: &StockExecutionPlan) -> impl IntoView {
    let settled = plan.phase == StockPlanPhase::Settled;
    let instruction = plan.terms.cex_instruction.clone();
    let has_chain = plan.chain_submission.is_some();
    plan.cex_order.clone().map(move |order| {
        let complete=order.receipt_complete();
        let status=if order.evidence_conflict {"回执冲突 · 停止自动处理"} else {match order.phase {
            StockCexOrderPhase::SubmissionUnknown=>"提交结果待确认",
            StockCexOrderPhase::Open=>"订单已接收 · 未终结",
            StockCexOrderPhase::Filled if complete=>"交易所已成交 · 收支已核对",
            StockCexOrderPhase::Filled=>"交易所已成交 · 明细/费用待核",
            StockCexOrderPhase::Cancelled=>"交易所已取消",
            StockCexOrderPhase::Expired=>"交易所已过期",
            StockCexOrderPhase::Rejected=>"交易所已拒绝",
        }};
        let totals=if order.fills.is_empty() && order.executed_quantity.is_none() {None} else {order.fill_totals()};
        let changes=instruction.as_ref().and_then(|i|order.net_asset_changes(i));
        let missing_changes=changes.is_none();
        let review=if complete {"回执齐全".into()} else if order.recheck.paused {"自动核对已暂停 · 可手动核对原订单".into()} else {format!("等待回执 · 已核对 {}/6 次",order.recheck.attempts)};
        view!{<div class="stock-order-receipt" aria-label="交易所订单回执">
            <strong>{status}</strong><span class="stock-rfq-note">{if settled {"交易收支已归档 · 预留已释放"}else if has_chain {"按双腿实际收支核对 · 资金仍保留占用"}else{"链上腿未完成 · 资金仍保留占用"}}</span>
            <dl class="stock-plan-evidence">
                <div><dt>"远端订单"</dt><dd>{order.order_id.unwrap_or_else(||"待确认".into())}</dd></div>
                <div><dt>"已核实成交 / 股"</dt><dd>{totals.map(|(q,_)|q.normalize().to_string()).unwrap_or_else(||"待核实".into())}</dd></div>
                <div><dt>"已核实成交额 / USDC"</dt><dd>{totals.map(|(_,v)|v.normalize().to_string()).unwrap_or_else(||"待核实".into())}</dd></div>
                <div><dt>"回执核对"</dt><dd>{review}</dd></div>
            </dl>
            {order.problem.map(|p|view!{<p class="stock-problem" role="status">{p}</p>})}
            {changes.map(|changes|view!{<dl class="stock-plan-evidence" aria-label="扣费后原币收支">{changes.into_iter().map(|(asset,quantity)|view!{<div><dt>{format!("{asset} 净变化")}</dt><dd>{quantity}</dd></div>}).collect_view()}</dl>})}
            {missing_changes.then(||view!{<p class="stock-rfq-note">"扣费后到账待核实，未计为套利收益"</p>})}
            <details><summary>{format!("原始成交与费用 · {} 笔",order.fills.len())}</summary>
                <dl class="stock-plan-evidence">{order.fills.into_iter().map(|f|view!{<div><dt>{format!("成交 {}",f.trade_id)}</dt><dd>{format!("{} 股 × {} USDC · {}",f.quantity,f.price,f.fee.map(|fee|format!("费用 {} {}",fee.quantity,fee.asset)).unwrap_or_else(||"费用待核实".into()))}</dd></div>}).collect_view()}</dl>
            </details>
        </div>}
    })
}

pub(super) fn rfq_receipt(plan: &StockExecutionPlan) -> impl IntoView {
    let has_chain = plan.chain_submission.is_some();
    plan.rfq_acceptance.clone().map(move |r| {
        let phase=super::rfq::phase_label(&r, r.updated_at_ms);
        let accepted=r.acceptance.clone();
        let complete=r.phase==StockRfqPhase::Filled && !r.settlement_pending();
        let review=if accepted.as_ref().is_some_and(|a|a.evidence_conflict){"回执冲突 · 保留原记录".into()}
            else if complete{"成交数量已核对 · 费用与净到账待核实".into()}
            else if r.settlement.paused{"自动核对已暂停，可手动核对原 RFQ".into()}
            else{format!("等待原请求回执 · 已核对 {}/6 次",r.settlement.attempts)};
        let next=super::rfq::next_step_label(&r);
        view!{<div class="stock-order-receipt" aria-label="RFQ 接受与结算回执">
            <strong>{phase}</strong><span class="stock-rfq-note">{if has_chain {"按双腿实际收支核对 · 资金仍保留占用"}else{"链上腿未完成 · 资金仍保留占用"}}</span>
            <dl class="stock-plan-evidence">
                <div><dt>"原 RFQ"</dt><dd>{r.rfq_id.unwrap_or_else(||"待核实".into())}</dd></div>
                <div><dt>"已提交的报价"</dt><dd>{accepted.map(|a|a.quote_id).unwrap_or_else(||"待核实".into())}</dd></div>
                <div><dt>"实际成交 / 股"</dt><dd>{r.executed_quantity.unwrap_or_else(||"待核实".into())}</dd></div>
                <div><dt>"成交金额 / USDC"</dt><dd>{r.executed_quote_quantity.unwrap_or_else(||"待核实".into())}</dd></div>
                <div><dt>"回执核对"</dt><dd>{review}</dd></div>
                <div><dt>"扣费后到账"</dt><dd>"待核实，未计为套利收益"</dd></div>
            </dl>
            {r.problem.map(|p|view!{<p class="stock-problem" role="status">{p}</p>})}
            {next.map(|text|view!{<p class="stock-rfq-note">{text}</p>})}
            <details><summary>{format!("原始 RFQ 成交 · {} 笔",r.fills.len())}</summary><dl class="stock-plan-evidence">
                {r.fills.into_iter().map(|f|view!{<div><dt>{format!("报价 {}",f.quote_id)}</dt><dd>{format!("{} 股 · 成交额 {} USDC · 价格 {}",f.quantity,f.quote_quantity,f.price)}</dd></div>}).collect_view()}
            </dl></details>
        </div>}
    })
}

fn instruction_label(instruction: Option<&StockCexInstruction>) -> String {
    match instruction {
        Some(StockCexInstruction::OrderBook {
            symbol,
            side,
            quantity,
            limit_price,
            ..
        }) => format!(
            "{symbol} · {} {quantity} 股 · 限价 {limit_price} USDC · 全部成交或取消 · 不借款",
            if *side == StockRfqSide::Ask {
                "卖"
            } else {
                "买"
            }
        ),
        Some(StockCexInstruction::AcceptRfq {
            rfq_id,
            quote_id,
            symbol,
            side,
            quantity,
            taker_price,
        }) => format!(
            "{symbol} · {} {quantity} 股 · {taker_price} USDC · RFQ {rfq_id} · 报价 {quote_id}",
            if *side == StockRfqSide::Ask {
                "卖"
            } else {
                "买"
            }
        ),
        None => "旧计划未编译交易指令，仅可查看或取消预留".into(),
    }
}
