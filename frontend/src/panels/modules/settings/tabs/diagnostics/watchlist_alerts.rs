use super::*;
use crate::panels::modules::settings::data::SettingsWatchlistAlertState;
use crate::panels::modules::settings::tabs::problem_message;
use shared_types::{AlertRule, AlertRulesEnvelope, WatchlistEnvelope, WatchlistItem};

#[path = "watchlist_alerts/format.rs"]
mod format;
use format::{
    delivery_status_label, optional_number, optional_time, persistence_label, prewarm_status_class,
    prewarm_status_label, runtime_status_class, runtime_status_label, storage_summary,
    transport_problem_summary, transport_summary,
};

#[path = "watchlist_alerts/table.rs"]
mod table;
use table::{runtime_table, RuntimeCell, ALERT_HEADERS, WATCHLIST_HEADERS};

pub(super) fn watchlist_alert_runtime_panel(state: SettingsWatchlistAlertState) -> impl IntoView {
    view! {
        <div class="settings-stack">
            <div class="settings-summary-line">
                <strong>"自选与提醒运行状态"</strong>
                <span>{move || optional_transport_summary(state)}</span>
            </div>
            {move || optional_runtime_content(state)}
        </div>
    }
}

fn optional_transport_summary(state: SettingsWatchlistAlertState) -> String {
    match state.runtime.surface_available.get() {
        None if state.runtime.probe_problem.with(Option::is_some) => "检测失败，等待重试".into(),
        None => "正在检测可选功能".into(),
        Some(false) => "可选功能未启用".into(),
        Some(true) => transport_summary(
            &state.runtime.watchlist_channel.get(),
            &state.runtime.alerts_channel.get(),
        ),
    }
}

fn optional_runtime_content(state: SettingsWatchlistAlertState) -> AnyView {
    match state.runtime.surface_available.get() {
        None => match state.runtime.probe_problem.get() {
            None => view! { <div class="empty-cell">"正在检测自选与提醒功能"</div> }.into_any(),
            Some(problem) => {
                let backoff = problem.retry_after_ms.is_some_and(|ms| ms > 0);
                view! {
                    <div class="settings-stack" role="alert" aria-label="自选与提醒读取失败">
                        <p class="settings-message is-error">{problem_message("自选与提醒读取失败", &problem)}</p>
                        <div class="settings-actions">
                            <button type="button" class="row-action" aria-label="重新检测自选与提醒"
                                disabled=move || state.runtime.probe_reading.get() || backoff
                                on:click=move |_| state.runtime.retry_probe.run(())>
                                {move || if state.runtime.probe_reading.get() { "正在重新读取…" } else { "重新检测" }}
                            </button>
                            {backoff.then(|| view! { <span>"将按后台要求自动重试"</span> })}
                        </div>
                    </div>
                }.into_any()
            }
        },
        Some(false) => {
            view! { <div class="empty-cell">"自选与提醒为可选功能，当前未启用"</div> }.into_any()
        }
        Some(true) => view! {
            <>
                {transport_problem_summary(
                    &state.runtime.watchlist_channel.get(),
                    &state.runtime.alerts_channel.get(),
                ).map(|message| view! {
                    <em class="settings-message is-error">{message}</em>
                })}
                {watchlist_table(state.watchlist.get())}
                {alert_rules_table(state.alert_rules.get())}
                <div class="settings-summary-line">
                    <strong>"最近提醒"</strong>
                    <span>{last_notification_label(state)}</span>
                </div>
            </>
        }
        .into_any(),
    }
}

fn watchlist_table(state: LoadState<WatchlistEnvelope>) -> AnyView {
    let (envelope, problem) = match state {
        LoadState::Ready(envelope) => (envelope, None),
        LoadState::Stale { value, problem } => (value, Some(problem)),
        LoadState::Error(problem) => return problem_cell("读取自选运行状态失败", &problem),
        LoadState::Loading => {
            return view! { <div class="empty-cell">"正在读取自选运行状态"</div> }.into_any();
        }
    };
    let summary = format!(
        "{} 项 · 每家交易所最多 {} 个行情品种 · {} · {}",
        envelope.items.len(),
        envelope.runtime.public_ticker_symbols_per_venue_limit,
        if envelope.runtime.volatile {
            "重启后丢失"
        } else {
            "重启后保留"
        },
        storage_summary(&envelope.runtime.storage),
    );
    let rows = envelope.items.into_iter().map(watchlist_row).collect();
    runtime_table(
        "自选行情",
        summary,
        problem,
        "自选刷新失败",
        WATCHLIST_HEADERS,
        rows,
    )
}

fn watchlist_row(item: WatchlistItem) -> Vec<RuntimeCell> {
    let persistence = persistence_label(&item.persistence);
    let threshold = format!(
        "最低净收益 {} · 最低日成交量 {}",
        optional_number(item.min_net_yield),
        optional_number(item.min_volume_24h),
    );
    let runtime = item.runtime;
    let runtime_resource = format!(
        "申请 {} · 已安排 {} · 去重 {} · 超限 {}",
        runtime.requested_public_legs,
        runtime.planned_ticker_legs,
        runtime.deduplicated_legs,
        runtime.capped_legs,
    );
    let problem = runtime
        .problem
        .as_ref()
        .map(|problem| problem_message("行情准备异常", problem))
        .unwrap_or_else(|| "-".into());
    vec![
        RuntimeCell::text(item.id.to_string()),
        RuntimeCell::text(item.symbol),
        RuntimeCell::text(item.venue_long.unwrap_or_else(|| "-".into())),
        RuntimeCell::text(item.venue_short.unwrap_or_else(|| "-".into())),
        RuntimeCell::text(threshold),
        RuntimeCell::text(persistence),
        RuntimeCell::status(
            prewarm_status_label(runtime.status),
            prewarm_status_class(runtime.status),
        ),
        RuntimeCell::text(runtime_resource),
        RuntimeCell::text(problem),
    ]
}

fn alert_rules_table(state: LoadState<AlertRulesEnvelope>) -> AnyView {
    let (envelope, problem) = match state {
        LoadState::Ready(envelope) => (envelope, None),
        LoadState::Stale { value, problem } => (value, Some(problem)),
        LoadState::Error(problem) => return problem_cell("读取提醒规则失败", &problem),
        LoadState::Loading => {
            return view! { <div class="empty-cell">"正在读取提醒规则"</div> }.into_any();
        }
    };
    let count = envelope.rules.len();
    let rows = envelope.rules.into_iter().map(alert_rule_row).collect();
    runtime_table(
        "提醒规则",
        format!("{count} 条 · 不订阅私有账户数据"),
        problem,
        "提醒刷新失败",
        ALERT_HEADERS,
        rows,
    )
}

fn alert_rule_row(rule: AlertRule) -> Vec<RuntimeCell> {
    let persistence = persistence_label(&rule.persistence);
    let delivery = rule.delivery;
    let runtime = rule.runtime;
    let status = runtime_status_label(runtime.status);
    let status_class = runtime_status_class(runtime.status);
    let resource = format!(
        "公开行情 {} 项 · 私有账户行情 {} 项",
        runtime.watchlist_public_prewarm_legs, runtime.private_ws_symbols,
    );
    let timing = format!(
        "下次 {} · 上次触发 {} · 已触发 {} 次 · {}",
        optional_time(runtime.next_eligible_at_ms),
        optional_time(delivery.last_fired_at_ms),
        runtime.trigger_count,
        delivery_status_label(delivery.last_delivery_status),
    );
    let problem = runtime
        .problem
        .as_ref()
        .map(|problem| problem_message("运行异常", problem))
        .unwrap_or_else(|| "-".into());
    vec![
        RuntimeCell::text(rule.id.to_string()),
        RuntimeCell::text(rule.watchlist_id.to_string()),
        RuntimeCell::text(runtime.transport),
        RuntimeCell::text(persistence),
        RuntimeCell::status(status, status_class),
        RuntimeCell::text(resource),
        RuntimeCell::text(timing),
        RuntimeCell::text(problem),
    ]
}

fn last_notification_label(state: SettingsWatchlistAlertState) -> String {
    state.runtime.last_notification.get().map_or_else(
        || "暂无排队中的提醒".into(),
        |notification| {
            format!(
                "{} · {} / {} · 单次费后 {:+.3}%",
                notification.symbol,
                notification.long_exchange,
                notification.short_exchange,
                notification.one_cycle_net_bps / 100.0,
            )
        },
    )
}

#[cfg(test)]
#[path = "watchlist_alerts/tests.rs"]
mod tests;
