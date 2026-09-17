#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::too_many_lines
    )
)]
//! 实时层：TTL 缓存 / 自动刷新快照 / WebSocket Hub / 频道路由 / Redis Pub/Sub。
//!
//! 对应 Python `analysis-service/services/arbitrage_cache_v3.py` + `WebSocketManager`。

pub mod alerts;
pub mod cache;
pub mod channels;
pub mod history;
pub mod hub;
pub mod pubsub;
pub mod quality;
pub mod snapshot;
pub mod throttle;

pub use alerts::{
    AlertChannel, AlertDeliveryState, AlertDeliveryStatus, AlertNotification, AlertRule,
    AlertRuleRuntime, AlertRuleRuntimeStatus, AlertRulesEnvelope, AlertStreamEvent,
    WatchlistAlertReplay, WatchlistAlertStore, WatchlistConfigSource, WatchlistEnvelope,
    WatchlistItem, WatchlistItemRuntime, WatchlistPersistStatus, WatchlistPersistence,
    WatchlistPrewarmStatus, WatchlistRuntimeContract, WatchlistStorageHealth,
    WatchlistStorageStatus, WatchlistStreamEvent,
};
pub use cache::TtlCache;
pub use channels::{parse as parse_topic, Topic};
pub use history::{
    history_migration_checksum_hex, FundingDiffQuery, FundingDiffRow, FundingDiffStatsProjector,
    FundingDiffStatsQuery, FundingQuery, FundingRow, HistoryStore, HistoryStoreHealth,
    IndexCompositionHistoryRow, IndexCompositionQuery, OpportunityQuery, OpportunityRow,
    HISTORY_SCHEMA_VERSION,
};
pub use hub::{WsChannelRuntimeSnapshot, WsHub, WsMessage};
pub use pubsub::{PubSubError, RedisPubSub};
pub use quality::VenueQualityTracker;
pub use shared_types::{FundingDiffStatsRow, FundingDiffWindowStats};
pub use snapshot::{staleness_ms, RefreshingSnapshot, SnapshotEntry};
pub use throttle::Throttle;
