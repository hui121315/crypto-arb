use super::*;

pub(super) fn panel<V: IntoView + 'static>(data: StockData, credentials: V) -> impl IntoView {
    let draft = data.rfq;
    let rows = Memo::new(move |_| {
        data.market
            .with(|m| m.value().map(|s| s.rfqs.clone()).unwrap_or_default())
    });
    let enabled = Memo::new(move |_| {
        data.market.with(|m| {
            m.value().is_some_and(|s| {
                s.trading_route.as_ref().is_some_and(|r| {
                    r.kind == StockRouteKind::Rfq && r.valid_until_ms > data.clock.get()
                })
            })
        })
    });
    let has_pending = Memo::new(move |_| {
        let side = draft.side.get();
        let asset = data.market.with(|m| {
            m.value()
                .and_then(|s| s.security.as_ref())
                .map(|s| s.asset.clone())
        });
        rows.with(|rows| {
            rows.iter().any(|r| {
                r.unresolved() && r.request.side == side && Some(&r.request.asset) == asset.as_ref()
            })
        })
    });
    let quantity_problem = Memo::new(move |_| data.market.with(|m| {
        let s = m.value()?;
        validate_rfq_quantity(&StockRfqRequest {
            request_id: String::new(), asset: s.security.as_ref()?.asset.clone(),
            side: draft.side.get(), quantity: draft.quantity.get(),
        }, s, data.clock.get()).err()
    }));
    view! {<section class="stock-section stock-rfq" aria-label="Backpack 股票 询价">
        <header><h3>"Backpack 询价"</h3><span>{move ||data.market.with(|m|if m.value().is_some_and(|s|s.rfq_connected){"账户已连接"}else if rows.with(|rows|rows.iter().all(|r|!r.needs_follow_up())){"空闲 · 账户未连接"}else{"账户等待连接"})}</span></header>
        {credentials}
        <div class="stock-rfq-form">
            <label><span>"交易所方向"</span><select prop:value=move ||if draft.side.get()==StockRfqSide::Ask{"Ask"}else{"Bid"}
                disabled=move ||draft.pending.get() || draft.attempt.with(Option::is_some) on:change=move |ev|draft.side.set(if event_target_value(&ev)=="Ask"{StockRfqSide::Ask}else{StockRfqSide::Bid})>
                <option value="Ask">"Backpack 卖出"</option><option value="Bid">"Backpack 买入"</option></select></label>
            <label><span>"询价股数"</span><input type="text" inputmode="decimal" autocomplete="off" placeholder="0.00"
                value=move ||draft.quantity.get() prop:value=move ||draft.quantity.get() disabled=move ||draft.pending.get() || draft.attempt.with(Option::is_some) on:input=move |ev|draft.quantity.set(event_target_value(&ev))/></label>
            <button type="button" class="workbench-primary" disabled=move ||draft.pending.get() || draft.attempt.with(Option::is_some) || !enabled.get() || has_pending.get() || quantity_problem.with(Option::is_some)
                on:click=move |_|draft.submit.run(())>{move ||if draft.pending.get(){"等待处理结果…"}else{"发送询价（不成交）"}}</button>
        </div>
        {move ||(!draft.quantity.with(String::is_empty) && enabled.get() && !draft.attempt.with(Option::is_some))
            .then(||quantity_problem.get()).flatten().map(|p|view!{<p class="stock-rfq-note" role="status">{p}</p>})}
        <p class="stock-rfq-note">{move ||if has_pending.get(){"该方向有未结询价，请先核对或取消。"}else if !enabled.get(){"当前未处于已核实的 询价 交易时段。"}else{"等待接受模式 · 不自动接受报价 · 不借贷或下单"}}</p>
        {move ||draft.attempt.get().map(|request|view!{
            <div class="stock-rfq-attempt"><span>{format!("上次请求尚未取得处理结果 · {} · {} {} 股",request.asset,if request.side==StockRfqSide::Ask{"卖"}else{"买"},request.quantity)}</span>
                {data.market.with(|m|m.value().and_then(|s|s.security.as_ref()).is_none_or(|s|s.asset!=request.asset))
                    .then(||{let asset=request.asset.clone();view!{<button type="button" class="row-action" disabled=move ||data.pending.get() || draft.pending.get()
                        on:click=move |_|data.watch.run(Some(asset.clone()))>"返回原股票"</button>}})}
                <button type="button" class="row-action" disabled=move ||draft.pending.get()
                    on:click=move |_|draft.action.run((request.request_id.clone(),false))>"核对原请求"</button>
                <button type="button" class="row-action" disabled=move ||draft.pending.get()
                    on:click=move |_|draft.submit.run(())>"重试原询价"</button>
                <button type="button" class="row-action" disabled=move ||draft.pending.get()
                    on:click=move |_|draft.finish_unsent.run(())>"结束未发送请求"</button></div>
        })}
        {move ||data.market.with(|m|m.value().and_then(|s|s.rfq_problem.clone())).map(|p|view!{<p role="status" class="stock-problem">{p}</p>})}
        {move ||rows.with(Vec::is_empty).then(||view!{<p class="stock-empty-inline">"暂无股票询价记录"</p>})}
        <div class="stock-rfq-records"><For each=move ||rows.get() key=|r|r.request.request_id.clone() children=move |record|{
            let id=record.request.request_id.clone();
            let current=Memo::new(move |_|rows.with(|rows|rows.iter().find(|r|r.request.request_id==id).cloned()).unwrap_or_else(||record.clone()));
            view!{<article class="stock-rfq-record">
                <div class="stock-rfq-record-top"><strong>{move ||current.with(|r|format!("{} · {} {} 股",r.request.asset,if r.request.side==StockRfqSide::Ask{"卖"}else{"买"},r.request.quantity))}</strong>
                    <span class="read-only-flag" data-pending=move ||current.with(|r|r.settlement_pending()).to_string()>{move ||current.with(|r|phase_label(r,data.clock.get()))}</span></div>
                <dl class="stock-summary"><div><dt>"询价 taker 价 / USDC"</dt><dd>{move ||current.with(|r|{
                    let live=data.market.with(|m|m.problem().is_none() && m.value().is_some_and(|s|s.rfq_connected && s.rfq_problem.is_none()));
                    r.current_candidate(live,data.clock.get()).map(|c|c.taker_price.clone()).unwrap_or_else(||"—".into())
                })}</dd></div><div><dt>"报价有效期"</dt><dd>{move ||current.with(|r|window_label(r,data.clock.get()))}</dd></div>
                    <div><dt>"实际成交 / 股"</dt><dd>{move ||current.with(|r|r.executed_quantity.clone().unwrap_or_else(||if r.phase==StockRfqPhase::Filled{"待核对"}else{"—"}.into()))}</dd></div>
                    <div><dt>"成交金额 / USDC"</dt><dd>{move ||current.with(|r|r.executed_quote_quantity.clone().unwrap_or_else(||"—".into()))}</dd></div></dl>
                {move ||current.with(|r|r.problem.clone()).map(|p|view!{<p class="stock-rfq-note">{p}</p>})}
                {move ||current.with(next_step_label).map(|text|view!{<p class="stock-rfq-note">{text}</p>})}
                <div class="stock-rfq-record-actions"><details><summary>"请求凭据"</summary><code>{move ||current.with(|r|if r.phase==StockRfqPhase::NotSent && r.client_id==0 {format!("{} · 本地未发送",r.request.request_id)}else{format!("{} · RFQ {} · client {}",r.request.request_id,r.rfq_id.as_deref().unwrap_or("待确认"),r.client_id)})}</code></details>
                    <button type="button" class="row-action" disabled=move ||draft.pending.get() on:click=move |_|draft.action.run((current.with(|r|r.request.request_id.clone()),false))>"核对原询价"</button>
                    <button type="button" class="row-action" disabled=move ||draft.pending.get() ||current.with(|r|r.phase.terminal() || r.phase==StockRfqPhase::AcceptedBinding || r.cancel_requested || r.acceptance.is_some())
                        on:click=move |_|draft.action.run((current.with(|r|r.request.request_id.clone()),true))>"取消询价"</button>
                </div>
            </article>}
        }/></div>
    </section>}
}

pub(super) fn phase_label(r: &StockRfq, now: i64) -> &'static str {
    if let Some(a) = &r.acceptance {
        if a.evidence_conflict {
            return "处理结果冲突 · 停止自动处理";
        }
        if a.rejected {
            return "接受报价被拒绝";
        }
        if r.phase == StockRfqPhase::AwaitingQuotes {
            return if a.acknowledged {
                "接受已处理结果 · 结算待确认"
            } else {
                "接受结果待确认 · 不重发"
            };
        }
    }
    match r.phase {
        StockRfqPhase::SubmissionUnknown => "提交待核对",
        StockRfqPhase::NotSent => "询价未发送",
        StockRfqPhase::Rejected => "询价被拒绝",
        StockRfqPhase::AwaitingQuotes => "等待报价",
        StockRfqPhase::Candidate if r.needs_recheck => "报价待核对",
        StockRfqPhase::Candidate if r.expiry_time_ms.is_some_and(|t| t <= now) => "报价已过期",
        StockRfqPhase::Candidate if r.submission_time_ms.is_some_and(|t| t > now) => "报价待开放",
        StockRfqPhase::Candidate => "已收到报价",
        StockRfqPhase::AcceptedBinding => "已锁资 · 待结算",
        StockRfqPhase::Filled if r.settlement_pending() && r.settlement.paused => {
            "已成交 · 核对已暂停"
        }
        StockRfqPhase::Filled if r.settlement_pending() => "已成交 · 金额待核",
        StockRfqPhase::Filled => "成交额已核 · 费用待核",
        StockRfqPhase::Cancelled if r.acceptance.is_some() => "结算已取消 · 另一腿待核",
        StockRfqPhase::Expired if r.acceptance.is_some() => "结算已过期 · 另一腿待核",
        StockRfqPhase::Cancelled => "已取消",
        StockRfqPhase::Expired => "已过期",
    }
}

pub(super) fn next_step_label(r: &StockRfq) -> Option<&'static str> {
    if let Some(a) = &r.acceptance {
        if a.evidence_conflict {
            return Some(
                "处理结果不一致：保留原交易与资金占用。核对原请求，不重复提交，也不计为套利收益。",
            );
        }
        if a.rejected {
            return Some("接受报价未成功：先核对链上腿是否已经成交，再处理剩余库存。");
        }
        if matches!(r.phase, StockRfqPhase::Cancelled | StockRfqPhase::Expired) {
            return Some("Backpack 结算未完成，不代表双边已撤销。先核对链上腿与实际余额，不能直接释放整个计划。");
        }
    }
    match r.phase {
        StockRfqPhase::AcceptedBinding => {
            Some("Backpack 已锁资，等待实际结算。该报价不能取消，也不能再次接受。")
        }
        StockRfqPhase::Filled if r.settlement_pending() && r.settlement.paused => {
            Some("成交明细尚未齐全，自动补查已暂停；可手动核对原询价，不重发。")
        }
        StockRfqPhase::Filled if r.settlement_pending() => {
            Some("正在核对实际成交股数和金额。收到成交事件不等于净到账已确认。")
        }
        StockRfqPhase::Filled => {
            Some("成交金额不等于扣费后到账：实际费用与净到账仍待核实，尚不能确认净利润。")
        }
        _ => None,
    }
}

fn window_label(r: &StockRfq, now: i64) -> String {
    if r.phase.terminal() {
        return "已结束".into();
    }
    if r.phase == StockRfqPhase::AcceptedBinding {
        return "等待结算".into();
    }
    if r.acceptance.is_some() {
        return "已提交 · 核对原请求".into();
    }
    if let Some(start) = r.submission_time_ms.filter(|t| *t > now) {
        return format!("{}s 后开放", (start - now + 999) / 1000);
    }
    match r.expiry_time_ms {
        Some(t) if t > now => format!("剩余 {}s", (t - now + 999) / 1000),
        Some(_) => "已过期".into(),
        None => "待核对".into(),
    }
}
