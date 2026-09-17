use super::*;

pub(super) fn panel(data: StockData) -> impl IntoView {
    let eligible = Memo::new(move |_| {
        data.market.with(|m| {
            m.value().and_then(|s| s.peer.as_ref()).is_some_and(|p| {
                p.selection.venue == "kraken"
                    && p.selection.product == StockPeerProduct::Spot
                    && p.share_unit_verified
            })
        })
    });
    view! {<div class="stock-peer-order" hidden=move ||!eligible.get()>
        <h4>"股票订单验证"</h4>
        <p class="stock-rfq-note">"仅验证所选交易所股票订单，不成交、不换汇。链上交易与双边执行尚未接通。"</p>
        <div class="stock-directions">{[StockChainDirection::Buy,StockChainDirection::Sell].into_iter().map(|direction|direction_row(data,direction)).collect_view()}</div>
    </div>}
}

fn direction_row(data: StockData, direction: StockChainDirection) -> impl IntoView {
    let draft = Memo::new(move |_| {
        data.market.with(|m| {
            let s = m.value().ok_or_else(|| "尚未读取股票行情".to_string())?;
            let asset = s.security.as_ref().ok_or("请先选择股票")?.asset.clone();
            let selection = s.peer.as_ref().ok_or("请选择对比市场")?.selection.clone();
            prepare_peer_order_check(
                s,
                StockPeerOrderCheckRequest {
                    asset,
                    selection,
                    direction,
                },
                data.clock.get(),
            )
        })
    });
    let report = Memo::new(move |_| {
        data.market.with(|m| {
            m.value().and_then(|s| {
                s.peer_order_checks
                    .iter()
                    .find(|r| {
                        r.draft.request.direction == direction
                            && s.security
                                .as_ref()
                                .is_some_and(|a| a.asset == r.draft.request.asset)
                            && s.peer
                                .as_ref()
                                .is_some_and(|p| p.selection == r.draft.request.selection)
                    })
                    .cloned()
            })
        })
    });
    let cooldown = Memo::new(move |_| {
        data.market.with(|m| {
            m.value()
                .map(|s| {
                    s.peer_order_checks
                        .iter()
                        .map(|r| {
                            let age = data.clock.get().saturating_sub(r.draft.prepared_at_ms);
                            if age < 0 {
                                5
                            } else {
                                ((5000 - age).max(0) + 999) / 1000
                            }
                        })
                        .max()
                        .unwrap_or(0)
                })
                .unwrap_or(0)
        })
    });
    view! {<section class="stock-direction">
        <header><h4>{if direction==StockChainDirection::Buy{"链买 / 交易所卖"}else{"交易所买 / 链卖"}}</h4><span>"不成交验证"</span></header>
        {move ||draft.get().ok().map(|d|view!{<dl class="stock-direction-values"><div><dt>"股票数量 / 股"</dt><dd>{d.quantity}</dd></div>
            <div><dt>{format!("限价 / {}",d.quote_asset)}</dt><dd>{d.limit_price}</dd></div></dl>})}
        {move ||draft.get().err().map(|e|view!{<p class="stock-rfq-note">{e}</p>})}
        <button type="button" class="row-action" disabled=move ||draft.get().is_err() ||cooldown.get().is_positive() ||data.peers.order_checking.get() ||data.peers.checking.get() ||data.peers.funding_checking.get() ||data.peers.pending.get()
            on:click=move |_|data.peers.order_check.run(direction)>
            {move ||if data.peers.order_checking.get(){"等待验证回复…".into()}else if cooldown.get()>0{format!("{}s 后可验证",cooldown.get())}else if direction==StockChainDirection::Buy{"验证股票卖单".into()}else{"验证股票买单".into()}}
        </button>
        {move ||report.get().map(|r|{
            let age=data.clock.get().saturating_sub(r.draft.prepared_at_ms);
            let label=if r.completed_at_ms.is_none(){if (0..=12_000).contains(&age){"等待回复"}else{"未取得回复"}}else{match r.status {
                StockPeerOrderCheckStatus::Passed=>"参数验证通过",StockPeerOrderCheckStatus::Rejected=>"验证拒绝",StockPeerOrderCheckStatus::Unknown=>"未确认通过"}};
            let message=if r.completed_at_ms.is_none() && !(0..=12_000).contains(&age){"未收到完整验证回复，不认定通过；可以重新验证".into()}else{r.message};
            view!{<details class="stock-peer-order-result"><summary>{format!("最近一次 · {label}")}</summary>
                <p>{message}</p><dl class="stock-plan-evidence">
                    <div><dt>"原生市场"</dt><dd>{r.draft.request.selection.native_symbol}</dd></div>
                    <div><dt>"本次股数 / 限价"</dt><dd>{format!("{} 股 / {} {}",r.draft.quantity,r.draft.limit_price,r.draft.quote_asset)}</dd></div>
                    <div><dt>"请求时间"</dt><dd>{if age>=0{format!("{}s 前",age/1000)}else{"时钟待核实".into()}}</dd></div>
                </dl>
            </details>}
        })}
    </section>}
}
