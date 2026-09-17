use super::*;
use shared_types::stocks::comparison::{evaluate, quote_current};

pub(super) fn panel(asset: String, data: StockData) -> impl IntoView {
    let identity_problem = Memo::new(move |_| data.market.with(|m| {
        m.value().and_then(|s| identity::backpack_token_identity(s).err().map(str::to_owned))
    }));
    let monitor = Memo::new(move |_| {
        data.market
            .with(|m| m.value().map(|s| s.monitor.clone()).unwrap_or_default())
    });
    let monitor_asset = asset.clone();
    let alert_asset = asset.clone();
    let quote = Memo::new(move |_| {
        data.market
            .with(|m| m.value().and_then(|s| s.comparison.clone()))
    });
    let estimates = Memo::new(move |_| {
        data.market.with(|m| {
            m.value()
                .map(|m| evaluate(m, data.clock.get()))
                .unwrap_or_default()
        })
    });
    view! { <section class="stock-section stock-comparison">
        <header><h3>"链上 / Backpack"</h3><span>"报价差额 · 未扣齐成本"</span></header>
        {move ||identity_problem.get().map(|p|view!{<p class="stock-empty-inline stock-identity-problem" role="status">{p}</p>})}
        <form class="stock-quote-form" on:submit=move |ev| {ev.prevent_default(); if identity_problem.get().is_some() {return;} if monitor.with(|m|m.enabled) {data.monitor.run((asset.clone(),true));}else{data.quote.run(asset.clone());}}>
            <label><span>"链买预算 / USDC"</span><input type="text" inputmode="decimal" autocomplete="off" aria-label="链买预算 USDC"
                value=move ||data.budget.get() prop:value=move ||data.budget.get() on:input=move |ev|data.budget.set(event_target_value(&ev))/></label>
            <label><span>"Jupiter 接入"</span><select aria-label="Jupiter 接入" prop:value=move || if data.keyed.get(){"keyed"}else{"public"}
                on:change=move |ev|data.keyed.set(event_target_value(&ev)=="keyed")><option value="public">"公共询价"</option><option value="keyed">"已配置 API Key"</option></select></label>
            <button type="submit" class="row-action" disabled=move || identity_problem.get().is_some() || data.quote_pending.get() || data.pending.get() || data.monitor_pending.get()>
                {move ||if data.quote_pending.get() || data.monitor_pending.get(){"处理中…"}else if monitor.with(|m|m.enabled){"应用监控参数"}else{"更新询价"}}</button>
        </form>
        <div class="stock-monitor-control">
            <label><input type="checkbox" aria-label="持续询价" prop:checked=move ||monitor.with(|m|m.enabled)
                disabled=move ||data.monitor_pending.get() || data.pending.get() || data.quote_pending.get() || (identity_problem.get().is_some() && !monitor.with(|m|m.enabled))
                on:change=move |ev| {
                    let enabled=event_target_checked(&ev);
                    event_target::<web_sys::HtmlInputElement>(&ev).set_checked(monitor.with(|m|m.enabled));
                    data.monitor.run((monitor_asset.clone(),enabled));
                }/><span>"持续询价"</span></label>
            <span role="status">{move ||phase_label(monitor.with(|m|m.phase))}</span>
            <span>{move ||monitor.with(|m| if m.enabled {format!("已更新 {} 次",m.completed_quotes)}else{String::new()})}</span>
            <span>{move ||monitor.with(|m|m.next_attempt_at_ms.filter(|t|*t>data.clock.get()).map(|t|format!("{}s 后{}",(t-data.clock.get()+999)/1000,if m.phase==StockMonitorPhase::QuantityLimited{"重查数量限制"}else{"更新"})).unwrap_or_default())}</span>
        </div>
        {move ||monitor.with(|m|m.problem.clone()).map(|p|view!{<p class="stock-empty-inline" role="status">{p}</p>})}
        {super::alerts::panel(alert_asset, data)}
        {move || quote.get().map(|q| {
            let buy_fee = q.buy.fee_bps.map(|b|format!("{}.{:02}%",b/100,b%100)).unwrap_or_else(||"未提供".into());
            let sell_fee = q.sell.as_ref().and_then(|q|q.fee_bps).map(|b|format!("{}.{:02}%",b/100,b%100)).unwrap_or_else(||"未提供".into());
            view!{<div class="stock-quote-meta"><span>{format!("本次 {} USDC · {}",q.budget_usdc,if q.keyed{"API Key"}else{"公共询价"})}</span>
                <a href=q.issuer_docs target="_blank" rel="noopener noreferrer">"发行方 1:1 兑换资料"</a>
                <span>{format!("股数倍率 {} · Mint {} 位",q.mint.ui_multiplier,q.mint.decimals)}</span>
                <span>{format!("Jupiter 费率 买 {buy_fee} / 卖 {sell_fee}")}</span>
                <span>{move ||if quote_current(&q.buy,data.clock.get()) {format!("链买询价 {}ms",data.clock.get().saturating_sub(q.buy.requested_at_ms))} else {"链买询价已陈旧".into()}}</span>
            </div>}
        })}
        <div class="stock-directions">
            {move ||estimates.get().into_iter().map(|row|view!{<article class="stock-direction">
                <header><h4>{row.direction}</h4><span>"仅观察"</span></header>
                <dl class="stock-direction-values"><div><dt>"报价差额 / USDC"</dt><dd>{row.gross_usdc.unwrap_or_else(||"—".into())}</dd></div>
                    <div><dt>"交易所数量 / 股"</dt><dd>{quantity(row.shares)}</dd></div>
                    <div><dt>{if row.direction.starts_with("链买"){"链上最低到账 / 股"}else{"链上最低到账 / USDC"}}</dt><dd>{quantity(row.minimum_output)}</dd></div>
                    <div><dt>"余量 / 股 · 不计盈利"</dt><dd>{quantity(row.remainder_shares)}</dd></div></dl>
                <ul>{row.blockers.into_iter().map(|b|view!{<li>{b}</li>}).collect_view()}</ul>
            </article>}).collect_view()}
        </div>
        {move ||quote.get().is_none().then(||view!{<p class="stock-empty-inline">"尚无链上报价；身份与股数关系未核实的证券暂不计算差额。"</p>})}
    </section> }
}

fn phase_label(phase: StockMonitorPhase) -> &'static str {
    match phase {
        StockMonitorPhase::Disabled => "持续询价已关闭",
        StockMonitorPhase::WaitingForViewers => "无人查看 · 已暂停",
        StockMonitorPhase::Refreshing => "正在询价",
        StockMonitorPhase::Watching => "监控中",
        StockMonitorPhase::QuantityLimited => "金额与当前时段不匹配",
        StockMonitorPhase::Backoff => "等待重试",
    }
}

pub(super) fn quantity(raw: Option<String>) -> impl IntoView {
    let exact = raw.unwrap_or_else(|| "—".into());
    // Display only: all sizing and comparison arithmetic remains Decimal in shared-types.
    let display = if exact
        .split_once('.')
        .is_some_and(|(_, fraction)| fraction.len() > 8)
    {
        match exact
            .parse::<f64>()
            .ok()
            .filter(|n| n.is_finite())
        {
            Some(n) if n > 0.0 && n < 0.00000001 => "<0.00000001".into(),
            Some(n) if n < 0.0 && n > -0.00000001 => ">-0.00000001".into(),
            Some(n) => format!("≈{n:.8}"),
            None => exact.clone(),
        }
    } else {
        exact.clone()
    };
    view! { <span title=exact>{display}</span> }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stock_quantity_display_preserves_exact_value_without_zeroing_dust() {
        let html = quantity(Some("0.0108151556350602000338".into())).to_html();
        assert!(html.contains("≈0.01081516"));
        assert!(html.contains("title=\"0.0108151556350602000338\""));
        assert!(quantity(Some("0.0000000001".into()))
            .to_html()
            .contains("&lt;0.00000001"));
        let negative = quantity(Some("-0.0108151556350602000338".into())).to_html();
        assert!(negative.contains("≈-0.01081516"));
        assert!(negative.contains("title=\"-0.0108151556350602000338\""));
        assert!(quantity(Some("-0.0000000001".into()))
            .to_html()
            .contains("&gt;-0.00000001"));
    }
}
