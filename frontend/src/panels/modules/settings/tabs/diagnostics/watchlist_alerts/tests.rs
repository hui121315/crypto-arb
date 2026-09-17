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
        "已入队"
    );
    assert_eq!(
        runtime_status_class(AlertRuleRuntimeStatus::Blocked),
        "status-pill blocked"
    );
    assert_eq!(
        prewarm_status_label(WatchlistPrewarmStatus::Capped),
        "已截断"
    );
    let mut watchlist = WsChannelState::new("watchlist");
    watchlist.message_count = 1;
    let mut alerts = WsChannelState::new("alerts");
    alerts.message_count = 2;
    let summary = transport_summary(&watchlist, &alerts);
    assert!(summary.contains("watchlist"));
    assert!(summary.contains("alerts"));
    assert!(summary.contains("watchlist 未连接 · 帧 1 · 错误 0"));
    assert!(summary.contains("alerts 未连接 · 帧 2 · 错误 0"));
    assert!(!summary.contains("risk-alerts"));
    assert_eq!(
        persist_status_label(WatchlistPersistStatus::Persisted),
        "已持久化"
    );
    assert_eq!(
        persistence_label(&WatchlistPersistence {
            source: WatchlistConfigSource::Migration,
            created_by: "operator".to_owned(),
            version: 3,
            persist_status: WatchlistPersistStatus::Persisted,
            ..WatchlistPersistence::default()
        }),
        "v3 · 迁移 · 已持久化 · operator"
    );
    assert_eq!(
        delivery_status_label(AlertDeliveryStatus::Blocked),
        "投递阻断"
    );
    let storage = WatchlistStorageHealth {
        backend: "sqlite".to_owned(),
        configured: true,
        status: WatchlistStorageStatus::Ready,
        revision: 7,
        ..WatchlistStorageHealth::default()
    };
    assert_eq!(storage_summary(&storage), "SQLite rev 7 正常");
}
