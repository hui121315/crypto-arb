use super::*;

pub(super) fn panel(report: StockPreflight, data: StockData) -> impl IntoView {
    let expected = report.clone();
    let current = Memo::new(move |_| {
        data.clock.get() >= expected.checked_at_ms
            && data.clock.get() < expected.valid_until_ms
            && data.preflight.wallet.get().trim()
                == expected.wallet_address.as_deref().unwrap_or("")
            && data.market.with(|m| {
                m.value().is_some_and(|s| {
                    s.security
                        .as_ref()
                        .is_some_and(|a| a.asset == expected.asset)
                })
            })
    });
    let source = report.source_plan.clone();
    let no_gap = report.funding.iter().all(|r| r.needs.is_empty());
    view! {<div class="stock-order-receipt" aria-label="下一笔库存复查">
        <strong>"下一笔库存 · 原计划规模"</strong>
        <p class="stock-rfq-note" role="status">{move ||if current.get(){"当前余额复查 · 不是下单许可"}else{"库存复查已失效 · 请重新读取"}}</p>
        <p class="stock-rfq-note">"交易收尾不代表库存已归位。这里只补原规模所需库存，新的行情、费用与收益须重新构建。"</p>
        <div class="stock-inventory-list">{report.directions.into_iter().flat_map(|r|r.inventory).map(|i|view!{<div class="stock-inventory-row">
            <strong>{format!("{} · {}",i.location,i.asset)}</strong>
            <span class="stock-inventory-status" data-status=match i.sufficient{Some(true)=>"enough",Some(false)=>"short",None=>"unknown"}>
                {match i.sufficient{Some(true)=>"数量足够",Some(false)=>"余额不足",None=>"待核实"}}</span>
            <span>"原规模需要"<br/>{i.required.unwrap_or_else(||"未知".into())}</span>
            <span>"当前可用"<br/>{i.available.unwrap_or_else(||"未知".into())}</span>
        </div>}).collect_view()}</div>
        {no_gap.then(||view!{<p class="stock-rfq-note">"本次快照无补库缺口 · 不创建转账"</p>})}
        {report.funding.into_iter().filter(|r|!r.needs.is_empty()).map(|r|super::funding::needs(r,data,report.asset.clone(),report.checked_at_ms,source.clone())).collect_view()}
        {source.map(|s|view!{<details><summary>"来源交易"</summary><dl class="stock-plan-evidence"><div><dt>"已归档计划"</dt><dd>{s.plan_id}</dd></div><div><dt>"版本"</dt><dd>{s.revision}</dd></div></dl></details>})}
    </div>}
}
