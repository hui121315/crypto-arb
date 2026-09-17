use leptos::prelude::*;
use shared_types::AutoProfitCloseConfig;

use super::draft::{finite_number, AutomationProtectionDraft};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ProtectionCapitalAssessment {
    pub message: String,
    pub needs_calibration: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct SmallLiveProtectionPreset {
    take_profit_enabled: bool,
    min_profit_usd: f64,
    min_profit_pct: f64,
    stop_loss_enabled: bool,
    max_loss_usd: f64,
    max_loss_pct: f64,
    liquidation_guard_enabled: bool,
    liquidation_distance_pct: f64,
}

const MAX_TAKE_PROFIT_CAPITAL_RATIO: f64 = 0.10;
const MAX_STOP_LOSS_CAPITAL_RATIO: f64 = 1.0;

impl AutomationProtectionDraft {
    pub(super) fn capital_assessment(
        self,
        capital_usd: f64,
    ) -> Option<ProtectionCapitalAssessment> {
        assess_protection_capital(
            capital_usd,
            finite_number(&self.min_profit_usd.get()),
            finite_number(&self.max_loss_usd.get()),
        )
    }

    pub(super) fn apply_small_live_preset(self, capital_usd: f64) {
        let preset = small_live_protection_preset(capital_usd);
        self.take_profit.set(preset.take_profit_enabled);
        self.min_profit_usd
            .set(compact_decimal(preset.min_profit_usd));
        self.min_profit_bps
            .set(compact_decimal(preset.min_profit_pct));
        self.stop_loss.set(preset.stop_loss_enabled);
        self.max_loss_usd.set(compact_decimal(preset.max_loss_usd));
        self.max_loss_bps.set(compact_decimal(preset.max_loss_pct));
        self.liquidation_guard.set(preset.liquidation_guard_enabled);
        self.liquidation_distance_pct
            .set(compact_decimal(preset.liquidation_distance_pct));
    }
}

pub(super) fn protection_capital_ready(config: &AutoProfitCloseConfig, capital_usd: f64) -> bool {
    let Some(capital_usd) = valid_capital(capital_usd) else {
        return false;
    };
    let enabled = config.enabled || config.stop_loss_enabled || config.liquidation_guard_enabled;
    enabled
        && (!config.enabled
            || config.min_net_profit_usd <= capital_usd * MAX_TAKE_PROFIT_CAPITAL_RATIO)
        && (!config.stop_loss_enabled
            || config.max_net_loss_usd <= capital_usd * MAX_STOP_LOSS_CAPITAL_RATIO)
}

fn small_live_protection_preset(capital_usd: f64) -> SmallLiveProtectionPreset {
    let capital_usd = valid_capital(capital_usd).unwrap_or(10.0);
    SmallLiveProtectionPreset {
        take_profit_enabled: true,
        min_profit_usd: (capital_usd * 0.002).max(0.02),
        min_profit_pct: 0.1,
        stop_loss_enabled: true,
        max_loss_usd: (capital_usd * 0.025).max(0.25),
        max_loss_pct: 2.0,
        liquidation_guard_enabled: true,
        liquidation_distance_pct: 12.0,
    }
}

fn assess_protection_capital(
    capital_usd: f64,
    min_profit_usd: Option<f64>,
    max_loss_usd: Option<f64>,
) -> Option<ProtectionCapitalAssessment> {
    let capital_usd = valid_capital(capital_usd)?;
    let min_profit_usd = min_profit_usd.filter(|value| value.is_finite() && *value > 0.0)?;
    let max_loss_usd = max_loss_usd.filter(|value| value.is_finite() && *value > 0.0)?;
    let profit_pct = min_profit_usd / capital_usd * 100.0;
    let loss_pct = max_loss_usd / capital_usd * 100.0;
    let needs_calibration = profit_pct > 10.0 || loss_pct > 100.0;
    let suffix = if needs_calibration {
        "；与小额实盘资金不匹配"
    } else {
        "；资金占比已校准（草稿）"
    };
    Some(ProtectionCapitalAssessment {
        message: format!(
            "止盈 ${min_profit_usd:.2}（{profit_pct:.2}%），止损 ${max_loss_usd:.2}（{loss_pct:.2}%）{suffix}"
        ),
        needs_calibration,
    })
}

fn valid_capital(value: f64) -> Option<f64> {
    (value.is_finite() && value > 0.0).then_some(value)
}

fn compact_decimal(value: f64) -> String {
    let value = format!("{value:.4}");
    value.trim_end_matches('0').trim_end_matches('.').to_owned()
}

#[cfg(test)]
mod tests {
    use super::{
        assess_protection_capital, protection_capital_ready, small_live_protection_preset,
    };
    use shared_types::AutoProfitCloseConfig;

    #[test]
    fn small_live_preset_scales_with_the_automation_capital() {
        let ten_dollar = small_live_protection_preset(10.0);
        assert_eq!(ten_dollar.min_profit_usd, 0.02);
        assert_eq!(ten_dollar.max_loss_usd, 0.25);
        assert!(ten_dollar.take_profit_enabled);
        assert!(ten_dollar.stop_loss_enabled);
        assert!(ten_dollar.liquidation_guard_enabled);
        assert_eq!(ten_dollar.min_profit_pct, 0.1);
        assert_eq!(ten_dollar.max_loss_pct, 2.0);
        assert_eq!(ten_dollar.liquidation_distance_pct, 12.0);

        let hundred_dollar = small_live_protection_preset(100.0);
        assert_eq!(hundred_dollar.min_profit_usd, 0.2);
        assert_eq!(hundred_dollar.max_loss_usd, 2.5);
    }

    #[test]
    fn capital_assessment_exposes_unreachable_legacy_thresholds() -> Result<(), &'static str> {
        let assessment =
            assess_protection_capital(10.0, Some(5.0), Some(25.0)).ok_or("valid assessment")?;
        assert!(assessment.needs_calibration);
        assert!(assessment.message.contains("50.00%"));
        assert!(assessment.message.contains("250.00%"));

        let calibrated =
            assess_protection_capital(10.0, Some(0.02), Some(0.25)).ok_or("valid assessment")?;
        assert!(!calibrated.needs_calibration);
        Ok(())
    }

    #[test]
    fn saved_protection_must_match_automation_capital() {
        let mut config = AutoProfitCloseConfig {
            enabled: true,
            ..AutoProfitCloseConfig::default()
        };
        assert!(!protection_capital_ready(&config, 10.0));

        config.min_net_profit_usd = 0.02;
        config.stop_loss_enabled = true;
        config.max_net_loss_usd = 0.25;
        config.liquidation_guard_enabled = true;
        assert!(protection_capital_ready(&config, 10.0));
    }
}
