use super::*;
use shared_types::{WebhookApplicationAck, WebhookDeliveryStatus};

pub(super) fn panel(asset: String, data: StockData) -> impl IntoView {
    let status = Memo::new(move |_| {
        data.market
            .with(|m| m.value().map(|s| s.alerts.clone()).unwrap_or_default())
    });
    let running = Memo::new(move |_| {
        data.market
            .with(|m| m.value().is_some_and(|s| s.monitor.enabled))
    });
    let dirty = Memo::new(move |_| {
        let saved = data.market.with(|m| {
            m.value()
                .map(|s| s.monitor.alerts.clone())
                .unwrap_or_default()
        });
        data.alerts.enabled.get() != saved.enabled
            || data.alerts.include_peer.get() != saved.include_peer
            || data.alerts.threshold.get() != saved.min_spread_pct
            || data.alerts.cooldown.get() != saved.cooldown_secs.to_string()
    });
    let toggle_asset = asset.clone();
    view! { <div class="stock-alerts">
        <header><h4>"股票价差 Webhook"</h4><a href="#settings">"投递设置"</a></header>
        <div class="stock-monitor-control">
            <label><input type="checkbox" aria-label="股票价差提醒" checked=move ||data.alerts.enabled.get() prop:checked=move ||data.alerts.enabled.get()
                disabled=move ||data.monitor_pending.get() || data.pending.get()
                on:change=move |ev| {
                    data.alerts.enabled.set(event_target_checked(&ev));
                    if running.get() { data.monitor.run((toggle_asset.clone(),true)); }
                }/><span>"价差提醒"</span></label>
            <span role="status">{move ||if !running.get() && data.alerts.enabled.get() {"持续询价未运行"}else if dirty.get(){"修改未应用"}else{phase(status.with(|s|s.phase))}}</span>
            <span>"仅观察 · 完整净收益未核齐"</span>
        </div>
        <div class="stock-monitor-control">
            <label><input type="checkbox" aria-label="同时监控所选交易所" checked=move ||data.alerts.include_peer.get() prop:checked=move ||data.alerts.include_peer.get()
                disabled=move ||data.monitor_pending.get() ||data.pending.get()
                on:change=move |ev|data.alerts.include_peer.set(event_target_checked(&ev))/><span>"同时监控所选交易所"</span></label>
            <span>{move ||data.market.with(|m|m.value().and_then(|s|s.peer.as_ref()).map(|p|format!("{} · {}",p.selection.venue,p.selection.native_symbol)).unwrap_or_else(||"未选择对比市场".into()))}</span>
        </div>
        <form class="stock-quote-form" on:submit=move |ev|{ev.prevent_default();data.monitor.run((asset.clone(),true));}>
            <label><span>"报价差额 ≥ / %"</span><input type="text" inputmode="decimal" autocomplete="off" aria-label="股票报价差额阈值百分比"
                disabled=move ||data.monitor_pending.get() || data.pending.get()
                value=move ||data.alerts.threshold.get() prop:value=move ||data.alerts.threshold.get()
                on:input=move |ev|data.alerts.threshold.set(event_target_value(&ev))/></label>
            <label><span>"最短提醒间隔 / 秒"</span><input type="number" min="10" max="3600" step="1" aria-label="股票提醒间隔秒"
                disabled=move ||data.monitor_pending.get() || data.pending.get()
                value=move ||data.alerts.cooldown.get() prop:value=move ||data.alerts.cooldown.get()
                on:input=move |ev|data.alerts.cooldown.set(event_target_value(&ev))/></label>
            <button class="row-action" type="submit" disabled=move ||!running.get() || data.monitor_pending.get() || data.pending.get()>
                {move ||if data.monitor_pending.get(){"保存中…"}else{"应用提醒"}}</button>
        </form>
        {move ||status.with(|s|s.problem.clone()).map(|p|view!{<p class="stock-problem" role="status">{p}</p>})}
        <div class="stock-alert-history" aria-label="股票提醒记录">
            {move ||status.with(|s|s.recent.clone()).into_iter().map(|row| {
                let result=delivery_label(&row);
                view! {<div class="stock-alert-record"><strong>{row.direction}</strong>
                    <span>{comparison::quantity(Some(row.spread_pct))}"% · "{comparison::quantity(Some(row.gross_usdc))}" USDC"</span>
                    <span>{result}</span><small>{move ||format!("{}s 前入队",data.clock.get().saturating_sub(row.queued_at_ms).max(0)/1000)}</small>
                </div>}
            }).collect_view()}
        </div>
    </div> }
}

fn phase(p: StockAlertPhase) -> &'static str {
    match p {
        StockAlertPhase::Disabled => "提醒已关闭",
        StockAlertPhase::NeedsWebhook => "等待投递配置",
        StockAlertPhase::WaitingQuotes => "等待有效双边报价",
        StockAlertPhase::Watching => "等待达到阈值",
        StockAlertPhase::Cooldown => "提醒冷却中",
        StockAlertPhase::Queued => "提醒已入队",
        StockAlertPhase::Degraded => "提醒待恢复",
    }
}

fn delivery_label(row: &StockAlertSummary) -> String {
    let Some(d) = row.delivery.as_ref() else {
        return "已入队 · 尚未确认投递".into();
    };
    match d.status {
        WebhookDeliveryStatus::Delivered
            if d.application_ack == WebhookApplicationAck::Accepted =>
        {
            "推送服务已确认".into()
        }
        WebhookDeliveryStatus::Delivered => "HTTP 已送达 · 无应用确认".into(),
        WebhookDeliveryStatus::Failed | WebhookDeliveryStatus::Dropped => {
            format!("投递失败 · 尝试 {} 次", d.attempts)
        }
        WebhookDeliveryStatus::Disabled => "投递已停用".into(),
        WebhookDeliveryStatus::Queued => "等待投递".into(),
    }
}
