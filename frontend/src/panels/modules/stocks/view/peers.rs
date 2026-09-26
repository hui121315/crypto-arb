use super::*;
mod preflight;
mod funding;
mod order;

pub(super) fn panel(data: StockData) -> impl IntoView {
    let p = data.peers;
    let catalog = Signal::derive(move || p.current_catalog());
    let peer = Memo::new(move |_| data.market.with(|m| m.value().and_then(|s| s.peer.clone())));
    let identity = Memo::new(move |_| peer.with(|p| p.as_ref().map(|p| p.identity.clone())));
    let selected_native = Memo::new(move |_| {
        peer.with(|peer| {
            peer.as_ref()
                .filter(|s| {
                    s.selection.venue == p.venue.get() && s.selection.product == p.product.get()
                })
                .map(|s| s.selection.native_symbol.clone())
                .unwrap_or_default()
        })
    });
    let estimates = Memo::new(move |_| {
        let mut rows = data.market.with(|m| {
            m.value()
                .filter(|s| s.peer.as_ref().is_some_and(|p| p.share_unit_verified))
                .map(|s| evaluate_peer(s, data.clock.get()))
                .unwrap_or_default()
        });
        if let Some(problem) = data.quote_draft_problem() {
            for row in &mut rows {
                row.shares = None; row.gross_usdc = None; row.remainder_shares = None;
                row.cex_notional_usdc = None;
                row.blockers = vec![problem.into()];
            }
        }
        rows
    });
    view! {<section class="stock-section stock-peers" aria-label="其他交易所股票对比">
        <header><h3>"其他交易所对比"</h3><span>"原生市场 · 仅观察"</span></header>
        <div class="stock-peer-search stock-quote-form">
            <label>"交易所"<select aria-label="股票对比交易所" prop:value=move ||p.venue.get() on:change=move |ev|{p.venue.set(event_target_value(&ev));p.refresh.run(());}>
                <option value="kraken">"Kraken"</option><option value="bitget">"Bitget"</option><option value="gate">"Gate"</option>
                <option value="bybit">"Bybit"</option><option value="binance">"Binance"</option><option value="okx">"OKX"</option>
                <option value="kucoin">"KuCoin"</option><option value="hyperliquid">"Hyperliquid"</option>
                <option value="hyperliquid:xyz">"Hyperliquid · XYZ"</option><option value="gate_crossex">"Gate CrossEx"</option>
            </select></label>
            <label>"产品"<select aria-label="股票对比产品" prop:value=move ||p.product.get().as_str() on:change=move |ev|{p.product.set(if event_target_value(&ev)=="spot"{StockPeerProduct::Spot}else{StockPeerProduct::Perpetual});p.refresh.run(());}>
                <option value="spot">"现货"</option><option value="perpetual">"永续"</option>
            </select></label>
            <label>"市场搜索"<input type="search" aria-label="股票对比市场搜索" placeholder="股票或原生市场" prop:value=move ||p.search.get()
                on:input=move |ev|p.search.set(event_target_value(&ev)) on:keydown=move |ev|{if ev.key()=="Enter"{p.refresh.run(());}}/></label>
            <button type="button" class="row-action" on:click=move |_|p.refresh.run(())>"搜索"</button>
        </div>
        <div class="stock-preflight-form stock-peer-select">
            <label>"选择对比市场"<select aria-label="选择对比市场" prop:value=move ||selected_native.get()
                disabled=move ||p.loading.get() ||p.pending.get() ||catalog.get().is_none() on:change=move |ev|{
                    let native=event_target_value(&ev);
                    // Keep the committed market visible until the backend accepts the change.
                    event_target::<web_sys::HtmlSelectElement>(&ev).set_value(&selected_native.get_untracked());
                    if let Some(s)=catalog.with_untracked(|c|c.as_ref().and_then(|c|c.rows.iter().find(|r|r.native_symbol==native).map(|r|StockPeerSelection{
                        venue:r.venue.clone(),product:c.request.product,native_symbol:r.native_symbol.clone()}))){p.select.run(Some(s));}
                }>
                <option value="" selected=move ||selected_native.get().is_empty()>{move ||if p.loading.get(){"正在读取目录…"}else{"请选择，不会自动配对"}}</option>
                {move ||{
                    let native=selected_native.get();
                    (!native.is_empty() &&catalog.with(|c|c.as_ref().is_none_or(|c|c.rows.iter().all(|r|r.native_symbol!=native))))
                        .then(||view!{<option value=native.clone() selected=true disabled=true>{format!("{native} · 当前对比")}</option>})
                }}
                {move ||catalog.with(|c|c.as_ref().map(|c|c.rows.clone()).unwrap_or_default()).into_iter().map(|s|{
                    let native=s.native_symbol.clone();
                    view!{<option value=s.native_symbol.clone() selected=move ||selected_native.get()==native>{format!("{} · {}",s.native_symbol,s.quote_asset.unwrap_or_default())}</option>}
                }).collect_view()}
            </select></label>
            <button type="button" class="row-action" disabled=move ||p.pending.get() ||peer.get().is_none() on:click=move |_|p.select.run(None)>"移除对比"</button>
        </div>
        {move ||p.problem.get().map(|e|view!{<p role="alert" class="stock-problem">{e}</p>})}
        <p class="stock-rfq-note" role="status">{move ||if p.pending.get(){"正在确认市场选择，当前对比保持不变".into()}else{catalog.with(|c|c.as_ref().map(|c|if c.registry_count==0{"尚未读取该交易所的官方市场，请确认订阅配置。".into()}else{format!("匹配 {} 个市场，显示 {} 个",c.matched,c.rows.len())}).unwrap_or_else(||if p.loading.get(){"正在读取市场目录，尚未改变当前对比".into()}else{"搜索后选择市场，当前对比保持不变".into()}))}}</p>
        {move ||peer.get().map(|s|{
            let quote=s.instrument.as_ref().and_then(|i|i.quote_asset.clone()).unwrap_or_else(||"未知计价币".into());
            view!{<div class="stock-peer-current">
                <header><strong>{format!("当前对比 · {} · {}",s.selection.venue,s.selection.native_symbol)}</strong><span>{if s.selection.product==StockPeerProduct::Spot{"现货"}else{"永续 · 非股票库存"}}</span></header>
                {s.problem.map(|e|view!{<p class="stock-rfq-note" role="status">{e}</p>})}
                <dl class="stock-summary">
                    <div><dt>{format!("买价 / {quote}")}</dt><dd>{s.quote.as_ref().map(|q|q.bid.clone()).unwrap_or_else(||"—".into())}</dd></div>
                    <div><dt>{format!("卖价 / {quote}")}</dt><dd>{s.quote.as_ref().map(|q|q.ask.clone()).unwrap_or_else(||"—".into())}</dd></div>
                    <div><dt>{if s.share_unit_verified {"买量 / 股"}else{"买量 / 原生单位"}}</dt><dd>{s.quote.as_ref().and_then(|q|q.bid_quantity.clone()).unwrap_or_else(||"未知".into())}</dd></div>
                    <div><dt>{if s.share_unit_verified {"卖量 / 股"}else{"卖量 / 原生单位"}}</dt><dd>{s.quote.as_ref().and_then(|q|q.ask_quantity.clone()).unwrap_or_else(||"未知".into())}</dd></div>
                </dl>
                <p class="stock-rfq-note">{move ||peer.with(|p|p.as_ref().and_then(|p|p.quote.as_ref()).map(|q|peer_quote_age(q,data.clock.get())).unwrap_or_else(||"尚无买卖报价".into()))}</p>
                {(!s.share_unit_verified).then(||view!{<p class="stock-rfq-note">"股数换算待核实 · 保留原生报价，不计算收益"</p>})}
                {s.share_unit_verified.then(||view!{<p class="stock-rfq-note">"行情按股数对比 · 不是可直接充提的 Token 数量"</p>})}
                {s.quote_conversion.map(|f|view!{<p class="stock-rfq-note">{format!("换汇 {} · 买 {} / 卖 {} · {}",f.symbol,f.bid,f.ask,peer_quote_age(&f,data.clock.get()))}</p>})}
            </div>}
        })}
            <details class="stock-peer-identity" hidden=move ||identity.get().is_none()>
                <summary>{move ||if identity.with(|i|i.as_ref().is_some_and(|i|i.underlying_verified)){"同一经济标的 · 不同发行方"}else{"证券对应关系待核实"}}</summary>
                {move ||identity.get().map(|i|view!{
                    <p>{i.reason}</p><dl class="stock-plan-evidence">
                        <div><dt>"标的 ISIN"</dt><dd>{i.underlying_isin.unwrap_or_else(||"未核实".into())}</dd></div>
                        <div><dt>"产品 ISIN"</dt><dd>{i.product_isin.unwrap_or_else(||"未核实".into())}</dd></div>
                        <div><dt>"发行方"</dt><dd>{i.issuer.unwrap_or_else(||"未核实".into())}</dd></div>
                    </dl>
                    {i.sources.into_iter().enumerate().map(|(index,url)|view!{<a href=url target="_blank" rel="noopener noreferrer">{format!("官方依据 {}",index+1)}</a>}).collect_view()}
                })}
            </details>
        <div class="stock-directions">{move ||estimates.get().into_iter().map(|e|view!{<section class="stock-direction">
            <header><h4>{if e.chain_buy{"链买 / 所选交易所卖"}else{"所选交易所买 / 链卖"}}</h4><span>"费用前试算"</span></header>
            <dl class="stock-direction-values"><div><dt>"对齐股数"</dt><dd>{comparison::quantity(e.shares)}</dd></div>
                <div><dt>"差额 / USDC"</dt><dd>{comparison::quantity(e.gross_usdc)}</dd></div>
                <div><dt>"未对齐余量 / 股"</dt><dd>{comparison::quantity(e.remainder_shares)}</dd></div></dl>
            <ul>{e.blockers.into_iter().map(|p|view!{<li>{p}</li>}).collect_view()}</ul>
        </section>}).collect_view()}</div>
        {preflight::panel(data)}
        {order::panel(data)}
        {super::peer_plans::builder(data)}
        {funding::panel(data)}
    </section>}
}

fn peer_quote_age(q: &StockPeerQuote, now: i64) -> String {
    let age = |t: i64| {
        if t > now {
            "时钟待核实".into()
        } else {
            format!("{:.1}s", now.saturating_sub(t) as f64 / 1000.0)
        }
    };
    format!(
        "{} · 缓存 {} · 源行情 {} · {}",
        if q.source == "ws_push" {
            "WS"
        } else {
            q.source.as_str()
        },
        age(q.received_at_ms),
        q.source_at_ms.map(age).unwrap_or_else(|| "未知".into()),
        if q.fresh_ws(now) {
            "新鲜报价"
        } else {
            "仅历史报价"
        }
    )
}
