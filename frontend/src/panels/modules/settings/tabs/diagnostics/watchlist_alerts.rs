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
                <strong>"自选与提醒运行态"</strong>
                <span>{move || optional_transport_summary(state)}</span>
            </div>
            {move || optional_runtime_content(state)}
        </div>
    }
}

fn optional_transport_summary(state: SettingsWatchlistAlertState) -> String {
    match state.runtime.surface_available.get() {
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
        None => view! { <div class="empty-cell">"正在检测自选与提醒功能"</div> }.into_any(),
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
        LoadState::Error(problem) => return problem_cell("读取自选运行态失败", &problem),
        LoadState::Loading => {
            return view! { <div class="empty-cell">"正在读取自选运行态"</div> }.into_any();
        }
    };
    let summary = format!(
        "{} 项 · ticker {}/venue · {} · {}",
        envelope.items.len(),
        envelope.runtime.public_ticker_symbols_per_venue_limit,
        if envelope.runtime.volatile {
            "volatile"
        } else {
            "durable"
        },
        storage_summary(&envelope.runtime.storage),
    );
    let rows = envelope.items.into_iter().map(watchlist_row).collect();
    runtime_table(
        "Watchlist",
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
        "yield {} · volume {}",
        optional_number(item.min_net_yield),
        optional_number(item.min_volume_24h),
    );
    let runtime = item.runtime;
    let runtime_resource = format!(
        "req {} · ticker {} · dedup {} · capped {}",
        runtime.requested_public_legs,
        runtime.planned_ticker_legs,
        runtime.deduplicated_legs,
        runtime.capped_legs,
    );
    let problem = runtime
        .problem
        .as_ref()
        .map(|problem| problem_message("Prewarm", problem))
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
        format!("{count} 条 · private WS 0"),
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
        "public legs {} · private WS {}",
        runtime.watchlist_public_prewarm_legs, runtime.private_ws_symbols,
    );
    let timing = format!(
        "next {} · fired {} · count {} · {}",
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
        || "暂无已入队提醒".into(),
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
