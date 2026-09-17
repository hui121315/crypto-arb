use super::*;
mod transfer;

pub(super) fn panel(data: StockData) -> impl IntoView {
    let rows = Memo::new(move |_| {
        data.market.with(|m| {
            m.value()
                .map(|s| s.funding_plans.clone())
                .unwrap_or_default()
        })
    });
    let problem = Memo::new(move |_| {
        data.market
            .with(|m| m.value().and_then(|s| s.funding_problem.clone()))
    });
    move || {
        (!rows.with(Vec::is_empty) ||problem.with(Option::is_some)).then(||view!{
        <section class="stock-section stock-funding-plans" aria-label="股票补库计划">
            <header><h3>"补库计划"</h3><span>"转出 · 到账 · 扣账"</span></header>
            {move ||problem.get().map(|p|view!{<p class="stock-problem" role="alert">{p}</p>})}
            <p class="stock-rfq-note">"保存不转账。转账须单独确认；链上确认、交易所入账与费用分别记录。"</p>
            <For each=move ||rows.get() key=|p|(p.plan_id.clone(),p.revision) children=move |p|{
                let state=p.clone();
                let phase=Memo::new(move |_|state.phase_at(data.clock.get()));
                let cancel=StockPlanRevisionRequest{plan_id:p.plan_id.clone(),revision:p.revision};
                let units=p.request.funding_asset.clone();
                let until=p.terms.valid_until_ms;
                let controls=withdrawal_controls(&p,data);
                let transfer=transfer::controls(&p,data);
                let tracking=p.clone();
                view!{<article class="stock-funding-plan">
                    <header><div><strong>{format!("{} · {} → {}",units,p.terms.need.source,p.terms.need.target)}</strong>
                        <span class="stock-plan-phase" data-phase=move ||format!("{:?}",phase.get())>{move ||phase.get().label()}</span></div>
                        <button type="button" class="row-action" disabled=move ||data.preflight.pending.get() ||phase.get()!=StockFundingPlanPhase::Reserved ||problem.with(Option::is_some)
                            on:click=move |_|data.preflight.funding_cancel.run(cancel.clone())>"取消补库预留"</button>
                    </header>
                    <dl class="stock-funding-amounts">
                        <div><dt>"目标缺口"</dt><dd>{format!("{} {}",p.terms.need.shortfall.as_deref().unwrap_or("未知"),units)}</dd></div>
                        <div><dt>"转出数量"</dt><dd>{format!("{} {}",p.terms.quantity,units)}</dd></div>
                        <div><dt>"保守备款"</dt><dd>{format!("{} {}",p.terms.source_budget,units)}</dd></div>
                        <div><dt>"资金占用"</dt><dd>{move ||if phase.get()==StockFundingPlanPhase::Reserved{format!("剩余 {} 秒",until.saturating_sub(data.clock.get()).max(0).saturating_add(999)/1000)}else if phase.get().holds_funds(){"核验期间保留".into()}else{"已释放".into()}}</dd></div>
                    </dl>
                    {move ||followup_status(&tracking,data.clock.get()).map(|text|view!{<p class="stock-rfq-note stock-funding-followup" role="status">{text}</p>})}
                    {controls}
                    {transfer}
                    <details><summary>"补库凭据"</summary><dl class="stock-plan-evidence">
                        <div><dt>"用途"</dt><dd>{format!("{} · {}",p.request.security_asset,p.request.direction.label())}</dd></div>
                        <div><dt>"接收地址 / Solana"</dt><dd>{p.terms.destination}</dd></div>
                        <div><dt>"关联钱包"</dt><dd>{p.request.wallet_address}</dd></div>
                        <div><dt>"最低入账目标 / 原始单位"</dt><dd>{p.terms.minimum_credit_raw}</dd></div>
                        <div><dt>"股票显示倍率"</dt><dd>{p.terms.mint.ui_multiplier}</dd></div>
                        {p.terms.withdrawal_capacity.map(|c|view!{<div><dt>"账户不借款可提上限"</dt><dd>{format!("{} {}",c.quantity,c.asset)}</dd></div>})}
                        <div><dt>"计划编号"</dt><dd>{p.plan_id}</dd></div>
                    </dl>
                    <p class="stock-rfq-note">{match p.request.target {
                        StockFundingTarget::Backpack=>"转账金额与 SOL 网络费分开记录。链上转出不等于交易所入账，原签名只查询、不重复发送。",
                        StockFundingTarget::Solana=>"不自动借款或赎回。保守备款不等于实际扣账，交易所回报费用与链上网络费分开记录；不明费用不按零处理。",
                    }}</p></details>
                </article>}
            }/>
        </section>
    })
    }
}

fn followup_status(plan: &StockFundingPlan, now: i64) -> Option<String> {
    if plan.withdrawal.as_ref().is_some_and(|w| w.evidence_conflict.is_some()) {
        return Some("自动核验已暂停 · 回执冲突需人工核对；可查询原记录，不会重新提现".into());
    }
    if !plan.funding_receipt_pending() {
        return plan.followup.as_ref().map(|_|match plan.phase {
            StockFundingPlanPhase::Received => "到账核验已结束 · 交易所扣账与费用仍待核清".into(),
            StockFundingPlanPhase::Deposited => "到账核验已结束 · Backpack 已确认入账".into(),
            _ => "自动核验已停止 · 请查看原交易收支".into(),
        });
    }
    let attempts=plan.followup.as_ref().map_or(0,|f|f.attempts);
    if plan.followup.as_ref().is_some_and(|f|f.paused) {
        let reason=plan.followup.as_ref().and_then(|f|f.problem.as_deref())
            .unwrap_or("本轮自动核验次数已用完，可手动核对原记录");
        return Some(format!("自动核验已暂停 · {attempts}/{STOCK_FUNDING_FOLLOWUP_LIMIT} · {reason}"));
    }
    plan.funding_followup_at().map(|at| {
        let seconds=at.saturating_sub(now).max(0).saturating_add(999)/1000;
        format!("原记录自动核验 · {attempts}/{STOCK_FUNDING_FOLLOWUP_LIMIT} · {}",if seconds==0{"等待本次结果".into()}else{format!("下次约 {seconds} 秒")})
    })
}

#[cfg(test)]
mod tests;

fn withdrawal_controls(plan: &StockFundingPlan, data: StockData) -> impl IntoView {
    let submitted = plan.withdrawal.clone();
    let ready = plan.phase_at(data.clock.get_untracked()) == StockFundingPlanPhase::Reserved
        && plan.request.target == StockFundingTarget::Solana;
    let destination = plan.terms.destination.clone();
    let request = StockFundingSubmitRequest {
        plan_id: plan.plan_id.clone(),
        revision: plan.revision,
        confirm_live: true,
        two_factor_token: None,
    };
    let read = StockPlanCancelRequest {
        plan_id: plan.plan_id.clone(),
    };
    let confirmed = RwSignal::new(false);
    let token = RwSignal::new(String::new());
    let expiry = plan.terms.valid_until_ms;
    let blocked = Memo::new(move |_| {
        data.preflight.pending.get()
            || data.clock.get() >= expiry
            || data
                .market
                .with(|m| m.value().is_none_or(|s| s.funding_problem.is_some()))
    });
    view! {
        {ready.then(||view!{<details class="stock-funding-confirmation">
            <summary>"确认 Backpack 提现"</summary>
            <dl class="stock-plan-evidence"><div><dt>"本次接收地址 / Solana"</dt><dd>{destination}</dd></div></dl>
            <p class="stock-rfq-note">"这是实际提币，不是套利下单。提交后不能本地取消；2FA 仅用于本次请求，不保存。需要旅行规则补充资料的账户暂不支持在此提交。"</p>
            <label class="stock-funding-token">"Backpack 签发的 2FA 凭证（按账户要求）"
                <input type="password" autocomplete="off" maxlength="2048" prop:value=move ||token.get()
                    disabled=move ||blocked.get() on:input=move |ev|token.set(event_target_value(&ev))/>
            </label>
            <div class="stock-monitor-control"><label><input type="checkbox" prop:checked=move ||confirmed.get()
                disabled=move ||blocked.get() on:change=move |ev|confirmed.set(event_target_checked(&ev))/><span>"确认上述资产、数量及接收地址，本次实盘提现"</span></label>
                <button type="button" class="row-action" disabled=move ||blocked.get() ||!confirmed.get()
                    on:click=move |_|{
                        if blocked.get_untracked() || !confirmed.get_untracked(){return;}
                        confirmed.set(false);
                        let mut r=request.clone();
                        let value=token.get_untracked();token.set(String::new());
                        r.two_factor_token=(!value.is_empty()).then_some(value);
                        data.preflight.funding_submit.run(r);
                    }>"提交本次提现"</button>
            </div>
        </details>})}
        {submitted.map(|w|{
            let next=w.last_query_at_ms.unwrap_or(0).saturating_add(5_000);
            let received=w.receipt.is_some();
            view!{<div class="stock-order-receipt stock-funding-receipt">
                {w.evidence_conflict.map(|s|view!{<p class="stock-problem" role="alert">{format!("原提现回执冲突：{s}。原始到账和资金占用已保留，后续正常回复不会自动解除。")}</p>})}
                {w.problem.map(|s|view!{<p class="stock-problem" role="status">{s}</p>})}
                <dl class="stock-plan-evidence">
                    <div><dt>"原提现编号"</dt><dd>{w.client_id}</dd></div>
                    <div><dt>"交易所状态"</dt><dd>{w.remote.as_ref().map(|r|r.status.clone()).unwrap_or("回复未确认".into())}</dd></div>
                    <div><dt>"交易所回报费用 / 币种待核"</dt><dd>{w.remote.as_ref().and_then(|r|r.fee.clone()).unwrap_or("未知".into())}</dd></div>
                    <div><dt>"扣账核对"</dt><dd>"尚未核清 · 未释放占用"</dd></div>
                    {w.receipt.map(|r|view!{
                        <div><dt>"链上实际到账 / 原始单位"</dt><dd>{r.credited_raw}</dd></div>
                        <div><dt>"网络费 / 付费地址"</dt><dd>{format!("{} lamports · {}",r.network_fee_lamports,r.fee_payer)}</dd></div>
                        <div><dt>"链上最终确认区块"</dt><dd>{r.slot.to_string()}</dd></div>
                        <div><dt>"链上交易"</dt><dd>{r.transaction_hash}</dd></div>
                    })}
                </dl>
                <button type="button" class="row-action" disabled=move ||data.preflight.pending.get() ||data.clock.get()<next
                    on:click=move |_|data.preflight.funding_recheck.run(read.clone())>{if received{"重新核对原提现"}else{"查询原提现与到账"}}</button>
            </div>}
        })}
    }
}
