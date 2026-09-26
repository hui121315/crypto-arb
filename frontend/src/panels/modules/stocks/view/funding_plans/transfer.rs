use super::*;

pub(super) fn controls(plan: &StockFundingPlan, data: StockData) -> impl IntoView {
    let inbound = plan.request.target == StockFundingTarget::Backpack;
    let ready = plan.phase_at(data.clock.get_untracked()) == StockFundingPlanPhase::Reserved;
    let t = plan.transfer.clone();
    let needs_prepare = inbound && ready && t.is_none();
    let prepare = StockPlanRevisionRequest {
        plan_id: plan.plan_id.clone(),
        revision: plan.revision,
    };
    let submit = StockFundingSubmitRequest {
        plan_id: plan.plan_id.clone(),
        revision: plan.revision,
        confirm_live: true,
        two_factor_token: None,
    };
    let read = StockPlanCancelRequest {
        plan_id: plan.plan_id.clone(),
    };
    let destination = plan.terms.destination.clone();
    let confirmed = RwSignal::new(false);
    let expiry = plan.terms.valid_until_ms;
    let holds = plan.phase.holds_funds();
    let deposited = plan.phase == StockFundingPlanPhase::Deposited;
    let blocked = Memo::new(move |_| {
        data.preflight.pending.get()
            || data.clock.get() >= expiry
            || data
                .market
                .with(|m| m.value().is_none_or(|s| s.funding_problem.is_some()))
    });
    view! {
        {needs_prepare.then(||view!{<button class="row-action" type="button" disabled=move||blocked.get()
            on:click=move |_|data.preflight.funding_prepare_transfer.run(prepare.clone())>"核算 Solana 转账与费用"</button>})}
        {t.map(|t|{
            let waiting=t.submitted_at_ms.is_some();
            let next=t.last_query_at_ms.unwrap_or(0).saturating_add(5000);
            let fee=t.preparation.network_fee_lamports;
            let retained=t.preparation.retained_sol_lamports;
            let creation=t.preparation.account_creation.as_ref().map(|c|c.rent_budget_lamports);
            let chain_checked=t.receipt.is_some();
            let continuing=t.deposit_scan.as_ref().is_some_and(|s|s.completed_at_ms.is_none() && s.scanned_rows>0);
            view!{<div class="stock-order-receipt stock-funding-transfer">
                <dl class="stock-funding-amounts">
                    <div><dt>"转账网络费 / SOL"</dt><dd>{format!("{}.{:09}",fee/1_000_000_000,fee%1_000_000_000)}</dd></div>
                    <div><dt>"保留套利备款 / SOL"</dt><dd>{format!("{}.{:09}",retained/1_000_000_000,retained%1_000_000_000)}</dd></div>
                    {creation.map(|rent|view!{<div><dt>"接收账户创建上限 / SOL"</dt><dd>{format!("{}.{:09}",rent/1_000_000_000,rent%1_000_000_000)}</dd></div>})}
                </dl>
                {creation.is_some().then(||view!{<p class="stock-rfq-note">"含接收账户创建。SOL 留在 Backpack 的接收账户，不计作可退回备款；已由其他交易创建时按实际支出记账。"</p>})}
                {(!waiting && ready).then(||view!{<details class="stock-funding-confirmation">
                    <summary>"确认转入 Backpack"</summary>
                    <dl class="stock-plan-evidence"><div><dt>"本次 Backpack 充值地址 / Solana"</dt><dd>{destination}</dd></div></dl>
                    <p class="stock-rfq-note">"这是实际链上转账。发送后不能本地撤销，Backpack 确认入账前不释放资金占用。"</p>
                    <div class="stock-monitor-control"><label><input type="checkbox" prop:checked=move||confirmed.get()
                        disabled=move||blocked.get() on:change=move|ev|confirmed.set(event_target_checked(&ev))/><span>"确认本次资产、数量、全部费用及收款地址"</span></label>
                        <button type="button" class="row-action" disabled=move||blocked.get() || !confirmed.get()
                            on:click=move |_|{if blocked.get_untracked() || !confirmed.get_untracked(){return;}confirmed.set(false);data.preflight.funding_submit.run(submit.clone());}>"提交本次链上转账"</button>
                    </div>
                </details>})}
                {t.problem.map(|p|view!{<p class=if deposited{"stock-rfq-note"}else{"stock-problem"} role="status">{p}</p>})}
                {t.evidence_conflict.map(|p|view!{<p class="stock-problem" role="alert">{format!("原入账处理结果冲突：{p}。原始收支和资金占用已保留，后续正常回复不会自动解除。")}</p>})}
                {t.deposit_scan.map(|s|view!{<p class="stock-rfq-note stock-deposit-scan" role="status">{if s.completed_at_ms.is_some(){format!("本轮入账历史已查完 · {} 条 · {}",s.scanned_rows,if s.matched{"已找到原交易"}else{"未找到原交易"})}else{format!("入账历史已核对 {} 条 · 进度已保存，重启后可继续；查完前保留占用",s.scanned_rows)}}</p>})}
                {waiting.then(||view!{<dl class="stock-plan-evidence">
                    <div><dt>"原链上交易"</dt><dd>{t.transaction_hash}</dd></div>
                    <div><dt>"RPC 接收"</dt><dd>{if t.acknowledged{"已接收 · 非到账证明"}else if chain_checked{"原回复未确认 · 已核对链上最终结果"}else{"回复未确认"}}</dd></div>
                    {t.receipt.map(|r|view!{
                        <div><dt>"链上最终结果"</dt><dd>{if r.succeeded{"已最终确认"}else{"已失败"}}</dd></div>
                        <div><dt>"实际转出 / 原始单位"</dt><dd>{r.source_debit_raw.to_string()}</dd></div>
                        <div><dt>"充值地址到账 / 原始单位"</dt><dd>{r.destination_credit_raw.to_string()}</dd></div>
                        <div><dt>"实际网络费 / lamports"</dt><dd>{r.network_fee_lamports.to_string()}</dd></div>
                        {creation.is_some().then(||view!{<div><dt>"实际接收账户支出 / lamports"</dt><dd>{r.account_creation_lamports.to_string()}</dd></div>})}
                        <div><dt>"SOL 实际扣账 / lamports"</dt><dd>{r.wallet_debit_lamports.to_string()}</dd></div>
                    })}
                    <div><dt>"Backpack 入账状态"</dt><dd>{t.deposit.as_ref().map(|d|d.status.clone()).unwrap_or("待核对".into())}</dd></div>
                    {t.deposit.map(|d|view!{<div><dt>"交易所入账数量"</dt><dd>{format!("{} {}",d.quantity,d.symbol)}</dd></div>})}
                </dl>})}
                {(waiting && holds).then(||view!{<button type="button" class="row-action" disabled=move||data.preflight.pending.get() || data.clock.get()<next
                    on:click=move |_|data.preflight.funding_recheck.run(read.clone())>{if continuing{"继续核对原转账与 Backpack 入账"}else{"核对原转账与 Backpack 入账"}}</button>})}
            </div>}
        })}
    }
}
