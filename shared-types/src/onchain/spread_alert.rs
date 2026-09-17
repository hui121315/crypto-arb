use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnchainSpreadAlertMode {
    #[default]
    VerifiedNet,
    RawObservation,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainSpreadAlertConfig {
    pub enabled: bool,
    #[serde(default)]
    pub mode: OnchainSpreadAlertMode,
    pub min_net_spread_bps: f64,
    #[serde(default = "default_min_raw_spread_bps")]
    pub min_raw_spread_bps: f64,
    pub cooldown_ms: i64,
}

impl Default for OnchainSpreadAlertConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: OnchainSpreadAlertMode::VerifiedNet,
            min_net_spread_bps: 20.0,
            min_raw_spread_bps: default_min_raw_spread_bps(),
            cooldown_ms: 300_000,
        }
    }
}

const fn default_min_raw_spread_bps() -> f64 {
    20.0
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainSpreadAlertConfigPatch {
    pub enabled: Option<bool>,
    pub mode: Option<OnchainSpreadAlertMode>,
    pub min_net_spread_bps: Option<f64>,
    pub min_raw_spread_bps: Option<f64>,
    pub cooldown_ms: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_config_defaults_to_verified_net_alerts() {
        let config: OnchainSpreadAlertConfig =
            serde_json::from_str(r#"{"enabled":true,"minNetSpreadBps":25.0,"cooldownMs":60000}"#)
                .expect("legacy spread alert config should deserialize");

        assert_eq!(config.mode, OnchainSpreadAlertMode::VerifiedNet);
        assert_eq!(config.min_raw_spread_bps, 20.0);
    }
}
