use serde::{Deserialize, Serialize};

pub const DEFAULT_AUTO_PROFIT_CLOSE_MIN_NET_PROFIT_USD: f64 = 5.0;
pub const DEFAULT_AUTO_PROFIT_CLOSE_MIN_ROI_BPS: f64 = 10.0;
pub const DEFAULT_AUTO_PROFIT_CLOSE_EXIT_BUFFER_BPS: f64 = 5.0;
pub const DEFAULT_AUTO_PROFIT_CLOSE_CONFIRMATION_SAMPLES: u16 = 3;
pub const DEFAULT_AUTO_PROFIT_CLOSE_COOLDOWN_SECS: u64 = 60;
pub const DEFAULT_AUTO_STOP_LOSS_MAX_NET_LOSS_USD: f64 = 25.0;
pub const DEFAULT_AUTO_STOP_LOSS_MAX_ROI_BPS: f64 = 100.0;
pub const DEFAULT_LIQUIDATION_EXIT_DISTANCE_PCT: f64 = 8.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoProfitCloseConfig {
    pub enabled: bool,
    pub min_net_profit_usd: f64,
    pub min_roi_bps: f64,
    pub exit_buffer_bps: f64,
    #[serde(default)]
    pub stop_loss_enabled: bool,
    #[serde(default = "default_stop_loss_max_net_loss_usd")]
    pub max_net_loss_usd: f64,
    #[serde(default = "default_stop_loss_max_roi_bps")]
    pub max_loss_roi_bps: f64,
    #[serde(default)]
    pub liquidation_guard_enabled: bool,
    #[serde(default = "default_liquidation_exit_distance_pct")]
    pub liquidation_exit_distance_pct: f64,
    pub confirmation_samples: u16,
    pub cooldown_secs: u64,
}

impl Default for AutoProfitCloseConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            min_net_profit_usd: DEFAULT_AUTO_PROFIT_CLOSE_MIN_NET_PROFIT_USD,
            min_roi_bps: DEFAULT_AUTO_PROFIT_CLOSE_MIN_ROI_BPS,
            exit_buffer_bps: DEFAULT_AUTO_PROFIT_CLOSE_EXIT_BUFFER_BPS,
            stop_loss_enabled: false,
            max_net_loss_usd: DEFAULT_AUTO_STOP_LOSS_MAX_NET_LOSS_USD,
            max_loss_roi_bps: DEFAULT_AUTO_STOP_LOSS_MAX_ROI_BPS,
            liquidation_guard_enabled: false,
            liquidation_exit_distance_pct: DEFAULT_LIQUIDATION_EXIT_DISTANCE_PCT,
            confirmation_samples: DEFAULT_AUTO_PROFIT_CLOSE_CONFIRMATION_SAMPLES,
            cooldown_secs: DEFAULT_AUTO_PROFIT_CLOSE_COOLDOWN_SECS,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoProfitCloseConfigPatch {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub min_net_profit_usd: Option<f64>,
    #[serde(default)]
    pub min_roi_bps: Option<f64>,
    #[serde(default)]
    pub exit_buffer_bps: Option<f64>,
    #[serde(default)]
    pub stop_loss_enabled: Option<bool>,
    #[serde(default)]
    pub max_net_loss_usd: Option<f64>,
    #[serde(default)]
    pub max_loss_roi_bps: Option<f64>,
    #[serde(default)]
    pub liquidation_guard_enabled: Option<bool>,
    #[serde(default)]
    pub liquidation_exit_distance_pct: Option<f64>,
    #[serde(default)]
    pub confirmation_samples: Option<u16>,
    #[serde(default)]
    pub cooldown_secs: Option<u64>,
}

const fn default_stop_loss_max_net_loss_usd() -> f64 {
    DEFAULT_AUTO_STOP_LOSS_MAX_NET_LOSS_USD
}

const fn default_stop_loss_max_roi_bps() -> f64 {
    DEFAULT_AUTO_STOP_LOSS_MAX_ROI_BPS
}

const fn default_liquidation_exit_distance_pct() -> f64 {
    DEFAULT_LIQUIDATION_EXIT_DISTANCE_PCT
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_disabled_and_conservative() {
        let config = AutoProfitCloseConfig::default();

        assert!(!config.enabled);
        assert!(config.min_net_profit_usd > 0.0);
        assert!(config.min_roi_bps > 0.0);
        assert!(config.exit_buffer_bps > 0.0);
        assert!(!config.stop_loss_enabled);
        assert!(config.max_net_loss_usd > 0.0);
        assert!(config.max_loss_roi_bps > 0.0);
        assert!(!config.liquidation_guard_enabled);
        assert!(config.liquidation_exit_distance_pct > 0.0);
        assert!(config.confirmation_samples > 1);
        assert!(config.cooldown_secs >= 30);
    }

    #[test]
    fn config_uses_camel_case_contract() {
        let encoded = serde_json::to_value(AutoProfitCloseConfig::default())
            .expect("auto profit close config encodes");

        assert_eq!(encoded["enabled"], false);
        assert_eq!(encoded["minNetProfitUsd"], 5.0);
        assert_eq!(encoded["minRoiBps"], 10.0);
        assert_eq!(encoded["exitBufferBps"], 5.0);
        assert_eq!(encoded["stopLossEnabled"], false);
        assert_eq!(encoded["maxNetLossUsd"], 25.0);
        assert_eq!(encoded["maxLossRoiBps"], 100.0);
        assert_eq!(encoded["liquidationGuardEnabled"], false);
        assert_eq!(encoded["liquidationExitDistancePct"], 8.0);
        assert_eq!(encoded["confirmationSamples"], 3);
        assert_eq!(encoded["cooldownSecs"], 60);
    }

    #[test]
    fn legacy_profit_only_config_keeps_new_protections_disabled() {
        let decoded = serde_json::from_value::<AutoProfitCloseConfig>(serde_json::json!({
            "enabled": true,
            "minNetProfitUsd": 5.0,
            "minRoiBps": 10.0,
            "exitBufferBps": 5.0,
            "confirmationSamples": 3,
            "cooldownSecs": 60
        }))
        .expect("legacy auto profit config decodes");

        assert!(decoded.enabled);
        assert!(!decoded.stop_loss_enabled);
        assert!(!decoded.liquidation_guard_enabled);
        assert_eq!(decoded.max_net_loss_usd, 25.0);
        assert_eq!(decoded.liquidation_exit_distance_pct, 8.0);
    }
}
