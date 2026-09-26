use super::*;

pub(super) fn panel(data: StockData) -> impl IntoView {
    let eligible = Memo::new(move |_| {
        data.market.with(|m| {
            m.value()
                .and_then(|s| s.peer.as_ref())
                .is_some_and(|p| p.share_unit_verified)
        })
    });
    let report = Memo::new(move |_| {
        data.market
            .with(|m| m.value().and_then(|s| s.peer_preflight.as_ref().filter(|r|
                s.security.as_ref().is_some_and(|a|a.asset==r.asset)
                    &&s.peer.as_ref().is_some_and(|p|p.selection==r.selection)).cloned()))
    });
    let rows = Memo::new(move |_| {
        data.market.with(|m| {
            m.value()
                .map(|s| {
                    let mut snapshot = s.clone();
                    if let Some(report) = snapshot.peer_preflight.as_mut() {
                        if report
                            .wallet
                            .as_ref()
                            .is_some_and(|w| w.owner != data.preflight.wallet.get().trim())
                        {
                            report.wallet = None;
                            report
                                .problems
                                .push("钱包地址已修改，请重新检查库存".into());
                        }
                    }
                    evaluate_peer_preflight(&snapshot, data.clock.get())
                })
                .unwrap_or_default()
        })
    });
    view! {<div class="stock-peer-preflight" hidden=move ||!eligible.get()>
        <h4>"所选交易所 · 账户与费用"</h4>
        <form class="stock-preflight-form" on:submit=move |ev|{ev.prevent_default();data.peers.preflight.run(Some(data.preflight.wallet.get_untracked()));}>
            <label>"Solana 钱包"<input type="text" autocomplete="off" aria-label="股票对比交易检查钱包" placeholder="钱包地址（可选）"
                prop:value=move ||data.preflight.wallet.get() value=move ||data.preflight.wallet.get()
                on:input=move |ev|data.preflight.wallet.set(event_target_value(&ev))/></label>
            <button type="submit" class="row-action" disabled=move ||data.peers.checking.get() ||data.peers.pending.get() ||data.peers.funding_checking.get() ||data.peers.order_checking.get()>
                {move ||if data.peers.checking.get(){"检查中…"}else{"检查账户与费用"}}</button>
        </form>
        <p class="stock-rfq-note">{move ||report.get().map(|r|format!("{} · {} · {}s 前检查 · 只读，未下单",r.selection.venue,r.selection.native_symbol,data.clock.get().saturating_sub(r.checked_at_ms).max(0)/1000))
            .unwrap_or_else(||"尚未读取所选交易所账户".into())}</p>
        {move ||report.get().filter(|r|data.clock.get()<r.checked_at_ms ||data.clock.get()-r.checked_at_ms>15_000).map(|_|view!{<p class="stock-rfq-note" role="status">"历史账户快照 · 请重新检查"</p>})}
        {move ||report.get().and_then(|r|r.account).map(|a|view!{<dl class="stock-direction-values">
            <div><dt>"股票 taker 费率 / %"</dt><dd>{comparison::quantity(a.stock_taker_pct)}</dd></div>
            <div><dt>"换汇 taker 费率 / %"</dt><dd>{comparison::quantity(a.fx_taker_pct)}</dd></div>
            <div><dt>{format!("账户可用 {}（不自动兑换）",a.quote_asset)}</dt><dd>{comparison::quantity(a.quote_available)}</dd></div>
        </dl>})}
        {move ||report.get().map(|_|view!{<div class="stock-directions">{rows.get().into_iter().map(|r|view!{<section class="stock-direction">
            <header><h4>{if r.chain_buy {"链买 / 所选交易所卖"}else{"所选交易所买 / 链卖"}}</h4><span>"费用与库存交易检查"</span></header>
            <dl class="stock-direction-values">
                <div><dt>"股票与换汇费用 / USDC"</dt><dd>{comparison::quantity(r.trading_cost_usdc)}</dd></div>
                <div><dt>"SOL 补回成本 / USDC"</dt><dd>{comparison::quantity(r.native_cost_usdc)}</dd></div>
                <div><dt>"已知费用后差额 / USDC"</dt><dd>{comparison::quantity(r.after_known_costs_usdc)}</dd></div>
            </dl>
            <div class="stock-peer-inventory">{r.inventory.into_iter().map(|i|view!{<div>
                <strong title=i.asset.clone()>{format!("{} · {}",i.location,if i.asset.chars().count()>24 {format!("{}…{}",i.asset.chars().take(6).collect::<String>(),i.asset.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect::<String>())}else{i.asset.clone()})}</strong>
                <span>"需 "{comparison::quantity(i.required)}</span><span>"可用 "{comparison::quantity(i.available)}</span>
                <span>{match i.sufficient {Some(true)=>"足够",Some(false)=>"不足",None=>"待核实"}}</span>
            </div>}).collect_view()}</div>
            <ul>{r.blockers.into_iter().map(|b|view!{<li>{b}</li>}).collect_view()}</ul>
        </section>}).collect_view()}</div>})}
    </div>}
}
