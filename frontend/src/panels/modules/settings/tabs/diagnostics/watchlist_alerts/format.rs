use crate::api::ws::{WsChannelState, WsStatus};
use crate::panels::shared::ws_channel_activity_label;
use shared_types::{
    AlertDeliveryStatus, AlertRuleRuntimeStatus, WatchlistConfigSource, WatchlistPersistStatus,
    WatchlistPersistence, WatchlistPrewarmStatus, WatchlistStorageHealth, WatchlistStorageStatus,
};
use wasm_bindgen::JsValue;

pub(super) fn transport_summary(watchlist: &WsChannelState, alerts: &WsChannelState) -> String {
    format!(
        "watchlist {} · alerts {}",
        channel_status_label(watchlist),
        channel_status_label(alerts),
    )
}

pub(super) fn transport_problem_summary(
    watchlist: &WsChannelState,
    alerts: &WsChannelState,
) -> Option<String> {
    let problems = [("watchlist", watchlist), ("alerts", alerts)]
        .into_iter()
        .filter_map(|(channel, state)| {
            state
                .last_error
                .as_ref()
                .map(|problem| format!("{channel}: {}", problem.message))
        })
        .collect::<Vec<_>>();
    (!problems.is_empty()).then(|| problems.join(" · "))
}

fn channel_status_label(state: &WsChannelState) -> String {
    let status = if let Some(problem) = &state.last_error {
        format!("异常 ({})", problem.code)
    } else {
        match (state.status, state.subscribed) {
            (WsStatus::Connected, true) => "已订阅".into(),
            (WsStatus::Connected, false) => "待 受理确认".into(),
            (WsStatus::Connecting, _) => "连接中".into(),
            (WsStatus::Disconnected, _) => "未连接".into(),
        }
    };
    format!("{status} · {}", ws_channel_activity_label(state))
}

pub(super) fn runtime_status_label(status: AlertRuleRuntimeStatus) -> &'static str {
    match status {
        AlertRuleRuntimeStatus::Configured => "待评估",
        AlertRuleRuntimeStatus::Disabled => "已停用",
        AlertRuleRuntimeStatus::Waiting => "等待匹配",
        AlertRuleRuntimeStatus::Cooldown => "冷却中",
        AlertRuleRuntimeStatus::Ready => "待投递",
        AlertRuleRuntimeStatus::Queued => "已入队",
        AlertRuleRuntimeStatus::Blocked => "阻断",
    }
}

pub(super) fn storage_summary(storage: &WatchlistStorageHealth) -> String {
    match storage.status {
        WatchlistStorageStatus::Disabled => "存储未启用".to_owned(),
        WatchlistStorageStatus::Ready => format!("SQLite rev {} 正常", storage.revision),
        WatchlistStorageStatus::Degraded => storage.problem.as_ref().map_or_else(
            || "SQLite 降级".to_owned(),
            |problem| format!("SQLite 降级 ({})", problem.code),
        ),
    }
}

pub(super) fn persistence_label(persistence: &WatchlistPersistence) -> String {
    let actor = if persistence.created_by.is_empty() {
        "unknown"
    } else {
        persistence.created_by.as_str()
    };
    format!(
        "v{} · {} · {} · {}",
        persistence.version,
        config_source_label(persistence.source),
        persist_status_label(persistence.persist_status),
        actor
    )
}

fn config_source_label(source: WatchlistConfigSource) -> &'static str {
    match source {
        WatchlistConfigSource::UserApi => "API",
        WatchlistConfigSource::Migration => "迁移",
    }
}

pub(super) fn persist_status_label(status: WatchlistPersistStatus) -> &'static str {
    match status {
        WatchlistPersistStatus::Pending => "待持久化",
        WatchlistPersistStatus::Persisted => "已持久化",
        WatchlistPersistStatus::Volatile => "仅内存",
        WatchlistPersistStatus::Degraded => "持久化失败",
    }
}

pub(super) fn delivery_status_label(status: AlertDeliveryStatus) -> &'static str {
    match status {
        AlertDeliveryStatus::Never => "未投递",
        AlertDeliveryStatus::Queued => "已入队",
        AlertDeliveryStatus::Blocked => "投递阻断",
    }
}

pub(super) fn prewarm_status_label(status: WatchlistPrewarmStatus) -> &'static str {
    match status {
        WatchlistPrewarmStatus::Idle => "无显式 venue",
        WatchlistPrewarmStatus::Disabled => "已停用",
        WatchlistPrewarmStatus::Planned => "已规划",
        WatchlistPrewarmStatus::Fresh => "正常",
        WatchlistPrewarmStatus::Degraded => "降级",
        WatchlistPrewarmStatus::Capped => "已截断",
    }
}

pub(super) fn prewarm_status_class(status: WatchlistPrewarmStatus) -> &'static str {
    match status {
        WatchlistPrewarmStatus::Fresh => "status-pill ready",
        WatchlistPrewarmStatus::Degraded | WatchlistPrewarmStatus::Capped => "status-pill blocked",
        _ => "status-pill pending",
    }
}

pub(super) fn runtime_status_class(status: AlertRuleRuntimeStatus) -> &'static str {
    match status {
        AlertRuleRuntimeStatus::Queued | AlertRuleRuntimeStatus::Ready => "status-pill ready",
        AlertRuleRuntimeStatus::Blocked => "status-pill blocked",
        _ => "status-pill pending",
    }
}

pub(super) fn optional_number(value: Option<f64>) -> String {
    value.map_or_else(|| "-".into(), |value| format!("{value:.4}"))
}

pub(super) fn optional_time(value: Option<i64>) -> String {
    value.map_or_else(|| "--:--".into(), time_label)
}

fn time_label(ms: i64) -> String {
    if ms <= 0 {
        return "--:--".into();
    }
    let date = js_sys::Date::new(&JsValue::from_f64(ms as f64));
    format!("{:02}:{:02}", date.get_hours(), date.get_minutes())
}
