use super::format::persist_status_label;
use super::*;
use crate::api::ws::WsChannelState;
use shared_types::{
    AlertDeliveryStatus, AlertRuleRuntimeStatus, WatchlistConfigSource, WatchlistPersistStatus,
    WatchlistPersistence, WatchlistPrewarmStatus, WatchlistStorageHealth, WatchlistStorageStatus,
};

#[test]
fn runtime_labels_distinguish_queued_blocked_and_risk_alert_channels() {
    assert_eq!(
        runtime_status_label(AlertRuleRuntimeStatus::Queued),
        "排队发送中"
    );
    assert_eq!(
        runtime_status_class(AlertRuleRuntimeStatus::Blocked),
        "status-pill blocked"
    );
    assert_eq!(
        prewarm_status_label(WatchlistPrewarmStatus::Capped),
        "超出数量限制"
    );
    let mut watchlist = WsChannelState::new("watchlist");
    watchlist.message_count = 1;
    let mut alerts = WsChannelState::new("alerts");
    alerts.message_count = 2;
    let summary = transport_summary(&watchlist, &alerts);
    assert!(summary.contains("自选行情"));
    assert!(summary.contains("提醒"));
    assert!(summary.contains("自选行情 未连接 · 帧 1 · 错误 0"));
    assert!(summary.contains("提醒 未连接 · 帧 2 · 错误 0"));
    assert!(!summary.contains("risk-alerts"));
    assert_eq!(
        persist_status_label(WatchlistPersistStatus::Persisted),
        "已保存"
    );
    assert_eq!(
        persistence_label(&WatchlistPersistence {
            source: WatchlistConfigSource::Migration,
            created_by: "operator".to_owned(),
            version: 3,
            persist_status: WatchlistPersistStatus::Persisted,
            ..WatchlistPersistence::default()
        }),
        "v3 · 旧配置迁入 · 已保存 · operator"
    );
    assert_eq!(
        delivery_status_label(AlertDeliveryStatus::Blocked),
        "发送受阻"
    );
    let storage = WatchlistStorageHealth {
        backend: "sqlite".to_owned(),
        configured: true,
        status: WatchlistStorageStatus::Ready,
        revision: 7,
        ..WatchlistStorageHealth::default()
    };
    assert_eq!(storage_summary(&storage), "保存正常 · 版本 7");
}
