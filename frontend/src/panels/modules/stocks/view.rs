use super::data::{use_data, StockData, StocksRuntime};
use crate::panels::shared::ModuleHeader;
use leptos::prelude::*;
use shared_types::stocks::*;

mod comparison;
mod peers;
mod preflight;
mod plans;
mod peer_plans;
mod peer_recovery;
mod peer_conversion;
mod peer_inventory;
mod peer_native_topup;
mod funding;
mod stablecoin;
mod exchange_conversion;
mod funding_plans;
mod recovery;
mod execution;
mod rfq;
mod alerts;

pub(in crate::panels) fn stocks_module(runtime: StocksRuntime) -> impl IntoView {
    let data = use_data(runtime);
    page(data, || {
        crate::panels::shared::onchain_provider_credentials_editor(
            RwSignal::new("backpack_stocks".into()),
            false,
            true,
        )
    })
}

fn page<V: IntoView + 'static>(data: StockData, credentials: fn() -> V) -> impl IntoView {
    let security = Memo::new(move |_| {
        data.market
            .with(|state| state.value().and_then(|s| s.security.clone()))
    });
    view! { <section class="module-page stock-arbitrage-page">
        <ModuleHeader title="股票套利"/>
        <div class="stock-workbench">
            {catalog(data)}
            <main class="stock-main">
                {move || data.notice.get().map(|p| view! { <p role="alert" class="stock-problem">{p}</p> })}
                {move || data.market.with(|state| state.problem().map(|p| view! { <p role="alert" class="stock-problem">{p.message.clone()}</p> }))}
                {move ||security.get().is_none().then(||view! {<p class="stock-empty">"选择股票后查看市场与链上合约"</p>})}
                {move ||security.get().is_none().then(||rfq::panel(data,credentials()))}
                {move ||security.get().is_none().then(||stablecoin::recovery_panel(data))}
                <For each={move || security.get().into_iter().collect::<Vec<_>>()} key=|s| s.asset.clone()
                    children=move |s| detail(s, data,credentials)/>
                {plans::panel(data)}
                {peer_plans::history(data)}
                {funding_plans::panel(data)}
                {exchange_conversion::panel(data)}
            </main>
        </div>
    </section> }
}

fn catalog(data: StockData) -> impl IntoView {
    catalog_with_filter(data, RwSignal::new(false))
}

fn catalog_rows(catalog: &StockCatalog, search: &str, verified_only: bool) -> Vec<StockSecurity> {
    let search = search.trim().to_uppercase();
    let mut rows = catalog.rows.iter().filter(|s| {
        (!verified_only || identity::backpack_issuer(s).is_ok())
            && (s.asset.contains(&search)
                || s.name.to_uppercase().contains(&search)
                || s.cusip.as_deref().is_some_and(|c| c.contains(&search)))
    }).cloned().collect::<Vec<_>>();
    rows.sort_by(|a, b| {
        identity::backpack_issuer(a).is_err().cmp(&identity::backpack_issuer(b).is_err())
            .then_with(|| a.asset.cmp(&b.asset))
    });
    rows
}

fn catalog_with_filter(data: StockData, verified_only: RwSignal<bool>) -> impl IntoView {
    let filtered = Memo::new(move |_| {
        let search = data.search.get();
        let verified_only = verified_only.get();
        data.catalog.with(|c| {
            c.value()
                .map(|c| catalog_rows(c, &search, verified_only))
                .unwrap_or_default()
        })
    });
    let page = Memo::new(move |_| data.page.get().min(filtered.with(|r| r.len().div_ceil(40).saturating_sub(1))));
    let coverage = Memo::new(move |_| data.catalog.with(|c| c.value().map(|c| {
        let verified = c.rows.iter().filter(|s| identity::backpack_issuer(s).is_ok()).count();
        format!("链上资料已核实 {verified} / {}", c.rows.len())
    })));
    view! { <aside class="stock-catalog" aria-label="Backpack 股票目录">
        <header><div><strong>"Backpack"</strong><span>"官方证券目录"</span></div>
            <button type="button" class="icon-button" title="刷新股票目录" aria-label="刷新股票目录" on:click=move |_| data.refresh.run(())>"↻"</button></header>
        <label class="stock-search"><span class="sr-only">"搜索股票"</span><input type="search" placeholder="股票、公司或 CUSIP"
            prop:value=move || data.search.get() on:input=move |ev| {data.search.set(event_target_value(&ev));data.page.set(0);}/></label>
        <div class="stock-catalog-filter"><label><input type="checkbox" aria-label="只看链上资料已核实" prop:checked=move ||verified_only.get()
            on:change=move |ev| {verified_only.set(event_target_checked(&ev));data.page.set(0);}/><span>"只看链上资料已核实"</span></label>
            <small>{move ||coverage.get().unwrap_or_default()}</small></div>
        <p class="stock-catalog-count">{move || if data.catalog.with(|c|c.value().is_none() && c.problem().is_none()) {"正在读取官方目录…".into()} else {format!("{} 个证券", filtered.with(Vec::len))}}</p>
        {move || data.catalog.with(|c| c.problem().map(|p| view! {<p role="alert" class="stock-problem">{p.message.clone()}</p>}))}
        <div class="stock-security-list">
            {move || filtered.with(|rows| rows.iter().skip(page.get()*40).take(40).cloned().map(|s| {
                let asset = s.asset.clone(); let active = s.asset.clone();
                let checked = identity::backpack_issuer(&s).is_ok();
                view! {<button type="button" class="stock-security" aria-pressed=move || data.market.with(|m| m.value().and_then(|m| m.security.as_ref()).is_some_and(|s|s.asset==active)).to_string()
                    disabled=move || data.pending.get() on:click=move |_| data.watch.run(Some(asset.clone()))>
                    <strong>{s.ticker}</strong><span>{s.name}</span><small>{if s.order_books.is_empty(){"RFQ"}else{"RFQ / 订单簿"}}</small>
                    <small class="stock-security-coverage" data-verified=if checked{"true"}else{"false"}>{if checked{"链上资料已核实"}else{"链上资料待核实"}}</small>
                </button>}
            }).collect_view())}
            {move ||(filtered.with(Vec::is_empty) && data.catalog.with(|c|c.value().is_some())).then(||view!{<p class="stock-empty-inline">"没有符合条件的证券"</p>})}
        </div>
        <footer class="stock-pagination">
            <button type="button" class="icon-button" title="上一页" aria-label="上一页" disabled=move || page.get()==0
                on:click=move |_| data.page.set(page.get().saturating_sub(1))>"←"</button>
            <span>{move || format!("{} / {}",page.get()+1,filtered.with(|r| r.len().div_ceil(40).max(1)))}</span>
            <button type="button" class="icon-button" title="下一页" aria-label="下一页" disabled=move || {(page.get()+1)*40>=filtered.with(Vec::len)}
                on:click=move |_| data.page.set(page.get()+1)>"→"</button>
        </footer>
    </aside> }
}

fn detail<V: IntoView + 'static>(
    s: StockSecurity,
    data: StockData,
    credentials: fn() -> V,
) -> impl IntoView {
    let connected = Memo::new(move |_| {
        data.market
            .with(|m| m.problem().is_none() && m.value().is_some_and(|s| s.connected))
    });
    let ref_quote = Memo::new(move |_| {
        data.market
            .with(|m| m.value().and_then(|s| s.reference.clone()))
    });
    let tokens = Memo::new(move |_| {
        data.market
            .with(|m| m.value().map(|s| s.tokens.clone()).unwrap_or_default())
    });
    let route = Memo::new(move |_| {
        data.market
            .with(|m| m.value().and_then(|s| s.trading_route.clone()))
    });
    let security = Memo::new(move |_| {
        data.market
            .with(|m| m.value().and_then(|s| s.security.clone()))
    });
    let status = Memo::new(move |_| data.market.with(|m| m.value().map(|s|
        super::readiness::module_status(s, &data.preflight.wallet.get(), &data.budget.get(), data.keyed.get(), data.clock.get()))
        .unwrap_or(("等待行情", "股票行情尚未就绪"))));
    let quote_rows = move || {
        security.with(|s|s.as_ref().map(|s|s.order_books.clone()).unwrap_or_default()).into_iter().map(|market| {
        let symbol = market.symbol.clone();
        let book = Memo::new(move |_| data.market.with(|m|m.value().and_then(|s|s.books.iter().find(|b|b.symbol==symbol).cloned())));
        let quote = market.quote.clone();
        view! {<tr><td><strong>{market.symbol.clone()}</strong><small>{format!("订单簿 {}",market.state)}</small></td>
            <td class="stock-number" data-label="买价">{move ||book.with(|b|b.as_ref().and_then(|b| b.bid.clone()).unwrap_or_else(|| "—".into()))}<small>{quote.clone()}</small></td>
            <td class="stock-number" data-label="买量 / 股">{move ||book.with(|b|b.as_ref().and_then(|b| b.bid_quantity.clone()).unwrap_or_else(|| "—".into()))}</td>
            <td class="stock-number" data-label="卖价">{move ||book.with(|b|b.as_ref().and_then(|b| b.ask.clone()).unwrap_or_else(|| "—".into()))}<small>{quote}</small></td>
            <td class="stock-number" data-label="卖量 / 股">{move ||book.with(|b|b.as_ref().and_then(|b| b.ask_quantity.clone()).unwrap_or_else(|| "—".into()))}</td>
            <td data-label="时效">{move || quote_age(connected.get(),book.with(|b|b.as_ref().map(|b|b.source_at_ms)),data.clock.get())}</td>
        </tr>}
    }).collect_view()
    };
    view! {
        <header class="stock-heading"><div><span>"BACKPACK · STOCKS"</span><h2>{s.ticker.clone()}</h2><p>{s.name.clone()}</p></div>
            <div class="stock-heading-actions"><span class="read-only-flag">{move || if connected.get() {"WS 已连接"}else{"WS 未连接"}}</span>
                <button type="button" class="row-action" disabled=move || data.pending.get() on:click=move |_| data.watch.run(None)>"停止监控"</button></div>
        </header>
        {move ||data.market.with(|m|m.value().and_then(|s|s.problem.clone())).map(|p|view!{<p role="status" class="stock-problem">{p}</p>})}
        {move ||(connected.get() && data.market.with(|m|m.value().is_some_and(|s|s.books.is_empty() && s.reference.is_none()))).then(||view!{<p role="status" class="stock-empty-inline">"WS 已连接，等待交易所推送该股票的行情。"</p>})}
        <dl class="stock-summary"><div><dt>"证券身份"</dt><dd>{s.asset.clone()}</dd></div><div><dt>"CUSIP"</dt><dd>{s.cusip.clone().unwrap_or_else(|| "官方未提供".into())}</dd></div>
            <div><dt>"链上报价"</dt><dd>{move || data.market.with(|m| if m.value().is_some_and(|m|m.comparison.is_some()) {"Jupiter · 只读"} else {"等待询价"})}</dd></div><div><dt>"计划状态"</dt><dd>{move ||status.get().0}</dd></div></dl>
        <div class="stock-trading-route" role="status">
            <strong>{move ||route.with(|r|match r.as_ref().filter(|r|r.valid_until_ms>data.clock.get()).map(|r|r.kind) {
                Some(StockRouteKind::Rfq)=>"当前通道 · RFQ",Some(StockRouteKind::OrderBook)=>"当前通道 · 现货订单簿",
                Some(StockRouteKind::Closed)=>"当前休市",_=>"交易通道待核验",
            })}</strong>
            <span>{move ||route.with(|r|r.as_ref().map(|r|if r.kind!=StockRouteKind::Unknown && r.valid_until_ms<=data.clock.get(){"交易日历状态已陈旧，等待更新".into()}else{r.reason.clone()}).unwrap_or_else(||"正在读取官方日历".into()))}</span>
            {move ||route.with(|r|r.as_ref().and_then(|r|r.session.clone())).map(|s|view!{<small>{format!("{} · 最少 {} 股 · 步长 {}",session_label(&s.name),s.min_quantity,s.step_size)}</small>})}
        </div>
        {comparison::panel(s.asset.clone(), data)}
        {preflight::panel(s.asset.clone(), data)}
        {rfq::panel(data,credentials())}
        <section class="stock-section"><header><h3>"交易所买卖一档"</h3><span>"仅显示该市场 WS 盘口"</span></header>
            <div class="stock-table-scroll"><table class="stock-bbo"><thead><tr><th>"市场"</th><th>"买价"</th><th>"买量 / 股"</th><th>"卖价"</th><th>"卖量 / 股"</th><th>"时效"</th></tr></thead><tbody>{quote_rows}</tbody></table></div>
            {move ||security.with(|s|s.as_ref().is_none_or(|s|s.order_books.is_empty())).then(||view!{<p class="stock-empty-inline">"该证券没有股票订单簿市场，需通过 RFQ 获取可成交报价。"</p>})}
        </section>
        <section class="stock-section"><header><h3>"外部股票参考"</h3><span>"非订单簿 / 非 RFQ 成交承诺"</span></header>
            {move ||data.market.with(|m|m.value().and_then(|s|s.reference_problem.clone())).map(|p|view!{<p class="stock-empty-inline" role="status">{p}</p>})}
            {move ||ref_quote.get().is_none().then(||view!{<p class="stock-empty-inline" role="status">"外部参考源尚未返回报价"</p>})}
            <dl class="stock-summary"><div><dt>"参考买价"</dt><dd>{move ||ref_quote.with(|q|q.as_ref().and_then(|q|q.bid.clone()).unwrap_or_else(||"—".into()))}</dd></div>
                <div><dt>"参考卖价"</dt><dd>{move ||ref_quote.with(|q|q.as_ref().and_then(|q|q.ask.clone()).unwrap_or_else(||"—".into()))}</dd></div>
                <div><dt>"中间价"</dt><dd>{move ||ref_quote.with(|q|q.as_ref().map(|q|q.mid.clone()).unwrap_or_else(||"—".into()))}</dd></div>
                <div><dt>"来源时效"</dt><dd>{move ||quote_age(connected.get(),ref_quote.with(|q|q.as_ref().map(|q|q.source_at_ms)),data.clock.get())}</dd></div></dl>
        </section>
        {peers::panel(data)}
        <section class="stock-section"><header><h3>"链上合约与充提"</h3><span>"公共目录快照 · 执行前需重查"</span></header>
            {move ||data.market.with(|m|m.value().and_then(|s|s.token_metadata_problem.clone())).map(|p|view!{<p class="stock-problem">{p}</p>})}
            {move ||tokens.with(Vec::is_empty).then(||view!{<p class="stock-empty-inline">"未取得该证券的官方链上合约映射"</p>})}
            {move ||tokens.get().into_iter().map(|t|view!{<div class="stock-token"><header><strong>{t.blockchain}</strong><span>{t.native_decimals.map(|n|format!("{n} 位精度")).unwrap_or_else(||"精度未提供".into())}</span></header>
                <code>{t.contract_address.unwrap_or_else(||"官方未提供合约地址，不能建立链上报价".into())}</code><dl class="stock-summary"><div><dt>"充值"</dt><dd>{flag(t.deposit_enabled)}</dd></div>
                    <div><dt>"提现"</dt><dd>{flag(t.withdraw_enabled)}</dd></div><div><dt>"提币费（原资产）"</dt><dd>{t.withdrawal_fee.unwrap_or_else(||"未知".into())}</dd></div>
                    <div><dt>"最低提币（原资产）"</dt><dd>{t.minimum_withdrawal.unwrap_or_else(||"未知".into())}</dd></div></dl></div>}).collect_view()}
        </section>
        <details class="stock-section"><summary>"交易时段与数量限制"</summary><div class="stock-table-scroll"><table><thead><tr><th>"官方时段"</th><th>"最少 / 股"</th><th>"最多 / 股"</th><th>"步长"</th></tr></thead><tbody>
            {move ||security.with(|s|s.as_ref().map(|s|s.sessions.clone()).unwrap_or_default()).into_iter().map(|session|view!{<tr><td>{session.name}</td><td>{session.min_quantity}</td><td>{session.max_quantity.unwrap_or_else(||"未提供".into())}</td><td>{session.step_size}</td></tr>}).collect_view()}
        </tbody></table></div></details>
        <footer class="stock-readiness"><strong>{move ||status.get().0}</strong><span>{move ||status.get().1}</span>
            <a href="https://docs.backpack.exchange/" target="_blank" rel="noopener noreferrer">"官方接口"</a></footer>
    }
}

fn session_label(name: &str) -> &str {
    match name {
        "US_EQUITIES_PRE_MARKET" => "美股盘前",
        "US_EQUITIES_REGULAR" => "美股常规时段",
        "US_EQUITIES_POST_MARKET" => "美股盘后",
        "US_EQUITIES_OVERNIGHT" => "美股夜盘",
        _ => name,
    }
}

fn flag(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "开放",
        Some(false) => "关闭",
        None => "未知",
    }
}
fn quote_age(connected: bool, time: Option<i64>, now: i64) -> String {
    if !connected {
        return "已断开 · 不可执行".into();
    }
    match time {
        Some(t) if t <= now && now - t <= 3000 => format!("{}ms", now - t),
        Some(t) if t <= now => format!("{}s · 已陈旧", (now - t) / 1000),
        _ => "等待报价".into(),
    }
}

#[cfg(test)]
mod tests;
