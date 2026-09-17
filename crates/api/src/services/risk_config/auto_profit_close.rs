use common::AppError;
use shared_types::{AutoProfitCloseConfig, AutoProfitCloseConfigPatch};

#[derive(Debug)]
pub(super) struct NormalizedAutoProfitClosePatch {
    enabled: Option<bool>,
    min_net_profit_usd: Option<f64>,
    min_roi_bps: Option<f64>,
    exit_buffer_bps: Option<f64>,
    stop_loss_enabled: Option<bool>,
    max_net_loss_usd: Option<f64>,
    max_loss_roi_bps: Option<f64>,
    liquidation_guard_enabled: Option<bool>,
    liquidation_exit_distance_pct: Option<f64>,
    confirmation_samples: Option<u16>,
    cooldown_secs: Option<u64>,
}

impl NormalizedAutoProfitClosePatch {
    pub(super) fn new(patch: &AutoProfitCloseConfigPatch) -> Result<Self, AppError> {
        Ok(Self {
            enabled: patch.enabled,
            min_net_profit_usd: patch
                .min_net_profit_usd
                .map(|value| positive_f64("autoProfitClose.minNetProfitUsd", value))
                .transpose()?,
            min_roi_bps: patch
                .min_roi_bps
                .map(|value| bounded_f64("autoProfitClose.minRoiBps", value, 0.01, 10_000.0))
                .transpose()?,
            exit_buffer_bps: patch
                .exit_buffer_bps
                .map(|value| bounded_f64("autoProfitClose.exitBufferBps", value, 0.0, 1_000.0))
                .transpose()?,
            stop_loss_enabled: patch.stop_loss_enabled,
            max_net_loss_usd: patch
                .max_net_loss_usd
                .map(|value| positive_f64("autoProfitClose.maxNetLossUsd", value))
                .transpose()?,
            max_loss_roi_bps: patch
                .max_loss_roi_bps
                .map(|value| bounded_f64("autoProfitClose.maxLossRoiBps", value, 0.01, 10_000.0))
                .transpose()?,
            liquidation_guard_enabled: patch.liquidation_guard_enabled,
            liquidation_exit_distance_pct: patch
                .liquidation_exit_distance_pct
                .map(|value| {
                    bounded_f64(
                        "autoProfitClose.liquidationExitDistancePct",
                        value,
                        0.1,
                        100.0,
                    )
                })
                .transpose()?,
            confirmation_samples: patch
                .confirmation_samples
                .map(|value| bounded_u16("autoProfitClose.confirmationSamples", value, 2, 30))
                .transpose()?,
            cooldown_secs: patch
                .cooldown_secs
                .map(|value| bounded_u64("autoProfitClose.cooldownSecs", value, 10, 3_600))
                .transpose()?,
        })
    }

    pub(super) fn apply(self, config: &mut AutoProfitCloseConfig) {
        if let Some(value) = self.enabled {
            config.enabled = value;
        }
        if let Some(value) = self.min_net_profit_usd {
            config.min_net_profit_usd = value;
        }
        if let Some(value) = self.min_roi_bps {
            config.min_roi_bps = value;
        }
        if let Some(value) = self.exit_buffer_bps {
            config.exit_buffer_bps = value;
        }
        if let Some(value) = self.stop_loss_enabled {
            config.stop_loss_enabled = value;
        }
        if let Some(value) = self.max_net_loss_usd {
            config.max_net_loss_usd = value;
        }
        if let Some(value) = self.max_loss_roi_bps {
            config.max_loss_roi_bps = value;
        }
        if let Some(value) = self.liquidation_guard_enabled {
            config.liquidation_guard_enabled = value;
        }
        if let Some(value) = self.liquidation_exit_distance_pct {
            config.liquidation_exit_distance_pct = value;
        }
        if let Some(value) = self.confirmation_samples {
            config.confirmation_samples = value;
        }
        if let Some(value) = self.cooldown_secs {
            config.cooldown_secs = value;
        }
    }
}

fn positive_f64(field: &str, value: f64) -> Result<f64, AppError> {
    if value.is_finite() && value > 0.0 {
        Ok(value)
    } else {
        Err(AppError::BadRequest(format!("{field} must be positive")))
    }
}

fn bounded_f64(field: &str, value: f64, min: f64, max: f64) -> Result<f64, AppError> {
    if value.is_finite() && (min..=max).contains(&value) {
        Ok(value)
    } else {
        Err(AppError::BadRequest(format!(
            "{field} must be between {min} and {max}"
        )))
    }
}

fn bounded_u16(field: &str, value: u16, min: u16, max: u16) -> Result<u16, AppError> {
    if (min..=max).contains(&value) {
        Ok(value)
    } else {
        Err(AppError::BadRequest(format!(
            "{field} must be between {min} and {max}"
        )))
    }
}

fn bounded_u64(field: &str, value: u64, min: u64, max: u64) -> Result<u64, AppError> {
    if (min..=max).contains(&value) {
        Ok(value)
    } else {
        Err(AppError::BadRequest(format!(
            "{field} must be between {min} and {max}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_and_applies_patch() -> Result<(), AppError> {
        let normalized = NormalizedAutoProfitClosePatch::new(&AutoProfitCloseConfigPatch {
            enabled: Some(true),
            min_net_profit_usd: Some(12.0),
            min_roi_bps: Some(25.0),
            exit_buffer_bps: Some(8.0),
            stop_loss_enabled: Some(true),
            max_net_loss_usd: Some(30.0),
            max_loss_roi_bps: Some(120.0),
            liquidation_guard_enabled: Some(true),
            liquidation_exit_distance_pct: Some(9.0),
            confirmation_samples: Some(4),
            cooldown_secs: Some(90),
        })?;
        let mut config = AutoProfitCloseConfig::default();

        normalized.apply(&mut config);

        assert!(config.enabled);
        assert_eq!(config.min_net_profit_usd, 12.0);
        assert_eq!(config.min_roi_bps, 25.0);
        assert_eq!(config.exit_buffer_bps, 8.0);
        assert!(config.stop_loss_enabled);
        assert_eq!(config.max_net_loss_usd, 30.0);
        assert_eq!(config.max_loss_roi_bps, 120.0);
        assert!(config.liquidation_guard_enabled);
        assert_eq!(config.liquidation_exit_distance_pct, 9.0);
        assert_eq!(config.confirmation_samples, 4);
        assert_eq!(config.cooldown_secs, 90);
        Ok(())
    }

    #[test]
    fn rejects_unsafe_limits() {
        let result = NormalizedAutoProfitClosePatch::new(&AutoProfitCloseConfigPatch {
            enabled: Some(true),
            min_net_profit_usd: Some(0.0),
            min_roi_bps: Some(0.0),
            exit_buffer_bps: Some(-1.0),
            stop_loss_enabled: Some(true),
            max_net_loss_usd: Some(0.0),
            max_loss_roi_bps: Some(0.0),
            liquidation_guard_enabled: Some(true),
            liquidation_exit_distance_pct: Some(0.0),
            confirmation_samples: Some(1),
            cooldown_secs: Some(1),
        });

        assert!(matches!(result, Err(AppError::BadRequest(_))));
    }
}
