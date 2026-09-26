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
            .with(|m| m.value().and_then(|s| s.peer_funding.as_ref().filter(|r|
                s.security.as_ref().is_some_and(|a|a.asset==r.asset)
                    &&s.peer.as_ref().is_some_and(|p|p.selection==r.selection)).cloned()))
    });
    let current = Memo::new(move |_| {
        data.market.with(|m| {
            m.value().is_some_and(|s| {
                s.peer_funding
                    .as_ref()
                    .is_some_and(|r| r.current(s, data.clock.get()))
            })
        })
    });
    view! {<div class="stock-peer-funding" hidden=move ||!eligible.get()>
        <header><h4>"所选交易所 · 充提"</h4><button type="button" class="row-action" disabled=move ||data.peers.funding_checking.get() ||data.peers.checking.get() ||data.peers.pending.get() ||data.peers.order_checking.get()
            on:click=move |_|data.peers.funding.run(())>{move ||if data.peers.funding_checking.get(){"检查中…"}else{"检查充提"}}</button></header>
        <p class="stock-rfq-note">{move ||match report.get(){None=>"尚未检查 · 不会自动转账".into(),Some(r)=>format!("{} · {} · {} · 充提使用未复权 Token 数量，不是订单簿股数",r.selection.venue,r.selection.native_symbol,if current.get(){"当前方法快照"}else{"历史资料，需重新检查"})}}</p>
        {move ||report.get().map(|r|r.routes.into_iter().map(|route|{
            let summary=if route.problem.is_some(){"读取未完成".into()}else if route.methods.is_empty(){"未返回可用方法".into()}else{format!("{} 个方法",route.methods.len())};
            view!{<details class="stock-peer-funding-route"><summary><strong>{format!("{} · {}",route.asset,route.direction.label())}</strong><span>{summary}</span></summary>
                {route.problem.clone().map(|p|view!{<p class="stock-rfq-note" role="status">{p}</p>})}
                {route.methods.iter().cloned().map(|method|{
                    let check_route=route.clone();let check_method=method.clone();
                    let status=move ||match data.market.with(|m|m.value().and_then(|s|peer_funding_contract_matches(s,&check_route,&check_method))) {
                        Some(true)=>"合约匹配 · 地址、账户额度与实时开关仍待核对",Some(false)=>"非当前链上合约或网络，不能直接充提",None=>"合约未核实，不能直接充提"};
                    view!{<section class="stock-peer-funding-method">
                        <h5>{method.network_name}</h5><p class="stock-rfq-note">{status}</p>
                        <dl class="stock-direction-values">
                            <div><dt>"最低 / Token"</dt><dd>{comparison::quantity(method.minimum_amount)}</dd></div>
                            <div><dt>"最高 / Token"</dt><dd>{comparison::quantity(method.maximum_amount)}</dd></div>
                            <div><dt title=method.fees.base.asset_class>{format!("基础费用 / {}",method.fees.base.asset)}</dt><dd>{comparison::quantity(Some(method.fees.base.amount))}</dd></div>
                            <div><dt>"比例费用 / %"</dt><dd>{comparison::quantity(method.fees.percentage)}</dd></div>
                        </dl>
                        <p class="stock-rfq-note">{if method.fees.included{"费用计入方法金额"}else{"费用另计"}}" · 不是本次精确费用报价"</p>
                        {method.fees.minimum.map(|a|view!{<p class="stock-rfq-note" title=a.asset_class>{format!("最低费用 {} {}",a.amount,a.asset)}</p>})}
                        {method.fees.maximum.map(|a|view!{<p class="stock-rfq-note" title=a.asset_class>{format!("最高费用 {} {}",a.amount,a.asset)}</p>})}
                        <dl class="stock-peer-funding-identity"><div><dt>"合约"</dt><dd>{method.contract_address.unwrap_or_else(||"未返回".into())}</dd></div>
                            <div><dt>"方法 ID"</dt><dd>{method.method_id}</dd></div><div><dt>"网络 ID"</dt><dd>{method.network_id}</dd></div></dl>
                    </section>}
                }).collect_view()}
                <p class="stock-rfq-note">"未创建充值地址、未校验提现白名单，未转移资金"</p>
            </details>}
        }).collect_view())}
    </div>}
}
