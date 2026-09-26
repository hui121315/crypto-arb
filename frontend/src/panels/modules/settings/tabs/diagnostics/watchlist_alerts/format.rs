use crate::api::ws::{WsChannelState, WsStatus};
use crate::panels::shared::ws_channel_activity_label;
use shared_types::{
    AlertDeliveryStatus, AlertRuleRuntimeStatus, WatchlistConfigSource, WatchlistPersistStatus,
    WatchlistPersistence, WatchlistPrewarmStatus, WatchlistStorageHealth, WatchlistStorageStatus,
};
use wasm_bindgen::JsValue;

pub(super) fn transport_summary(watchlist: &WsChannelState, alerts: &WsChannelState) -> String {
    format!(
        "自选行情 {} · 提醒 {}",
        channel_status_label(watchlist),
        channel_status_label(alerts),
    )
}

pub(super) fn transport_problem_summary(
    watchlist: &WsChannelState,
    alerts: &WsChannelState,
) -> Option<String> {
    let problems = [("自选行情", watchlist), ("提醒", alerts)]
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
            (WsStatus::Connected, false) => "等待订阅确认".into(),
            (WsStatus::Connecting, _) => "连接中".into(),
            (WsStatus::Disconnected, _) => "未连接".into(),
        }
    };
    format!("{status} · {}", ws_channel_activity_label(state))
}

pub(super) fn runtime_status_label(status: AlertRuleRuntimeStatus) -> &'static str {
    match status {
        AlertRuleRuntimeStatus::Configured => "等待检查条件",
        AlertRuleRuntimeStatus::Disabled => "已停用",
        AlertRuleRuntimeStatus::Waiting => "等待匹配",
        AlertRuleRuntimeStatus::Cooldown => "等待下次提醒",
        AlertRuleRuntimeStatus::Ready => "待发送",
        AlertRuleRuntimeStatus::Queued => "排队发送中",
        AlertRuleRuntimeStatus::Blocked => "暂不能发送",
    }
}

pub(super) fn storage_summary(storage: &WatchlistStorageHealth) -> String {
    match storage.status {
        WatchlistStorageStatus::Disabled => "存储未启用".to_owned(),
        WatchlistStorageStatus::Ready => format!("保存正常 · 版本 {}", storage.revision),
        WatchlistStorageStatus::Degraded => storage.problem.as_ref().map_or_else(
            || "存储异常".to_owned(),
            |problem| format!("存储异常 ({})", problem.code),
        ),
    }
}

pub(super) fn persistence_label(persistence: &WatchlistPersistence) -> String {
    let actor = if persistence.created_by.is_empty() {
        "创建者未知"
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
        WatchlistConfigSource::UserApi => "用户设置",
        WatchlistConfigSource::Migration => "旧配置迁入",
    }
}

pub(super) fn persist_status_label(status: WatchlistPersistStatus) -> &'static str {
    match status {
        WatchlistPersistStatus::Pending => "等待保存",
        WatchlistPersistStatus::Persisted => "已保存",
        WatchlistPersistStatus::Volatile => "临时生效，重启后丢失",
        WatchlistPersistStatus::Degraded => "保存失败",
    }
}

pub(super) fn delivery_status_label(status: AlertDeliveryStatus) -> &'static str {
    match status {
        AlertDeliveryStatus::Never => "尚未发送",
        AlertDeliveryStatus::Queued => "排队发送中",
        AlertDeliveryStatus::Blocked => "发送受阻",
    }
}

pub(super) fn prewarm_status_label(status: WatchlistPrewarmStatus) -> &'static str {
    match status {
        WatchlistPrewarmStatus::Idle => "未指定交易所",
        WatchlistPrewarmStatus::Disabled => "已停用",
        WatchlistPrewarmStatus::Planned => "等待读取",
        WatchlistPrewarmStatus::Fresh => "正常",
        WatchlistPrewarmStatus::Degraded => "部分数据异常",
        WatchlistPrewarmStatus::Capped => "超出数量限制",
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
