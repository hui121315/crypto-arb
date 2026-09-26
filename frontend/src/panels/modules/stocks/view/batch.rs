use super::*;
use crate::state::load_state::LoadState;

pub(super) fn panel(data: StockData) -> impl IntoView {
    let search = RwSignal::new(String::new());
    let state = Memo::new(move |_| {
        data.market
            .with(|m| m.value().map(|s| s.batch.clone()).unwrap_or_default())
    });
    let enabled = move || state.with(|s| s.request.as_ref().is_some_and(|r| r.enabled));
    let coverage = Memo::new(move |_| {
        let now = data.clock.get();
        state.with(|s| s.request.as_ref().map(|request| {
            let fresh = s.rows.iter().filter(|row| request.assets.contains(&row.security.asset)
                && row.token_price(true, now).is_some() && row.token_price(false, now).is_some()).count();
            (fresh, request.assets.len())
        }))
    });
    let changed = move || {
        state.with(|s| {
            s.request.as_ref().is_some_and(|r| {
                r.assets != data.batch.selected.get()
                    || r.budget_usdc != data.batch.budget.get()
                    || r.keyed != data.batch.keyed.get()
                    || r.interval_secs != data.batch.interval.get()
            })
        })
    };
    let rows = Memo::new(move |_| {
        let selected = state
            .with(|s| {
                s.request
                    .as_ref()
                    .filter(|r| r.enabled)
                    .map(|r| r.assets.clone())
            })
            .unwrap_or_else(|| data.batch.selected.get());
        data.catalog.with(|c| {
            c.value()
                .map(|c| {
                    c.rows
                        .iter()
                        .filter(|s| selected.contains(&s.asset))
                        .cloned()
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        })
    });
    let visible_rows = Memo::new(move |_| {
        let query = search.get().trim().to_lowercase();
        rows.with(|rows| rows.iter().filter(|s| query.is_empty()
            || s.ticker.to_lowercase().contains(&query)
            || s.name.to_lowercase().contains(&query)
            || s.asset.to_lowercase().contains(&query)).cloned().collect::<Vec<_>>())
    });
    view! {<section class="stock-batch" aria-label="批量链上监控">
        <div class="stock-monitor-overview" aria-label="股票监控摘要">
            <dl class="stock-monitor-fact"><dt>{move ||if enabled(){"运行范围"}else{"待监控股票"}}</dt>
                <dd><strong>{move ||if enabled(){state.with(|s|s.request.as_ref().map_or(0,|r|r.assets.len()))}else{data.batch.selected.with(Vec::len)}}</strong><span>"只股票"</span></dd>
            </dl>
            <dl class="stock-monitor-fact"><dt>"链上报价覆盖"</dt><dd>
                <span class="stock-batch-coverage" aria-label="双向新鲜报价"
                    class:is-warning=move ||enabled() && coverage.get().is_some_and(|(fresh,total)|fresh<total)>
                    {move ||coverage.get().map(|(fresh,total)|format!("双向新鲜 {fresh}/{total}")).unwrap_or_else(||"待开始".into())}
                </span>
            </dd></dl>
            <dl class="stock-monitor-fact stock-batch-status"><dt>"轮询状态"</dt><dd><span class="stock-batch-state" role="status"
                class:is-warning=move ||enabled() && state.with(|s|s.problem.is_some())
                class:is-muted=move ||!enabled() ||state.with(|s|s.waiting_for_viewers)
            >{move ||if data.market.with(|s|s.value().is_none()) {"状态待确认".into()} else {state.with(|s|if s.request.is_none(){"未开始".into()}else if !s.request.as_ref().is_some_and(|r|r.enabled){"已暂停".into()}else if s.waiting_for_viewers{"无人查看 · 暂停请求".into()}else if s.running{format!("本轮更新 {}/{}",s.rows.iter().filter(|r|!r.refreshing).count(),s.request.as_ref().map_or(0,|r|r.assets.len()))}else if s.problem.is_some(){format!("等待重试 · {}s",s.next_at_ms.unwrap_or(data.clock.get()).saturating_sub(data.clock.get()).max(0).saturating_add(999)/1000)}else{format!("监控中 · {} 轮",s.completed_rounds)})}}</span>
            </dd></dl>
            <dl class="stock-batch-timing" aria-label="批量轮询时效">
                <div><dt>{move ||if state.with(|s|s.running){"本轮已耗时"}else{"上轮耗时"}}</dt><dd>{move ||state.with(|s|{
                    let elapsed=if s.running{s.round_started_at_ms.filter(|at|*at<=data.clock.get()).map(|at|data.clock.get().saturating_sub(at) as u64)}else{s.last_round_elapsed_ms};
                    elapsed.map(duration).unwrap_or_else(||"—".into())
                })}</dd></div>
                <div><dt>"下轮更新"</dt><dd>{move ||state.with(|s|{
                    if !s.request.as_ref().is_some_and(|r|r.enabled){return "未运行".into();}
                    if s.waiting_for_viewers{return "等待查看".into();}
                    if s.running{return "本轮结束后".into();}
                    s.next_at_ms.map(|at|if at<=data.clock.get(){"等待调度".into()}else{format!("{} 后",duration(at.saturating_sub(data.clock.get()) as u64))}).unwrap_or_else(||"—".into())
                })}</dd></div>
            </dl>
        </div>
        <div id="stock-batch-content">
        <header class="stock-batch-heading"><h2>"监控列表"</h2>
            <span class="stock-batch-result-count">{move ||format!("{} / {} 只",visible_rows.with(Vec::len),rows.with(Vec::len))}</span>
            <div class="stock-batch-search">
                <input type="search" aria-label="搜索监控股票" placeholder="搜索股票 / 名称"
                    prop:value=move ||search.get() on:input=move |ev|search.set(event_target_value(&ev))/>
            </div>
        </header>
        <form class="stock-batch-toolbar" on:submit=move |ev|{ev.prevent_default();data.batch.apply.run(true);}>
            <label><span>"每只询价 / USDC"</span><input aria-label="批量询价金额" inputmode="decimal" disabled=move ||data.batch.pending.get() prop:value=move ||data.batch.budget.get() on:input=move |ev|data.batch.budget.set(event_target_value(&ev))/></label>
            <label><span>"轮后间隔"</span><select aria-label="批量更新间隔" disabled=move ||data.batch.pending.get() prop:value=move ||data.batch.interval.get().to_string() on:change=move |ev|{if let Ok(n)=event_target_value(&ev).parse(){data.batch.interval.set(n);}}>
                <option value="5" selected=move ||data.batch.interval.get()==5>"5 秒"</option><option value="15" selected=move ||data.batch.interval.get()==15>"15 秒"</option><option value="30" selected=move ||data.batch.interval.get()==30>"30 秒"</option><option value="60" selected=move ||data.batch.interval.get()==60>"60 秒"</option>
            </select></label>
            <label><span>"报价源"</span><select aria-label="批量报价源" disabled=move ||data.batch.pending.get() prop:value=move ||if data.batch.keyed.get(){"keyed"}else{"public"} on:change=move |ev|data.batch.keyed.set(event_target_value(&ev)=="keyed")>
                <option value="public" selected=move ||!data.batch.keyed.get()>"Jupiter · 公共"</option><option value="keyed" selected=move ||data.batch.keyed.get()>"Jupiter · API Key"</option>
            </select></label>
            <div class="stock-batch-controls"><span>{move ||format!("已选 {} / {}",data.batch.selected.with(Vec::len),STOCK_BATCH_LIMIT)}</span>
                <button type="submit" class="row-action stock-batch-primary" disabled=move ||data.batch.pending.get() ||data.batch.conflict.get() ||data.batch.selected.with(Vec::is_empty) ||(enabled()&&!changed())>{move ||if data.batch.journal.busy.get(){"保存中…"}else if data.batch.journal.locked(){"等待核对"}else if data.batch.pending.get(){"等待读取"}else if enabled(){"应用参数"}else{"开始批量轮询"}}</button>
                {move ||enabled().then(||view!{<button type="button" class="row-action" disabled=move ||data.batch.pending.get() on:click=move |_|data.batch.apply.run(false)>"暂停"</button>})}
            </div>
        </form>
        {move ||(data.batch.conflict.get() && !data.batch.journal.locked()).then(||view!{
            <div class="stock-problem stock-batch-conflict" role="alert" aria-label="批量参数冲突">
                <strong>"后台参数已更新，本地草稿尚未应用"</strong>
                <span>{move ||state.with(|s|s.request.as_ref().map(|r|format!("后台：{} · {} USDC / 只 · {} 秒 · {} · {}",
                    if r.enabled{"监控已开启"}else{"已暂停"},r.budget_usdc,r.interval_secs,
                    if r.keyed{"API Key"}else{"公共报价"},r.assets.join("、")))
                    .unwrap_or_else(||"后台尚未启用批量监控，可能已重启。".into()))}</span>
                <div><button type="button" class="row-action" disabled=move ||data.batch.pending.get()
                    on:click=move |_|data.batch.reconcile.run(false)>"载入后台参数"</button>
                    <button type="button" class="row-action" disabled=move ||data.batch.pending.get()
                    on:click=move |_|data.batch.reconcile.run(true)>"保留草稿待应用"</button></div>
            </div>
        })}
        {move ||data.market.with(|s|s.value().is_some_and(|s|s.batch.revision.is_empty())).then(||view!{
            <div class="stock-problem stock-batch-conflict" role="alert"><span>"后台未提供配置版本，暂不能保存。请更新后台后重读监控状态。"</span>
                <div><button type="button" class="row-action" on:click=move |_|data.batch.refresh.run(())>"重读监控状态"</button></div></div>
        })}
        {crate::panels::shared::operation_journal::settings_recovery_panel(data.batch.journal, data.batch.recheck)}
        {move ||data.batch.storage_problem.get().map(|problem| view!{<div class="stock-problem" role="alert">
            <span>{problem}</span><button type="button" class="row-action" on:click=move |_|data.batch.reset_draft.run(())>"清除本地草稿"</button>
        </div>})}
        {move ||data.market.with(|state|matches!(state, LoadState::Error(_) | LoadState::Stale { .. })).then(||view!{
            <div class="stock-problem" role="alert"><span>"当前监控状态读取异常，保留已知数据"</span>
                <button type="button" class="row-action" on:click=move |_|data.batch.refresh.run(())>"重读监控状态"</button></div>
        })}
        {move ||data.batch.problem.get().map(|p|view!{<p class="stock-problem stock-batch-save-problem" role="alert">{p}</p>})}
        {move ||(changed() && !data.batch.conflict.get()).then(||view!{<p class="stock-empty-inline" role="status">{if enabled(){"有未应用的更改 · 当前行情仍使用已保存参数"}else{"有未应用的更改 · 后台监控仍已暂停"}}</p>})}
        {move ||state.with(|s|s.problem.clone()).map(|p|view!{<p class="stock-problem" role="alert">{p}</p>})}
        <div class="stock-batch-table-wrap"><table class="clean-table stock-batch-table" aria-label="股票监控报价">
        <colgroup><col class="stock-col-security"/><col class="stock-col-price"/><col class="stock-col-price"/><col class="stock-col-book"/><col class="stock-col-transfer"/><col class="stock-col-status"/><col class="stock-col-action"/></colgroup>
        <thead><tr>
            <th scope="col">"股票"</th><th scope="col">"链买上限"<small>"USDC / Token"</small></th><th scope="col">"链卖下限"<small>"USDC / Token"</small></th>
            <th scope="col"><div class="stock-book-prices"><span>"BP 买价"</span><span>"BP 卖价"</span></div><small>"USDC / 股 · 仅盘口"</small></th>
            <th scope="col">"充 / 提"</th><th scope="col">"报价状态"</th><th scope="col">"操作"</th>
        </tr></thead><tbody><For each=move ||visible_rows.get() key=|s|s.asset.clone() children=move |security|{
            let asset=security.asset.clone();let active=asset.clone();let row_asset=asset.clone();
            let row=Memo::new(move |_|state.with(|b|b.rows.iter().find(|r|r.security.asset==row_asset).cloned()));
            let quote_status=move ||row.with(|r|match r{
                None=>"等待开始".into(),Some(r) if r.refreshing=>"排队 / 更新中".into(),
                Some(r) if r.problem.is_some() && r.buy.is_some() && r.sell.is_some()=>"双向时效未齐".into(),
                Some(r) if r.problem.is_some()=>if r.token_price(true,data.clock.get()).is_some(){"缺链卖报价".into()}
                    else if r.token_price(false,data.clock.get()).is_some(){"缺链买报价".into()}else{"暂无双向报价".into()},
                Some(r) if r.token_price(true,data.clock.get()).is_some() &&r.token_price(false,data.clock.get()).is_some()=>format!("{}s 前",data.clock.get().saturating_sub(r.checked_at_ms.unwrap_or(0))/1000),
                Some(_)=>"报价已过期".into(),
            });
            view!{<tr aria-selected=move ||data.market.with(|m|m.value().and_then(|s|s.security.as_ref()).is_some_and(|s|s.asset==active)).to_string()>
                <td class="stock-security-cell"><strong>{security.ticker}</strong><small>{security.name}</small></td>
                <td data-label="链买上限 · USDC / Token">{move ||comparison::quantity(row.with(|r|r.as_ref().and_then(|r|r.token_price(true,data.clock.get()))))}</td>
                <td data-label="链卖下限 · USDC / Token">{move ||comparison::quantity(row.with(|r|r.as_ref().and_then(|r|r.token_price(false,data.clock.get()))))}</td>
                <td data-label="BP 买 / 卖 · USDC / 股">{move ||row.with(|r|match book_prices(r.as_ref(),data.clock.get()) {
                    Ok((bid,ask))=>view!{<div class="stock-book-prices"><span class="stock-book-bid">{bid}</span><span class="stock-book-ask">{ask}</span></div>}.into_any(),
                    Err(status)=>view!{<span class="stock-book-status">{status}</span>}.into_any(),
                })}</td>
                <td data-label="充 / 提">{move ||row.with(|r|r.as_ref().and_then(|r|r.token.as_ref()).map(|t|format!("{} / {}",flag(t.deposit_enabled),flag(t.withdraw_enabled))).unwrap_or_else(||"待读取".into()))}</td>
                <td class="stock-quote-status-cell" data-label="报价状态"><span class="stock-row-quote-state"
                    class:is-current=move ||row.with(|r|r.as_ref().is_some_and(|r|!r.refreshing && r.problem.is_none() && r.token_price(true,data.clock.get()).is_some() && r.token_price(false,data.clock.get()).is_some()))
                    >{quote_status}</span>{move ||row.with(|r|r.as_ref().map(|r|if r.issuer_verified{"发行映射已匹配"}else if r.token.is_some(){"仅官方合约映射"}else{"合约映射待核实"})).map(|s|view!{<small>{s}</small>})}
                    {move ||row.with(|r|r.as_ref().and_then(|r|r.problem.clone())).map(|p|view!{<details class="stock-batch-issue"><summary>"原因"</summary><p>{p}</p></details>})}
                </td><td><button type="button" class="row-action" disabled=move ||data.pending.get() on:click=move |_|{data.watch.run(Some(asset.clone()));data.section.set(0);}>"查看"</button></td>
            </tr>}
        }/></tbody></table></div>
        {move ||rows.with(Vec::is_empty).then(||view!{<p class="stock-empty-inline">"尚未选择监控股票"</p>})}
        {move ||(!rows.with(Vec::is_empty) && visible_rows.with(Vec::is_empty)).then(||view!{<p class="stock-empty-inline" role="status">"没有匹配的监控股票"</p>})}
        <footer class="stock-batch-footer"><span>"只读询价 · 未扣齐成本"</span>
            <details><summary>"计价与时效"</summary><p>"Token 与股票份额分开计量；刷新周期受 报价服务配额与响应时间影响。"</p></details>
        </footer>
        </div>
    </section>}
}

fn duration(ms: u64) -> String {
    let seconds = ms / 1_000;
    if seconds < 60 { format!("{seconds}s") } else { format!("{}m {:02}s", seconds / 60, seconds % 60) }
}

fn book_prices(row: Option<&StockBatchRow>, now: i64) -> Result<(String, String), &'static str> {
    let Some(row) = row else {
        return Err("待读取");
    };
    if row.security.order_books.is_empty() {
        return Err("仅 询价");
    }
    if !row.connected {
        return Err("WS 未连接");
    }
    row.books
        .iter()
        .find(|b| {
            row.security
                .order_books
                .iter()
                .any(|m| m.symbol == b.symbol && m.quote == "USDC")
                && now >= b.source_at_ms
                && now - b.source_at_ms <= 3000
        })
        .map(|b| (b.bid.clone().unwrap_or_else(||"—".into()), b.ask.clone().unwrap_or_else(||"—".into())))
        .ok_or("暂无新鲜盘口")
}
