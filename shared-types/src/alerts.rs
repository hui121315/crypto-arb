use serde::{Deserialize, Serialize};

use crate::{ApiProblem, StrategyKind};

pub const MAX_ALERT_COOLDOWN_SECS: u64 = 7 * 24 * 60 * 60;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WatchlistConfigSource {
    #[default]
    UserApi,
    Migration,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WatchlistPersistStatus {
    Pending,
    Persisted,
    #[default]
    Volatile,
    Degraded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchlistPersistence {
    #[serde(default)]
    pub source: WatchlistConfigSource,
    #[serde(default)]
    pub created_by: String,
    #[serde(default)]
    pub updated_at_ms: i64,
    #[serde(default = "default_persistence_version")]
    pub version: u64,
    #[serde(default)]
    pub persist_status: WatchlistPersistStatus,
}

impl Default for WatchlistPersistence {
    fn default() -> Self {
        Self {
            source: WatchlistConfigSource::UserApi,
            created_by: String::new(),
            updated_at_ms: 0,
            version: default_persistence_version(),
            persist_status: WatchlistPersistStatus::Volatile,
        }
    }
}

impl WatchlistPersistence {
    pub fn pending(created_by: impl Into<String>, now_ms: i64) -> Self {
        Self {
            created_by: created_by.into(),
            updated_at_ms: now_ms,
            persist_status: WatchlistPersistStatus::Pending,
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchlistItem {
    #[serde(default)]
    pub id: i64,
    pub symbol: String,
    #[serde(default)]
    pub venue_long: Option<String>,
    #[serde(default)]
    pub venue_short: Option<String>,
    #[serde(default)]
    pub min_net_yield: Option<f64>,
    #[serde(default)]
    pub min_volume_24h: Option<f64>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub created_at_ms: i64,
    #[serde(flatten)]
    pub persistence: WatchlistPersistence,
    #[serde(default)]
    pub runtime: WatchlistItemRuntime,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WatchlistPrewarmStatus {
    #[default]
    Idle,
    Disabled,
    Planned,
    Fresh,
    Degraded,
    Capped,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchlistItemRuntime {
    pub status: WatchlistPrewarmStatus,
    pub requested_public_legs: usize,
    pub planned_ticker_legs: usize,
    pub deduplicated_legs: usize,
    pub capped_legs: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_prewarm_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<ApiProblem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum AlertChannel {
    Toast,
    TelegramWebhook {
        #[serde(skip_serializing)]
        url: String,
    },
    GenericWebhook {
        #[serde(skip_serializing)]
        url: String,
        #[serde(default, skip_serializing)]
        secret: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlertRule {
    #[serde(default)]
    pub id: i64,
    pub watchlist_id: i64,
    pub channel: AlertChannel,
    #[serde(default = "default_cooldown_secs")]
    pub cooldown_secs: u64,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub created_at_ms: i64,
    #[serde(flatten)]
    pub persistence: WatchlistPersistence,
    #[serde(flatten)]
    pub delivery: AlertDeliveryState,
    #[serde(default)]
    pub runtime: AlertRuleRuntime,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertDeliveryStatus {
    #[default]
    Never,
    Queued,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlertDeliveryState {
    #[serde(default = "default_delivery_kind")]
    pub delivery_kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_fired_at_ms: Option<i64>,
    #[serde(default)]
    pub last_delivery_status: AlertDeliveryStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<ApiProblem>,
}

impl Default for AlertDeliveryState {
    fn default() -> Self {
        Self {
            delivery_kind: default_delivery_kind(),
            last_fired_at_ms: None,
            last_delivery_status: AlertDeliveryStatus::Never,
            last_error: None,
        }
    }
}

impl AlertDeliveryState {
    pub fn configured(channel: &AlertChannel) -> Self {
        Self {
            delivery_kind: channel.delivery_kind().to_owned(),
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertRuleRuntimeStatus {
    Configured,
    Disabled,
    Waiting,
    Cooldown,
    Ready,
    Queued,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlertRuleRuntime {
    pub status: AlertRuleRuntimeStatus,
    pub transport: String,
    pub delivery_supported: bool,
    pub watchlist_public_prewarm_legs: usize,
    pub private_ws_symbols: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_evaluated_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_triggered_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_eligible_at_ms: Option<i64>,
    #[serde(default)]
    pub trigger_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_opportunity_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<ApiProblem>,
}

impl Default for AlertRuleRuntime {
    fn default() -> Self {
        Self {
            status: AlertRuleRuntimeStatus::Configured,
            transport: "unconfigured".to_owned(),
            delivery_supported: false,
            watchlist_public_prewarm_legs: 0,
            private_ws_symbols: 0,
            last_evaluated_at_ms: None,
            last_triggered_at_ms: None,
            next_eligible_at_ms: None,
            trigger_count: 0,
            last_opportunity_id: None,
            problem: None,
        }
    }
}

impl AlertRuleRuntime {
    pub fn configured(channel: &AlertChannel, enabled: bool, public_prewarm_legs: usize) -> Self {
        Self {
            status: if enabled {
                AlertRuleRuntimeStatus::Configured
            } else {
                AlertRuleRuntimeStatus::Disabled
            },
            transport: channel.runtime_transport().to_owned(),
            delivery_supported: channel.is_runtime_deliverable(),
            watchlist_public_prewarm_legs: public_prewarm_legs,
            private_ws_symbols: 0,
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchlistEnvelope {
    pub items: Vec<WatchlistItem>,
    pub runtime: WatchlistRuntimeContract,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlertRulesEnvelope {
    pub rules: Vec<AlertRule>,
    pub runtime: WatchlistRuntimeContract,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlertNotification {
    pub id: String,
    pub rule_id: i64,
    pub watchlist_id: i64,
    pub opportunity_id: String,
    pub symbol: String,
    pub strategy: StrategyKind,
    pub long_exchange: String,
    pub short_exchange: String,
    #[serde(default, alias = "finalScore")]
    pub one_cycle_net_bps: f64,
    pub net_single_yield: f64,
    pub queued_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum AlertStreamEvent {
    AlertRulesChanged {
        envelope: Box<AlertRulesEnvelope>,
        #[serde(rename = "timestampMs")]
        timestamp_ms: i64,
    },
    AlertTriggered {
        notification: AlertNotification,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum WatchlistStreamEvent {
    WatchlistChanged {
        envelope: WatchlistEnvelope,
        #[serde(rename = "timestampMs")]
        timestamp_ms: i64,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchlistRuntimeContract {
    pub feature_gate: String,
    pub persistence: String,
    pub volatile: bool,
    pub restart_behavior: String,
    #[serde(default)]
    pub storage: WatchlistStorageHealth,
    pub public_ticker_symbols_per_venue_limit: usize,
    pub private_ws_symbols_from_watchlist: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WatchlistStorageStatus {
    #[default]
    Disabled,
    Ready,
    Degraded,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchlistStorageHealth {
    pub backend: String,
    pub configured: bool,
    pub status: WatchlistStorageStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<u32>,
    #[serde(default)]
    pub revision: u64,
    #[serde(default)]
    pub watchlist_item_count: usize,
    #[serde(default)]
    pub alert_rule_count: usize,
    #[serde(default)]
    pub persist_attempts: u64,
    #[serde(default)]
    pub persist_successes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_persisted_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<ApiProblem>,
}

impl Default for WatchlistStorageHealth {
    fn default() -> Self {
        Self {
            backend: "memory".to_owned(),
            configured: false,
            status: WatchlistStorageStatus::Disabled,
            schema_version: None,
            revision: 0,
            watchlist_item_count: 0,
            alert_rule_count: 0,
            persist_attempts: 0,
            persist_successes: 0,
            last_persisted_at_ms: None,
            problem: None,
        }
    }
}

impl WatchlistItem {
    pub fn validate(&self) -> Result<(), String> {
        if self.symbol.trim().is_empty() {
            return Err("symbol is required".to_owned());
        }
        for (field, venue) in [
            ("venueLong", self.venue_long.as_deref()),
            ("venueShort", self.venue_short.as_deref()),
        ] {
            if venue.is_some_and(|value| value.trim().is_empty()) {
                return Err(format!("{field} must not be blank when provided"));
            }
        }
        validate_threshold("minNetYield", self.min_net_yield)?;
        validate_threshold("minVolume24h", self.min_volume_24h)?;
        Ok(())
    }
}

impl AlertRule {
    pub fn validate(&self) -> Result<(), String> {
        if self.watchlist_id <= 0 {
            return Err("watchlistId is required".to_owned());
        }
        if self.cooldown_secs > MAX_ALERT_COOLDOWN_SECS {
            return Err(format!(
                "cooldownSecs must not exceed {MAX_ALERT_COOLDOWN_SECS}"
            ));
        }
        self.channel.validate()
    }
}

impl AlertChannel {
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Toast => Ok(()),
            Self::TelegramWebhook { .. } | Self::GenericWebhook { .. } => {
                Err("webhook delivery is disabled; use toast alerts".to_owned())
            }
        }
    }

    pub const fn is_runtime_deliverable(&self) -> bool {
        matches!(self, Self::Toast)
    }

    pub const fn delivery_kind(&self) -> &'static str {
        match self {
            Self::Toast => "toast",
            Self::TelegramWebhook { .. } => "telegram_webhook_disabled",
            Self::GenericWebhook { .. } => "generic_webhook_disabled",
        }
    }

    pub const fn runtime_transport(&self) -> &'static str {
        match self {
            Self::Toast => "app_websocket_toast",
            Self::TelegramWebhook { .. } | Self::GenericWebhook { .. } => "unsupported",
        }
    }
}

fn validate_threshold(field: &str, value: Option<f64>) -> Result<(), String> {
    if let Some(value) = value {
        if !value.is_finite() {
            return Err(format!("{field} must be a finite number"));
        }
        if value < 0.0 {
            return Err(format!("{field} must not be negative"));
        }
    }
    Ok(())
}

fn default_enabled() -> bool {
    true
}

fn default_cooldown_secs() -> u64 {
    300
}

fn default_persistence_version() -> u64 {
    1
}

fn default_delivery_kind() -> String {
    "unconfigured".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_item() -> WatchlistItem {
        WatchlistItem {
            id: 0,
            symbol: "BTC-USDT".into(),
            venue_long: Some("binance".into()),
            venue_short: Some("okx".into()),
            min_net_yield: Some(0.5),
            min_volume_24h: Some(1000.0),
            enabled: true,
            created_at_ms: 0,
            persistence: WatchlistPersistence::default(),
            runtime: WatchlistItemRuntime::default(),
        }
    }

    fn runtime_contract() -> WatchlistRuntimeContract {
        WatchlistRuntimeContract {
            feature_gate: "api_surface.watchlist_alerts".to_owned(),
            persistence: "memory".to_owned(),
            volatile: true,
            restart_behavior: "cleared_on_restart".to_owned(),
            storage: WatchlistStorageHealth::default(),
            public_ticker_symbols_per_venue_limit: 32,
            private_ws_symbols_from_watchlist: 0,
        }
    }

    #[test]
    fn watchlist_item_serde_uses_camel_case_and_defaults() {
        let item: WatchlistItem = serde_json::from_value(json!({
            "symbol": "BTC-USDT",
            "venueLong": "binance",
            "minNetYield": 0.5
        }))
        .unwrap_or_else(|error| panic!("deserialize watchlist item: {error}"));

        assert_eq!(item.id, 0);
        assert!(item.enabled);
        assert_eq!(item.venue_long.as_deref(), Some("binance"));
        assert_eq!(item.min_net_yield, Some(0.5));
        assert_eq!(item.persistence.source, WatchlistConfigSource::UserApi);
        assert_eq!(item.persistence.version, 1);
        assert_eq!(
            item.persistence.persist_status,
            WatchlistPersistStatus::Volatile
        );
        let serialized = serde_json::to_value(item)
            .unwrap_or_else(|error| panic!("serialize watchlist item: {error}"));
        assert_eq!(serialized["venueLong"], "binance");
        assert_eq!(serialized["minNetYield"], 0.5);
        assert!(serialized.get("venue_long").is_none());
    }

    #[test]
    fn webhook_configuration_is_never_serialized() {
        let channel = AlertChannel::GenericWebhook {
            url: "https://hooks.example.com/abc".into(),
            secret: Some("super-secret-token".into()),
        };

        let serialized = serde_json::to_string(&channel).unwrap_or_default();

        assert!(!serialized.contains("https://hooks.example.com/abc"));
        assert!(!serialized.contains("super-secret-token"));
        assert!(!serialized.contains("secret"));
        assert!(serialized.contains("genericWebhook"));
    }

    #[test]
    fn generic_webhook_secret_is_still_accepted_on_input() {
        let channel: AlertChannel = serde_json::from_value(json!({
            "kind": "genericWebhook",
            "url": "https://hooks.example.com/abc",
            "secret": "inbound-token"
        }))
        .unwrap_or_else(|error| panic!("deserialize alert channel: {error}"));

        let secret = match channel {
            AlertChannel::GenericWebhook { secret, .. } => secret,
            _ => None,
        };
        assert_eq!(secret.as_deref(), Some("inbound-token"));
    }

    #[test]
    fn alert_rule_serde_applies_defaults() {
        let rule: AlertRule = serde_json::from_value(json!({
            "watchlistId": 7,
            "channel": { "kind": "toast" }
        }))
        .unwrap_or_else(|error| panic!("deserialize alert rule: {error}"));

        assert_eq!(rule.id, 0);
        assert_eq!(rule.cooldown_secs, 300);
        assert!(rule.enabled);
        assert_eq!(rule.created_at_ms, 0);
        assert_eq!(rule.persistence.version, 1);
        assert_eq!(rule.delivery.delivery_kind, "unconfigured");
        assert_eq!(
            rule.delivery.last_delivery_status,
            AlertDeliveryStatus::Never
        );
        assert!(rule.validate().is_ok());
    }

    #[test]
    fn alert_rule_requires_positive_watchlist_id() {
        let rule = AlertRule {
            id: 0,
            watchlist_id: 0,
            channel: AlertChannel::Toast,
            cooldown_secs: 300,
            enabled: true,
            created_at_ms: 0,
            persistence: WatchlistPersistence::default(),
            delivery: AlertDeliveryState::configured(&AlertChannel::Toast),
            runtime: AlertRuleRuntime::default(),
        };
        assert!(rule.validate().is_err());
    }

    #[test]
    fn alert_rule_rejects_cooldown_above_product_limit() {
        let rule = AlertRule {
            id: 0,
            watchlist_id: 1,
            channel: AlertChannel::Toast,
            cooldown_secs: MAX_ALERT_COOLDOWN_SECS + 1,
            enabled: true,
            created_at_ms: 0,
            persistence: WatchlistPersistence::default(),
            delivery: AlertDeliveryState::configured(&AlertChannel::Toast),
            runtime: AlertRuleRuntime::default(),
        };

        assert_eq!(
            rule.validate(),
            Err(format!(
                "cooldownSecs must not exceed {MAX_ALERT_COOLDOWN_SECS}"
            ))
        );
    }

    #[test]
    fn toast_channel_is_always_valid() {
        assert!(AlertChannel::Toast.validate().is_ok());
    }

    #[test]
    fn webhook_channel_is_fail_closed_even_with_https_url() {
        let channel = AlertChannel::GenericWebhook {
            url: "https://hooks.example.com/abc".into(),
            secret: Some("token".into()),
        };
        assert!(channel.validate().is_err());
        assert!(!channel.is_runtime_deliverable());
    }

    #[test]
    fn valid_watchlist_item_passes_validation() {
        assert!(sample_item().validate().is_ok());
    }

    #[test]
    fn blank_symbol_is_rejected() {
        let mut item = sample_item();
        item.symbol = "   ".into();
        assert!(item.validate().is_err());
    }

    #[test]
    fn blank_venue_override_is_rejected() {
        let mut item = sample_item();
        item.venue_long = Some("  ".into());
        assert!(item.validate().is_err());
    }

    #[test]
    fn non_finite_threshold_is_rejected() {
        let mut item = sample_item();
        item.min_net_yield = Some(f64::NAN);
        assert!(item.validate().is_err());
        let mut item = sample_item();
        item.min_volume_24h = Some(f64::INFINITY);
        assert!(item.validate().is_err());
    }

    #[test]
    fn negative_threshold_is_rejected() {
        let mut item = sample_item();
        item.min_volume_24h = Some(-1.0);
        assert!(item.validate().is_err());
    }

    #[test]
    fn omitted_optional_thresholds_pass() {
        let mut item = sample_item();
        item.venue_long = None;
        item.venue_short = None;
        item.min_net_yield = None;
        item.min_volume_24h = None;
        assert!(item.validate().is_ok());
    }

    #[test]
    fn watchlist_envelope_round_trips_runtime_contract() {
        let envelope = WatchlistEnvelope {
            items: vec![sample_item()],
            runtime: runtime_contract(),
        };

        let serialized = serde_json::to_value(&envelope)
            .unwrap_or_else(|error| panic!("serialize watchlist envelope: {error}"));
        assert_eq!(
            serialized["runtime"]["featureGate"],
            "api_surface.watchlist_alerts"
        );
        assert_eq!(serialized["runtime"]["persistence"], "memory");
        assert_eq!(
            serialized["runtime"]["restartBehavior"],
            "cleared_on_restart"
        );
        assert!(serialized["runtime"]
            .get("publicOrderbookPrewarmLimit")
            .is_none());
        assert_eq!(serialized["runtime"]["privateWsSymbolsFromWatchlist"], 0);

        let decoded: WatchlistEnvelope = serde_json::from_value(serialized)
            .unwrap_or_else(|error| panic!("deserialize watchlist envelope: {error}"));
        assert_eq!(decoded, envelope);
    }

    #[test]
    fn alert_rules_envelope_round_trips_runtime_contract() {
        let envelope = AlertRulesEnvelope {
            rules: vec![AlertRule {
                id: 11,
                watchlist_id: 7,
                channel: AlertChannel::Toast,
                cooldown_secs: 300,
                enabled: true,
                created_at_ms: 123,
                persistence: WatchlistPersistence::pending("operator", 123),
                delivery: AlertDeliveryState::configured(&AlertChannel::Toast),
                runtime: AlertRuleRuntime::configured(&AlertChannel::Toast, true, 2),
            }],
            runtime: runtime_contract(),
        };

        let serialized = serde_json::to_value(&envelope)
            .unwrap_or_else(|error| panic!("serialize alert rules envelope: {error}"));
        assert_eq!(serialized["rules"][0]["watchlistId"], 7);
        assert_eq!(serialized["rules"][0]["channel"]["kind"], "toast");
        let decoded: AlertRulesEnvelope = serde_json::from_value(serialized)
            .unwrap_or_else(|error| panic!("deserialize alert rules envelope: {error}"));
        assert_eq!(decoded, envelope);
    }

    #[test]
    fn watchlist_alert_stream_timestamps_use_camel_case() {
        let watchlist = WatchlistStreamEvent::WatchlistChanged {
            envelope: WatchlistEnvelope {
                items: vec![],
                runtime: runtime_contract(),
            },
            timestamp_ms: 42,
        };
        let alerts = AlertStreamEvent::AlertRulesChanged {
            envelope: Box::new(AlertRulesEnvelope {
                rules: vec![],
                runtime: runtime_contract(),
            }),
            timestamp_ms: 43,
        };

        for event in [
            serde_json::to_value(watchlist)
                .unwrap_or_else(|error| panic!("serialize watchlist event: {error}")),
            serde_json::to_value(alerts)
                .unwrap_or_else(|error| panic!("serialize alert event: {error}")),
        ] {
            assert!(event.get("timestampMs").is_some());
            assert!(event.get("timestamp_ms").is_none());
        }
    }
}
