use super::*;

pub(super) fn selected_fee(data: StockData) -> Option<String> {
    let ids = data.preflight.conversion_cost_ids.get();
    data.market.with(|m| {
        m.value()?
            .selected_conversion_fee_usdc(&ids)
            .ok()
            .map(|n| n.normalize().to_string())
    })
}

pub(super) fn selector(data: StockData) -> impl IntoView {
    let selected = data.preflight.conversion_cost_ids;
    let sources = Memo::new(move |_| {
        data.market.with(|m| {
            m.value()
                .map(|s| {
                    s.exchange_conversions
                        .iter()
                        .filter_map(|p| {
                            p.confirmed_fee_usdc().ok().map(|fee| {
                                (
                                    p.clone(),
                                    fee.normalize().to_string(),
                                    s.claimed_conversion_cost_ids.contains(&p.plan_id),
                                )
                            })
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        })
    });
    view! {<details class="stock-conversion-costs">
        <summary>"前期换币手续费"<span>{move || if selected.with(Vec::is_empty) {" · 未归集".into()}
            else {selected_fee(data).map(|n|format!(" · {n} USDC")).unwrap_or(" · 已选费用待核对".into())}}</span></summary>
        <p class="stock-rfq-note">"仅归入所选记录的已付交易费。换币价差、未选费用和充提费不在此合计。"</p>
        {move || sources.with(Vec::is_empty).then(||view!{<p class="stock-rfq-note">"暂无已核清的兑换费用"</p>})}
        <For each=move ||sources.get() key=|(p,_,used)|(p.plan_id.clone(),p.revision,*used) children=move |(p,fee,used)| {
            let id=p.plan_id.clone();
            let checked_id=id.clone();
            let change_id=id.clone();
            let disabled_id=id.clone();
            let order=p.order.as_ref().and_then(|o|o.order_id.clone()).unwrap_or_default();
            let label=format!("{} USDT · {} USDC 手续费{}",p.request.input_usdt,fee,if used {" · 已归入其他计划"}else{""});
            view!{<div class="stock-monitor-control"><label title=format!("原兑换订单 {order}")>
                <input type="checkbox" aria-label=label.clone()
                    prop:checked=move ||selected.with(|v|v.contains(&checked_id))
                    disabled=move ||used || data.preflight.pending.get() || data.market.with(|m|m.value().is_none_or(|s|s.exchange_conversion_problem.is_some()))
                        || selected.with(|v|v.len()>=STOCK_CONVERSION_COST_LIMIT && !v.contains(&disabled_id))
                    on:change=move |ev| {
                        let checked=event_target_checked(&ev);
                        selected.update(|ids| {
                            ids.retain(|id|id!=&change_id);
                            if checked && !used && ids.len()<STOCK_CONVERSION_COST_LIMIT {ids.push(change_id.clone());ids.sort();}
                        });
                    }/>
                <span>{label}</span>
            </label></div>}
        }/>
    </details>}
}
