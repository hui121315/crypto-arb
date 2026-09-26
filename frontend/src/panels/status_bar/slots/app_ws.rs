use super::*;
use crate::api::ws::{WsChannelState, WsStatus};
use crate::panels::shared::ws_channel_activity_label;
use shared_types::{VenueOperationHealthSnapshot, VenueOperationKind, VenueOperationStatus};

pub(super) const APP_WS_SLOT_LABEL: &str = "后台连接";

#[component]
pub fn AppWsStatusSlot(
    channel: Memo<WsChannelState>,
    operation_health: Memo<Option<VenueOperationHealthSnapshot>>,
) -> impl IntoView {
    let readiness = Memo::new(move |_| {
        app_connection_readiness(&channel.get(), operation_health.get().as_ref()).readiness
    });
    view! {
        <div
            data-testid="status-app-ws"
            data-state=move || readiness.get().state()
            class=move || readiness.get().slot_class()
            title=move || {
                let health = operation_health.get();
                app_ws_title(&channel.get(), health.as_ref())
            }
        >
            <span class=move || readiness.get().dot_class()></span>
            <span class="slot-label">{APP_WS_SLOT_LABEL}</span>
            <span class="num">{move || {
                let health = operation_health.get();
                app_ws_label(&channel.get(), health.as_ref())
            }}</span>
        </div>
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct AppWsLagSummary {
    pub(super) channels: usize,
    pub(super) lag_events: u64,
    pub(super) skipped_messages: u64,
    pub(super) recent_channels: usize,
    pub(super) recent_skipped_messages: u64,
}

#[cfg(test)]
pub(super) fn app_ws_degraded(
    channel: &WsChannelState,
    operation_health: Option<&VenueOperationHealthSnapshot>,
) -> bool {
    app_connection_readiness(channel, operation_health)
        .readiness
        .needs_attention()
}

pub(super) fn app_ws_label(
    channel: &WsChannelState,
    operation_health: Option<&VenueOperationHealthSnapshot>,
) -> String {
    let lag = app_ws_lag_summary(operation_health);
    if lag.recent_channels > 0 {
        format!("丢帧 {}", lag.recent_skipped_messages)
    } else if channel
        .last_error
        .as_ref()
        .is_some_and(|problem| problem.code == "WS_BROADCAST_LAGGED")
    {
        "发生丢帧".to_owned()
    } else if channel.last_error.is_some() {
        "异常".to_owned()
    } else if channel.status == WsStatus::Connected && channel.subscribed {
        "已订阅".to_owned()
    } else {
        match channel.status {
            WsStatus::Connecting => "连接中".to_owned(),
            WsStatus::Connected => "待订阅".to_owned(),
            WsStatus::Disconnected => "已断开".to_owned(),
        }
    }
}

pub(super) fn app_ws_title(
    channel: &WsChannelState,
    operation_health: Option<&VenueOperationHealthSnapshot>,
) -> String {
    let lag = app_ws_lag_summary(operation_health);
    let status = format!(
        "App WS channel {}：{}，subscription_ack={}，不使用 subscriber count 代理健康",
        channel.channel,
        app_ws_label(channel, operation_health),
        channel.subscribed
    );
    let lag_evidence = format!(
        "后端频道 {} · lag 事件 {} · 累计丢帧 {} · 近期异常频道 {}",
        lag.channels, lag.lag_events, lag.skipped_messages, lag.recent_channels
    );
    let freshness = channel
        .last_message_at_ms
        .map(|at| format!("last_message_at_ms {at}"))
        .unwrap_or_default();
    let retry = channel
        .retry_after_ms
        .map(|ms| format!("retry {ms}ms"))
        .unwrap_or_default();
    title_parts([
        status,
        lag_evidence,
        ws_channel_activity_label(channel),
        freshness,
        retry,
        channel
            .last_error
            .as_ref()
            .map(api_problem_summary)
            .unwrap_or_default(),
    ])
}

pub(super) fn app_ws_lag_summary(
    snapshot: Option<&VenueOperationHealthSnapshot>,
) -> AppWsLagSummary {
    snapshot
        .into_iter()
        .flat_map(|snapshot| snapshot.rows.iter())
        .filter(|row| {
            VenueOperationKind::parse(&row.operation) == VenueOperationKind::AppWsBroadcast
        })
        .fold(AppWsLagSummary::default(), |mut summary, row| {
            summary.channels += 1;
            summary.lag_events = summary
                .lag_events
                .saturating_add(row.requested.unwrap_or_default());
            summary.skipped_messages = summary
                .skipped_messages
                .saturating_add(row.rows.unwrap_or_default());
            if row.status != VenueOperationStatus::Ok {
                summary.recent_channels += 1;
                summary.recent_skipped_messages = summary
                    .recent_skipped_messages
                    .saturating_add(row.rows.unwrap_or_default());
            }
            summary
        })
}
