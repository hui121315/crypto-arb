use super::data::{use_data, StockData, StocksRuntime};
use crate::panels::shared::ModuleHeader;
use leptos::prelude::*;
use shared_types::stocks::*;

mod comparison;
mod peers;
mod preflight;
mod plans;
mod history;
mod peer_plans;
mod peer_recovery;
mod peer_conversion;
mod peer_inventory;
mod peer_native_topup;
mod funding;
mod restock;
mod stablecoin;
mod exchange_conversion;
mod conversion_costs;
mod funding_plans;
mod recovery;
mod execution;
mod rfq;
mod alerts;
mod batch;

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
    let catalog_open = RwSignal::new(false);
    let picker_toggle = NodeRef::<leptos::html::Button>::new();
    let close_picker = Callback::new(move |_| {
        catalog_open.set(false);
        if let Some(button) = picker_toggle.get() { let _ = button.focus(); }
    });
    let security = Memo::new(move |_| {
        data.market
            .with(|state| state.value().and_then(|s| s.security.clone()))
    });
    let funds_status = Memo::new(move |_| data.market.with(|m|m.value()
        .and_then(|s|super::readiness::funds_status(s,data.clock.get()))));
    let initial_view = StoredValue::new(false);
    Effect::new(move |_| {
        let selected = security.get();
        if data.pending.get() || data.market.with(|state| state.value().is_none()) { return; }
        let section = data.section.get_untracked();
        let initialize = !initial_view.get_value();
        initial_view.set_value(true);
        if initialize && selected.is_some() && section == 6 {
            data.section.set(0);
        } else if initialize && selected.is_none() && funds_status.get().is_some() && section == 6 {
            data.section.set(3);
        } else if selected.is_none() && !matches!(section,1|2|3|6) {
            data.section.set(6);
        }
    });
    view! { <section class="module-page stock-arbitrage-page">
        <div class="stock-module-heading">
            <ModuleHeader title="股票套利"/>
            <span class="stock-module-source">"Backpack / Solana"</span>
            {move ||funds_status.get().map(|(label,detail)|view!{
                <button type="button" class="stock-funds-shortcut" title=detail aria-label="查看待处理资金"
                    on:click=move |_|data.section.set(3)>{label}</button>
            })}
            <button node_ref=picker_toggle type="button" class="row-action stock-picker-toggle" aria-label="选择股票"
                aria-controls="stock-picker" aria-expanded=move ||catalog_open.get().to_string()
                on:click=move |_|catalog_open.update(|open|*open=!*open)>
                {move ||if catalog_open.get(){"收起目录"}else{"选择股票"}}
                <span>{move ||format!("{} / {}",data.batch.selected.with(Vec::len),STOCK_BATCH_LIMIT)}</span>
            </button>
        </div>
        <div id="stock-picker" class="stock-picker" hidden=move ||!catalog_open.get()
            on:keydown=move |ev|{if ev.key()=="Escape"{ev.stop_propagation();close_picker.run(());}}>
            {catalog(data)}
            <div class="stock-picker-footer"><button type="button" class="row-action" on:click=move |_|close_picker.run(())>"完成选择"</button></div>
        </div>
        <div class="stock-workbench">
            <section class="stock-main" aria-label="股票监控与操作">
                {move || data.notice.get().map(|p| view! { <p role=if p.problem{"alert"}else{"status"}
                    class=if p.problem{"stock-problem"}else{"stock-notice"}>{p.message}</p> })}
                {move || data.market.with(|state| state.problem().map(|p| view! { <p role="alert" class="stock-problem">{p.message.clone()}</p> }))}
                {crate::panels::shared::operation_journal::settings_recovery_panel(data.monitor_journal, data.monitor_recheck)}
                {crate::panels::shared::operation_journal::settings_recovery_panel(data.preflight.build_journal, data.preflight.build_recheck)}
                {crate::panels::shared::operation_journal::settings_recovery_panel(data.peers.plans.journal, data.peers.plans.build_recheck)}
                <div class="stock-workspace-context" hidden=move ||data.section.get()==6>
                {move ||security.get().map(|s|view!{
                    <header class="stock-heading"><div class="stock-heading-identity"><h2>{s.ticker}</h2><p>{s.name}</p></div>
                        <div class="stock-heading-actions"><span class="stock-connection" data-connected=move ||data.market.with(|m|m.problem().is_none() && m.value().is_some_and(|s|s.connected)).to_string()>
                            {move ||if data.market.with(|m|m.problem().is_none() && m.value().is_some_and(|s|s.connected)){"WS 已连接"}else{"WS 未连接"}}</span>
                            <button type="button" class="stock-back-to-market" on:click=move |_|data.section.set(6)>"返回监控"</button>
                            <button type="button" class="row-action" disabled=move ||data.pending.get() on:click=move |_|{if !matches!(data.section.get_untracked(),1|2|3){data.section.set(3);}data.watch.run(None);}>"关闭详情"</button>
                        </div>
                    </header>
                })}
                {move ||data.market.with(|m|m.value().is_some_and(|s|s.security.is_none())).then(||view!{
                    <header class="stock-heading"><div class="stock-heading-identity"><h2>"账户与记录"</h2><p>"未选择股票"</p></div>
                        {move ||funds_status.get().map(|(label,detail)|view!{<span class="stock-account-state" title=detail>{label}</span>})}
                    </header>
                })}
                </div>
                <nav class="module-toolbar stock-detail-tabs" aria-label="股票详情视图">
                    {[(6,"市场监控"),(0,"行情与提醒"),(5,"跨所对比"),(1,"库存与成本"),(2,"询价与执行"),(3,"执行记录"),(4,"合约资料")].into_iter().map(|(id,label)|view!{
                        <button type="button" aria-pressed=move ||(data.section.get()==id).to_string()
                            class:active=move ||data.section.get()==id
                            hidden=move ||security.get().is_none() && !matches!(id,1|2|3|6)
                            disabled=move ||security.get().is_none() && !matches!(id,1|2|3|6)
                            on:click=move |_|data.section.set(id)>{label}</button>
                    }).collect_view()}
                </nav>
                <div hidden=move ||data.section.get()!=6>{batch::panel(data)}</div>
                <div class="stock-detail-workspace" hidden=move ||data.section.get()==6>
                <div class="stock-detail-panel" hidden=move ||data.section.get()!=2 ||security.get().is_some()>
                    {move || security.get().is_none().then(|| rfq::panel(data,credentials()))}
                </div>
                <div class="stock-detail-panel" hidden=move ||data.section.get()!=1 ||security.get().is_some()>
                    {move || security.get().is_none().then(|| stablecoin::recovery_panel(data))}
                </div>
                <For each={move || security.get().into_iter().collect::<Vec<_>>()} key=|s| s.asset.clone()
                    children=move |s| detail(s, data,credentials)/>
                <div class="stock-detail-panel" hidden=move ||data.section.get()!=1>
                    {exchange_conversion::panel(data)}
                </div>
                <div class="stock-detail-panel" hidden=move ||data.section.get()!=3>
                    {plans::panel(data)}
                    {peer_plans::history(data)}
                    {funding_plans::panel(data)}
                </div>
                </div>
            </section>
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
    let selected_only = RwSignal::new(false);
    let filtered = Memo::new(move |_| {
        let search = data.search.get();
        let verified_only = verified_only.get();
        let selected = selected_only.get().then(|| data.batch.selected.get());
        data.catalog.with(|c| {
            c.value()
                .map(|c| catalog_rows(c, &search, verified_only).into_iter()
                    .filter(|s| selected.as_ref().is_none_or(|assets| assets.contains(&s.asset)))
                    .collect::<Vec<_>>())
                .unwrap_or_default()
        })
    });
    let page = Memo::new(move |_| data.page.get().min(filtered.with(|r| r.len().div_ceil(40).saturating_sub(1))));
    let visible = Memo::new(move |_| filtered.with(|rows|
        rows.iter().skip(page.get() * 40).take(40).cloned().collect::<Vec<_>>()));
    let page_additions = Memo::new(move |_| {
        let selected = data.batch.selected.get();
        visible.with(|rows| rows.iter()
            .filter(|s| !selected.contains(&s.asset))
            .take(STOCK_BATCH_LIMIT.saturating_sub(selected.len()))
            .map(|s| s.asset.clone()).collect::<Vec<_>>())
    });
    let coverage = Memo::new(move |_| data.catalog.with(|c| c.value().map(|c| {
        let verified = c.rows.iter().filter(|s| identity::backpack_issuer(s).is_ok()).count();
        format!("发行资料已收录 {verified} / {}", c.rows.len())
    })));
    view! { <aside class="stock-catalog" aria-label="Backpack 股票目录">
        <header><div><strong>"Backpack"</strong><span>"官方证券目录"</span></div>
            <button type="button" class="icon-button" title="刷新股票目录" aria-label="刷新股票目录" on:click=move |_| data.refresh.run(())>"↻"</button></header>
        <label class="stock-search"><span class="sr-only">"搜索股票"</span><input type="search" placeholder="股票、公司或 CUSIP"
            prop:value=move || data.search.get() on:input=move |ev| {data.search.set(event_target_value(&ev));data.page.set(0);}/></label>
        <div class="stock-catalog-filter"><label><input type="checkbox" aria-label="只看已收录发行资料" prop:checked=move ||verified_only.get()
            on:change=move |ev| {verified_only.set(event_target_checked(&ev));data.page.set(0);}/><span>"只看已收录发行资料"</span></label>
            <label><input type="checkbox" aria-label="只看已选监控" prop:checked=move ||selected_only.get()
                on:change=move |ev| {selected_only.set(event_target_checked(&ev));data.page.set(0);}/><span>"只看已选监控"</span></label>
            <small>{move ||coverage.get().unwrap_or_default()}</small></div>
        <p class="stock-catalog-count">{move || data.catalog.with(|state| match (state.value(), state.problem()) {
            (None, None) => "正在读取官方目录…".into(),
            (None, Some(_)) => "证券数量待确认".into(),
            (Some(_), Some(_)) => format!("{} 个证券 · 上次目录", filtered.with(Vec::len)),
            (Some(_), None) => format!("{} 个证券", filtered.with(Vec::len)),
        })}</p>
        <div class="stock-catalog-actions"><span>{move ||format!("监控选择 {} / {}",data.batch.selected.with(Vec::len),STOCK_BATCH_LIMIT)}</span>
            <div><button type="button" class="row-action" disabled=move ||data.batch.pending.get() ||page_additions.with(Vec::is_empty)
                on:click=move |_| {
                    let additions = page_additions.get_untracked();
                    data.batch.selected.update(|selected| selected.extend(additions));
                }>{move ||format!("加入本页 {}",page_additions.with(Vec::len))}</button>
            <button type="button" class="row-action" disabled=move ||data.batch.pending.get() ||data.batch.selected.with(Vec::is_empty)
                on:click=move |_|data.batch.selected.set(vec![])>"清空选择"</button></div></div>
        {move || data.catalog.with(|c| c.problem().map(|p| view! {<p role="alert" class="stock-problem">{p.message.clone()}</p>}))}
        <div class="stock-security-list">
            <For each=move ||visible.get() key=|s|s.asset.clone() children=move |s| {
                let asset = s.asset.clone(); let active = s.asset.clone();
                let selected_asset=s.asset.clone();let toggle_asset=s.asset.clone();
                let label=format!("监控 {}",s.ticker);
                let checked = identity::backpack_issuer(&s).is_ok();
                view! {<div class="stock-catalog-row"><input type="checkbox" aria-label=label
                    prop:checked=move ||data.batch.selected.with(|s|s.contains(&selected_asset)) disabled=move ||data.batch.pending.get()
                    on:change=move |ev|{
                        let checked=event_target_checked(&ev);
                        if checked && data.batch.selected.with(Vec::len)>=STOCK_BATCH_LIMIT {
                            event_target::<web_sys::HtmlInputElement>(&ev).set_checked(false);
                            data.notice.set(Some(format!("每批最多 {} 只股票",STOCK_BATCH_LIMIT)));return;
                        }
                        data.batch.selected.update(|rows|{rows.retain(|s|s!=&toggle_asset);if checked{rows.push(toggle_asset.clone());}});
                    }/>
                    <button type="button" class="stock-security" aria-pressed=move || data.market.with(|m| m.value().and_then(|m| m.security.as_ref()).is_some_and(|s|s.asset==active)).to_string()
                    disabled=move || data.pending.get() on:click=move |_| {data.watch.run(Some(asset.clone()));data.section.set(0);}>
                    <strong>{s.ticker}</strong><span>{s.name}</span><small>{if s.order_books.is_empty(){"询价"}else{"询价 / 订单簿"}}</small>
                    <small class="stock-security-coverage" data-verified=if checked{"true"}else{"false"}>{if checked{"发行资料已收录"}else{"发行资料未收录"}}</small>
                </button></div>}
            }/>
            {move ||(filtered.with(Vec::is_empty) && data.catalog.with(|c|c.value().is_some())).then(||view!{<p class="stock-empty-inline">"没有符合条件的证券"</p>})}
        </div>
        <footer class="stock-pagination">
            <button type="button" class="icon-button" title="上一页" aria-label="上一页" disabled=move || page.get()==0
                on:click=move |_| data.page.set(page.get().saturating_sub(1))>"←"</button>
            <span>{move || if data.catalog.with(|c| c.value().is_none()) { "— / —".into() }
                else { format!("{} / {}",page.get()+1,filtered.with(|r| r.len().div_ceil(40).max(1))) }}</span>
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
    let status = Memo::new(move |_| if data.peers.plans.journal.locked() {
        ("双边构建待核对", "原请求尚未核清，请核对上次操作；不会自动重复预留或提交订单")
    } else {data.market.with(|m| m.value().map(|s|
        super::readiness::module_status(s, &data.preflight.wallet.get(), &data.budget.get(), data.keyed.get(), data.clock.get()))
        .unwrap_or(("等待行情", "股票行情尚未就绪")))});
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
        <div class="stock-detail-panel" hidden=move ||data.section.get()==3>
        {move ||data.market.with(|m|m.value().and_then(|s|s.problem.clone())).map(|p|view!{<p role="status" class="stock-problem">{p}</p>})}
        {move ||(connected.get() && data.market.with(|m|m.value().is_some_and(|s|s.books.is_empty() && s.reference.is_none()))).then(||view!{<p role="status" class="stock-empty-inline">"WS 已连接，等待交易所推送该股票的行情。"</p>})}
        <dl class="stock-summary stock-context-summary"><div><dt>"证券身份"</dt><dd>{s.asset.clone()}</dd></div><div><dt>"询价金额 / USDC"</dt><dd>{move ||data.budget.get()}</dd></div>
            <div><dt>"链上报价"</dt><dd>{move ||if data.quote_pending.get() ||data.monitor_pending.get(){"正在更新询价"}else{data.market.with(|m|m.value().map(|s|super::readiness::quote_summary(s,&data.budget.get(),data.keyed.get(),data.clock.get())).unwrap_or("等待询价"))}}</dd></div><div><dt>"计划状态"</dt><dd>{move ||status.get().0}</dd></div></dl>
        <details class="stock-trading-route" hidden=move ||!matches!(data.section.get(),0|4|5)><summary>
            <strong>{move ||route.with(|r|match r.as_ref().filter(|r|r.valid_until_ms>data.clock.get()).map(|r|r.kind) {
                Some(StockRouteKind::Rfq)=>"当前通道 · 询价",Some(StockRouteKind::OrderBook)=>"当前通道 · 现货订单簿",
                Some(StockRouteKind::Closed)=>"当前休市",_=>"交易通道待核对",
            })}</strong>
            {move ||route.with(|r|r.as_ref().and_then(|r|r.session.clone())).map(|s|view!{<small>{format!("{} · 最少 {} 股 · 步长 {}",session_label(&s.name),s.min_quantity,s.step_size)}</small>})}
            </summary><p>{move ||route.with(|r|r.as_ref().map(|r|if r.kind!=StockRouteKind::Unknown && r.valid_until_ms<=data.clock.get(){"交易日历状态已陈旧，等待更新".into()}else{r.reason.clone()}).unwrap_or_else(||"正在读取官方日历".into()))}</p>
        </details>
        </div>
        <div class="stock-detail-panel" hidden=move ||!matches!(data.section.get(),0|1|5)>
            {comparison::controls(s.asset.clone(), data)}
        </div>
        <div class="stock-detail-panel" hidden=move ||data.section.get()!=0>
        {comparison::panel(s.asset.clone(), data)}
        <details class="stock-secondary"><summary>"盘口与参考行情"</summary>
        <section class="stock-section"><header><h3>"交易所买卖一档"</h3><span>"仅显示该市场 WS 盘口"</span></header>
            <div class="stock-table-scroll"><table class="stock-bbo"><thead><tr><th>"市场"</th><th>"买价"</th><th>"买量 / 股"</th><th>"卖价"</th><th>"卖量 / 股"</th><th>"时效"</th></tr></thead><tbody>{quote_rows}</tbody></table></div>
            {move ||security.with(|s|s.as_ref().is_none_or(|s|s.order_books.is_empty())).then(||view!{<p class="stock-empty-inline">"该证券没有股票订单簿市场，需通过 询价 获取可成交报价。"</p>})}
        </section>
        <section class="stock-section"><header><h3>"外部股票参考"</h3><span>"非订单簿 / 非 询价 成交承诺"</span></header>
            {move ||data.market.with(|m|m.value().and_then(|s|s.reference_problem.clone())).map(|p|view!{<p class="stock-empty-inline" role="status">{p}</p>})}
            {move ||ref_quote.get().is_none().then(||view!{<p class="stock-empty-inline" role="status">"外部参考源尚未返回报价"</p>})}
            <dl class="stock-summary"><div><dt>"参考买价"</dt><dd>{move ||ref_quote.with(|q|q.as_ref().and_then(|q|q.bid.clone()).unwrap_or_else(||"—".into()))}</dd></div>
                <div><dt>"参考卖价"</dt><dd>{move ||ref_quote.with(|q|q.as_ref().and_then(|q|q.ask.clone()).unwrap_or_else(||"—".into()))}</dd></div>
                <div><dt>"中间价"</dt><dd>{move ||ref_quote.with(|q|q.as_ref().map(|q|q.mid.clone()).unwrap_or_else(||"—".into()))}</dd></div>
                <div><dt>"来源时效"</dt><dd>{move ||quote_age(connected.get(),ref_quote.with(|q|q.as_ref().map(|q|q.source_at_ms)),data.clock.get())}</dd></div></dl>
        </section>
        </details>
        </div>
        <div class="stock-detail-panel" hidden=move ||data.section.get()!=5>
            {peers::panel(data)}
        </div>
        <div class="stock-detail-panel" hidden=move ||data.section.get()!=1>
            {preflight::panel(s.asset.clone(), data)}
        </div>
        <div class="stock-detail-panel" hidden=move ||data.section.get()!=2>
            {rfq::panel(data,credentials())}
        </div>
        <div class="stock-detail-panel" hidden=move ||data.section.get()!=4>
        <section class="stock-section"><header><h3>"链上合约与充提"</h3><span>"公共目录快照 · 执行前需重查"</span></header>
            <p class="stock-contract-meta">"CUSIP "<code>{s.cusip.clone().unwrap_or_else(||"官方未提供".into())}</code></p>
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
        </div>
        <footer class="stock-readiness" hidden=move ||data.section.get()==3><strong>{move ||status.get().0}</strong><span>{move ||status.get().1}</span>
            <button type="button" class="row-action" on:click=move |_|data.section.set(3)>"查看执行记录"</button>
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
