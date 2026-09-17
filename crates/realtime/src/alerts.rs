pub use shared_types::{
    AlertChannel, AlertDeliveryState, AlertDeliveryStatus, AlertNotification, AlertRule,
    AlertRuleRuntime, AlertRuleRuntimeStatus, AlertRulesEnvelope, AlertStreamEvent,
    WatchlistConfigSource, WatchlistEnvelope, WatchlistItem, WatchlistItemRuntime,
    WatchlistPersistStatus, WatchlistPersistence, WatchlistPrewarmStatus, WatchlistRuntimeContract,
    WatchlistStorageHealth, WatchlistStorageStatus, WatchlistStreamEvent,
};

mod evaluation;
mod mutations;
mod storage;

pub use evaluation::{
    apply_alert_queue_result, evaluate_alert_rules, AlertEvaluation, AlertQueueAttempt,
};
pub use mutations::{
    insert_alert_rule, insert_watchlist_item, remove_alert_rule, remove_watchlist_item,
    AlertRuleMutation, WatchlistMutation, WatchlistRemoval,
};
pub use storage::{
    storage_schema_hash, WatchlistAlertReplay, WatchlistAlertStore,
    WATCHLIST_ALERT_STORAGE_MIGRATION_ID, WATCHLIST_ALERT_STORAGE_MIGRATION_PATH,
    WATCHLIST_ALERT_STORAGE_SCHEMA_VERSION,
};

pub const WATCHLIST_ALERTS_FEATURE: &str = "api_surface.watchlist_alerts";
pub const WATCHLIST_TICKER_SYMBOLS_PER_VENUE: usize = 32;

pub fn watchlist_envelope(items: Vec<WatchlistItem>) -> WatchlistEnvelope {
    watchlist_envelope_with_storage(items, WatchlistStorageHealth::default())
}

pub fn watchlist_envelope_with_storage(
    items: Vec<WatchlistItem>,
    storage: WatchlistStorageHealth,
) -> WatchlistEnvelope {
    WatchlistEnvelope {
        items,
        runtime: watchlist_runtime_contract(storage),
    }
}

pub fn alert_rules_envelope(rules: Vec<AlertRule>) -> AlertRulesEnvelope {
    alert_rules_envelope_with_storage(rules, WatchlistStorageHealth::default())
}

pub fn alert_rules_envelope_with_storage(
    rules: Vec<AlertRule>,
    storage: WatchlistStorageHealth,
) -> AlertRulesEnvelope {
    AlertRulesEnvelope {
        rules,
        runtime: watchlist_runtime_contract(storage),
    }
}

pub fn watchlist_runtime_contract(storage: WatchlistStorageHealth) -> WatchlistRuntimeContract {
    let (persistence, volatile, restart_behavior) = match storage.status {
        WatchlistStorageStatus::Ready => ("sqlite", false, "restored_from_sqlite"),
        WatchlistStorageStatus::Degraded => ("sqlite_degraded", true, "restore_or_persist_failed"),
        WatchlistStorageStatus::Disabled => ("memory", true, "cleared_on_restart"),
    };
    WatchlistRuntimeContract {
        feature_gate: WATCHLIST_ALERTS_FEATURE.to_owned(),
        persistence: persistence.to_owned(),
        volatile,
        restart_behavior: restart_behavior.to_owned(),
        storage,
        public_ticker_symbols_per_venue_limit: WATCHLIST_TICKER_SYMBOLS_PER_VENUE,
        private_ws_symbols_from_watchlist: 0,
    }
}

#[cfg(test)]
#[path = "alerts/tests.rs"]
mod tests;
