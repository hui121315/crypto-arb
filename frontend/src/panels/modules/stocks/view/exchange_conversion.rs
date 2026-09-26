use super::super::data::preflight::exchange_conversion::{ConversionAction, ConversionData};
use super::*;

pub(super) fn panel(data: StockData) -> impl IntoView {
    let d = data.preflight.conversion;
    let plans = Memo::new(move |_| {
        data.market.with(|m| {
            m.value()
                .map(|s| s.exchange_conversions.clone())
                .unwrap_or_default()
        })
    });
    let problem = Memo::new(move |_| {
        data.market.with(|m| {
            m.value()
                .and_then(|s| s.exchange_conversion_problem.clone())
        })
    });
    let busy = Memo::new(move |_| data.pending.get() || data.preflight.pending.get());
    let blocked = Memo::new(move |_| {
        let now = data.clock.get();
        problem.get().is_some()
            || plans.get().iter().any(|p| p.holds_funds(now))
            || data.market.with(|m| {
                m.value().is_none_or(|s| {
                    s.plan_problem.is_some()
                        || s.funding_problem.is_some()
                        || s.plans.iter().any(|p| p.holds_funds(now))
                        || s.funding_plans
                            .iter()
                            .any(|p| p.phase_at(now).holds_funds())
                })
            })
    });
    let has_security = Memo::new(move |_| {
        data.market
            .with(|m| m.value().is_some_and(|s| s.security.is_some()))
    });
    let estimate = Memo::new(move |_| {
        d.sizing.get().filter(|s| {
            has_security.get() &&s.input_usdt == d.input.get().trim() && s.request.minimum_usdc == d.minimum.get().trim()
        })
    });
    let estimate_stale = Memo::new(move |_| {
        estimate.with(|s| {
            s.as_ref().is_some_and(|s| {
                data.clock.get() < s.checked_at_ms || data.clock.get() >= s.valid_until_ms
            })
        })
    });
    view! {<section class="stock-section stock-stablecoin" id="stock-account-conversion" aria-label="Backpack 账户兑换">
        <header><h3>"Backpack 账户兑换"</h3><span>"USDT → USDC · 账户内"</span></header>
        <form class="stock-stablecoin-form" hidden=move ||!has_security.get() on:submit=move |e|{e.prevent_default();d.run.run(ConversionAction::Build);}>
            <label><span>"投入 / USDT"</span><input type="text" inputmode="decimal" autocomplete="off" aria-label="Backpack 兑换投入 USDT" placeholder="输入数量"
                prop:value=move ||d.input.get() on:input=move |e|d.input.set(event_target_value(&e))/></label>
            <label><span>"最低到账 / USDC"</span><input type="text" inputmode="decimal" autocomplete="off" aria-label="Backpack 最低到账 USDC" placeholder="扣除交易费后"
                prop:value=move ||d.minimum.get() on:input=move |e|d.minimum.set(event_target_value(&e))/></label>
            <div class="stock-conversion-form-actions">
                <button class="row-action" type="button" title="按当前 WS 买价、账户费率和数量步长反算；不预留或下单"
                    disabled=move ||busy.get() ||!has_security.get() ||d.minimum.get().trim().is_empty()
                    on:click=move |_|d.size.run(())>"计算所需 USDT"</button>
                <button class="row-action" type="submit" disabled=move ||busy.get() ||blocked.get() ||!has_security.get() ||estimate_stale.get() ||d.input.get().trim().is_empty() ||d.minimum.get().trim().is_empty()>"生成兑换计划"</button>
            </div>
        </form>
        {move ||estimate.get().map(|s|view!{<div class="stock-conversion-estimate" role="status" aria-label="账户兑换试算">
            <p class="stock-rfq-note">{move ||if estimate_stale.get(){"试算已过期 · 需重新计算"}else{"只读试算 · 尚未预留"}}</p>
            <dl class="stock-direction-values">
                <div><dt>"所需投入 / USDT"</dt><dd>{s.input_usdt}</dd></div>
                <div><dt>"当前费后 / USDC"</dt><dd>{s.minimum_net_usdc}</dd></div>
                <div><dt>"预估费用 / USDC"</dt><dd>{s.fee_budget_usdc}</dd></div>
                <div><dt>"数量步长 / USDT"</dt><dd>{s.step_size}</dd></div>
            </dl>
        </div>})}
        {move ||(!has_security.get()).then(||view!{<p class="stock-rfq-note">"选择股票后接入账户兑换行情"</p>})}
        {move ||d.problem.get().map(|p|view!{<p class="stock-rfq-note" role="alert">{p}</p>})}
        {move ||problem.get().map(|p|view!{<p class="stock-rfq-note" role="alert">{p}</p>})}
        <For each=move ||plans.get() key=|p|(p.plan_id.clone(),p.revision) children=move |p|row(p,d,busy,data.clock)/>
        {move ||plans.with(|rows|rows.iter().any(|p|p.accounting().is_ok()
            && p.order.as_ref().is_some_and(|o|o.phase==StockCexOrderPhase::Filled)))
            .then(||super::funding::refresh_inventory(data))}
    </section>}
}

fn row(
    p: StockExchangeConversionPlan,
    d: ConversionData,
    busy: Memo<bool>,
    clock: RwSignal<i64>,
) -> impl IntoView {
    let frozen = p.clone();
    let ready = Memo::new(move |_| frozen.can_submit(clock.get()));
    // A queued button update can outlive the keyed row when cancellation replaces its revision.
    let confirmed = ArcRwSignal::new(false);
    let submit = StockStablecoinSubmitRequest {
        plan_id: p.plan_id.clone(),
        revision: p.revision,
        confirm_live: true,
    };
    let cancel = StockPlanRevisionRequest {
        plan_id: p.plan_id.clone(),
        revision: p.revision,
    };
    let recheck = StockPlanCancelRequest {
        plan_id: p.plan_id.clone(),
    };
    let pending = p.order.as_ref().is_some_and(|o| !o.receipt_complete());
    let next = p
        .order
        .as_ref()
        .and_then(|o| o.recheck.next_at_ms)
        .unwrap_or(0);
    let state = if p.cancelled_at_ms.is_some() {
        "已取消 · 未兑换"
    } else if p.order.is_some() {
        if p.accounting().is_err()
            && p.order
                .as_ref()
                .is_some_and(StockCexOrder::receipt_complete)
        {
            "收支已核对 · 未满足原计划"
        } else if p.accounting().is_err() {
            "已提交 · 收支待核对"
        } else if p
            .order
            .as_ref()
            .is_some_and(|o| o.phase == StockCexOrderPhase::Filled)
        {
            "兑换已完成 · 实际费用已核对"
        } else {
            "未成交 · 已释放预留"
        }
    } else {
        "已预留 · 未兑换"
    };
    let no_order = p.order.is_none() && p.cancelled_at_ms.is_none();
    // A target or fee-budget violation must not hide already reconciled cash movements.
    let actual = p
        .order
        .as_ref()
        .and_then(|o| o.net_asset_changes(&p.terms.instruction));
    let other_assets = actual
        .as_ref()
        .map(|rows| {
            rows.iter()
                .filter(|(asset, quantity)| {
                    !matches!(asset.as_str(), "USDT" | "USDC") && quantity.as_str() != "0"
                })
                .map(|(asset, quantity)| (asset.clone(), quantity.clone()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let unresolved = p.order.as_ref().filter(|o| !o.receipt_complete()).cloned();
    let fees = p.order.as_ref().map(|o| {
        o.fills
            .iter()
            .filter_map(|f| {
                f.fee
                    .as_ref()
                    .map(|f| format!("{} {}", f.quantity, f.asset))
            })
            .collect::<Vec<_>>()
            .join(" + ")
    });
    let review = p
        .order
        .as_ref()
        .filter(|o| o.receipt_complete())
        .and_then(|_| p.accounting().err());
    view! {<section class="stock-stablecoin-plan" aria-label="已保存 Backpack 兑换">
        <header><strong>{format!("{} USDT → 至少 {} USDC",p.request.input_usdt,p.request.minimum_usdc)}</strong>
            <span>{move ||if no_order &&!ready.get(){"报价已过期 · 未兑换"}else{state}}</span></header>
        <dl class="stock-direction-values">
            <div><dt>"原限价 / USDC"</dt><dd>{p.terms.book.bid.clone().unwrap_or_default()}</dd></div>
            <div><dt>"原报价费后 / USDC"</dt><dd>{p.terms.minimum_net_usdc.clone()}</dd></div>
            <div><dt>"预估交易费 / USDC"</dt><dd>{p.terms.fee_budget_usdc.clone()}</dd></div>
        </dl>
        {actual.map(|a|view!{<div class="stock-stablecoin-receipt" aria-label="账户兑换实际收支">
            <dl class="stock-direction-values">
                <div><dt>"实际 USDT 变化"</dt><dd>{a.get("USDT").cloned().unwrap_or_default()}</dd></div>
                <div><dt>"实际净入账 / USDC"</dt><dd>{a.get("USDC").cloned().unwrap_or_default()}</dd></div>
                <div><dt>"实际交易费用"</dt><dd>{fees.clone().filter(|s|!s.is_empty()).unwrap_or_else(||"无成交".into())}</dd></div>
                {other_assets.into_iter().map(|(asset,quantity)|view!{<div><dt>{format!("实际 {asset} 变化")}</dt><dd>{quantity}</dd></div>}).collect_view()}
            </dl></div>})}
        {unresolved.map(|o|view!{<div class="stock-stablecoin-receipt" aria-label="账户兑换待核对收支">
            <dl class="stock-direction-values">
                <div><dt>"报告成交 / USDT"</dt><dd>{o.executed_quantity.unwrap_or_else(||"待核实".into())}</dd></div>
                <div><dt>"报告成交额 / USDC · 未扣费用"</dt><dd>{o.executed_quote_quantity.unwrap_or_else(||"待核实".into())}</dd></div>
            </dl><p class="stock-rfq-note">"成交、最终结果与费用尚未全部核齐，净入账未知，资金占用保留。"</p>
        </div>})}
        {review.as_ref().map(|_|view!{<p class="stock-rfq-note">"以上是已核实的原币变化，不是计划达标或套利利润；仍保留资金占用，不自动继续兑换或转账。"</p>})}
        {p.order.as_ref().and_then(|o|o.problem.clone()).map(|e|view!{<p class="stock-rfq-note" role="status">{e}</p>})}
        {review.map(|e|view!{<p class="stock-rfq-note" role="alert">{e}</p>})}
        <details><summary>"原订单与规格"</summary><p class="stock-rfq-note">{format!("{} · 版本 {} · 最小 {} USDT · 步长 {} USDT · FOK",p.plan_id,p.revision,p.terms.market.min_quantity,p.terms.market.step_size)}</p>
            {p.order.and_then(|o|o.order_id).map(|id|view!{<p class="stock-rfq-note">{format!("订单 {id}")}</p>})}</details>
        {move ||ready.get().then(||{let submit=submit.clone();let cancel=cancel.clone();
            let checked=confirmed.clone();let change=confirmed.clone();let accepted=confirmed.clone();
            view!{<div class="stock-stablecoin-actions">
            <label><input type="checkbox" aria-label="确认本次 Backpack 实盘兑换" prop:checked=move ||checked.get() on:change=move |e|change.set(event_target_checked(&e)) disabled=move ||busy.get()/><span>"确认本次账户实盘兑换"</span></label>
            <button class="row-action" type="button" disabled=move ||busy.get() ||!accepted.get() on:click=move |_|d.run.run(ConversionAction::Submit(submit.clone()))>"提交兑换"</button>
            <button class="row-action" type="button" disabled=move ||busy.get() on:click=move |_|d.run.run(ConversionAction::Cancel(cancel.clone()))>"取消预留"</button>
        </div>}})}
        {pending.then(||view!{<button class="row-action" type="button" disabled=move ||busy.get() ||clock.get()<next on:click=move |_|d.run.run(ConversionAction::Recheck(recheck.clone()))>"核对原订单"</button>})}
    </section>}
}

#[cfg(test)]
mod tests;
