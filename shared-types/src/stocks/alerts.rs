use super::*;
use rust_decimal::Decimal;
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockAlertConfig {
    pub enabled: bool,
    #[serde(default)]
    pub include_peer: bool,
    pub min_spread_pct: String,
    pub cooldown_secs: u32,
}

impl Default for StockAlertConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            include_peer: false,
            min_spread_pct: "0.5".into(),
            cooldown_secs: 60,
        }
    }
}

impl StockAlertConfig {
    pub fn threshold(&self) -> Result<Decimal, String> {
        if !(10..=3600).contains(&self.cooldown_secs) {
            return Err("股票提醒间隔须为 10–3600 秒".into());
        }
        Decimal::from_str(&self.min_spread_pct)
            .ok()
            .filter(|n| *n > Decimal::ZERO && *n <= Decimal::from(100) && n.scale() <= 4)
            .ok_or_else(|| "报价差额阈值须大于 0、不超过 100%，最多 4 位小数".into())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StockAlertPhase {
    #[default]
    Disabled,
    NeedsWebhook,
    WaitingQuotes,
    Watching,
    Cooldown,
    Queued,
    Degraded,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockAlertSummary {
    pub event_id: String,
    pub direction: String,
    pub gross_usdc: String,
    pub spread_pct: String,
    pub queued_at_ms: i64,
    pub delivery: Option<crate::WebhookDeliveryRecord>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockAlertRuntime {
    pub phase: StockAlertPhase,
    pub problem: Option<String>,
    pub recent: Vec<StockAlertSummary>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stock_alert_config_preserves_percent_decimals_and_old_clients_default_to_off() {
        let config = StockAlertConfig {
            enabled: true,
            include_peer: true,
            min_spread_pct: "0.0001".into(),
            cooldown_secs: 10,
        };
        assert_eq!(config.threshold().unwrap().to_string(), "0.0001");
        for invalid in ["NaN", "0", "-1", "100.01", "0.00001"] {
            assert!(StockAlertConfig {
                min_spread_pct: invalid.into(),
                ..config.clone()
            }
            .threshold()
            .is_err());
        }
        for cooldown_secs in [0, 9, 3601] {
            assert!(StockAlertConfig {
                cooldown_secs,
                ..config.clone()
            }
            .threshold()
            .is_err());
        }
        let request: StockMonitorRequest = serde_json::from_value(
            serde_json::json!({"enabled":true,"quote":{"asset":"MU.US","budgetUsdc":"10"}}),
        )
        .unwrap();
        assert!(!request.alerts.enabled);
        assert!(!request.alerts.include_peer);
        assert_eq!(request.alerts.cooldown_secs, 60);
        let snapshot:StockMarketSnapshot=serde_json::from_value(serde_json::json!({"security":null,"tokens":[],"connected":false,"books":[],"reference":null,"problem":null,"observedAtMs":0})).unwrap();
        assert_eq!(snapshot.alerts, StockAlertRuntime::default());
    }
}
