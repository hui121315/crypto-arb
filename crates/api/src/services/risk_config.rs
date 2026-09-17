use crate::trading_service::TradingService;
use common::AppError;
use shared_types::{
    normalized_venue_name, AutoProfitCloseConfigPatch, ProtectedPositionFingerprint,
    RiskConfigPatch, TradingRiskStatus,
};
use std::collections::BTreeSet;
use trading::RiskConfig;

mod auto_profit_close;
use auto_profit_close::NormalizedAutoProfitClosePatch;

const MAX_PROTECTED_POSITIONS: usize = 32;

pub(crate) fn update(
    trading_service: &TradingService,
    patch: RiskConfigPatch,
) -> Result<RiskConfig, AppError> {
    let normalized = NormalizedRiskConfigPatch::new(patch)?;
    Ok(trading_service.update_risk_config(move |config| normalized.apply(config)))
}

pub(crate) fn snapshot(config: &RiskConfig) -> TradingRiskStatus {
    TradingRiskStatus {
        live_trading_enabled: config.live_trading_enabled,
        kill_switch_active: config.kill_switch_active,
        max_order_notional: config.max_order_notional,
        max_open_orders: config.max_open_orders,
        max_hedge_imbalance_pct: config.max_hedge_imbalance_pct,
        liquidation_warn_pct: config.liquidation_warn_pct,
        liquidation_danger_pct: config.liquidation_danger_pct,
        allowed_exchanges: config.allowed_exchanges.iter().cloned().collect(),
        allowed_symbols: config.allowed_symbols.iter().cloned().collect(),
        protected_positions: config.protected_positions.clone(),
        auto_profit_close: config.auto_profit_close.clone(),
    }
}

pub(crate) fn restored_config(status: &TradingRiskStatus) -> Result<RiskConfig, AppError> {
    let mut config = RiskConfig::default();
    NormalizedRiskConfigPatch::new(full_snapshot_patch(status))?.apply(&mut config);
    config.kill_switch_active = status.kill_switch_active;
    config.liquidation_warn_pct = positive_f64("liquidationWarnPct", status.liquidation_warn_pct)?;
    config.liquidation_danger_pct =
        positive_f64("liquidationDangerPct", status.liquidation_danger_pct)?;
    if config.liquidation_danger_pct > config.liquidation_warn_pct {
        return Err(AppError::BadRequest(
            "liquidationDangerPct must not exceed liquidationWarnPct".to_owned(),
        ));
    }
    Ok(config)
}

fn full_snapshot_patch(status: &TradingRiskStatus) -> RiskConfigPatch {
    let exit = &status.auto_profit_close;
    RiskConfigPatch {
        max_order_notional: Some(status.max_order_notional),
        max_open_orders: Some(status.max_open_orders),
        max_hedge_imbalance_pct: Some(status.max_hedge_imbalance_pct),
        allowed_exchanges: Some(status.allowed_exchanges.clone()),
        allowed_symbols: Some(status.allowed_symbols.clone()),
        protected_positions: Some(status.protected_positions.clone()),
        auto_profit_close: Some(AutoProfitCloseConfigPatch {
            enabled: Some(exit.enabled),
            min_net_profit_usd: Some(exit.min_net_profit_usd),
            min_roi_bps: Some(exit.min_roi_bps),
            exit_buffer_bps: Some(exit.exit_buffer_bps),
            stop_loss_enabled: Some(exit.stop_loss_enabled),
            max_net_loss_usd: Some(exit.max_net_loss_usd),
            max_loss_roi_bps: Some(exit.max_loss_roi_bps),
            liquidation_guard_enabled: Some(exit.liquidation_guard_enabled),
            liquidation_exit_distance_pct: Some(exit.liquidation_exit_distance_pct),
            confirmation_samples: Some(exit.confirmation_samples),
            cooldown_secs: Some(exit.cooldown_secs),
        }),
    }
}

#[derive(Debug)]
struct NormalizedRiskConfigPatch {
    max_order_notional: Option<f64>,
    max_open_orders: Option<usize>,
    max_hedge_imbalance_pct: Option<f64>,
    allowed_exchanges: Option<BTreeSet<String>>,
    allowed_symbols: Option<BTreeSet<String>>,
    protected_positions: Option<Vec<ProtectedPositionFingerprint>>,
    auto_profit_close: Option<NormalizedAutoProfitClosePatch>,
}

impl NormalizedRiskConfigPatch {
    fn new(patch: RiskConfigPatch) -> Result<Self, AppError> {
        Ok(Self {
            max_order_notional: patch
                .max_order_notional
                .map(|value| positive_f64("maxOrderNotional", value))
                .transpose()?,
            max_open_orders: patch
                .max_open_orders
                .map(|value| non_zero_usize("maxOpenOrders", value))
                .transpose()?,
            max_hedge_imbalance_pct: patch
                .max_hedge_imbalance_pct
                .map(|value| ratio_f64("maxHedgeImbalancePct", value))
                .transpose()?,
            allowed_exchanges: patch
                .allowed_exchanges
                .map(|values| normalized_venue_set("allowedExchanges", values))
                .transpose()?,
            allowed_symbols: patch
                .allowed_symbols
                .map(|values| normalized_set("allowedSymbols", values))
                .transpose()?,
            protected_positions: patch
                .protected_positions
                .map(normalized_protected_positions)
                .transpose()?,
            auto_profit_close: patch
                .auto_profit_close
                .as_ref()
                .map(NormalizedAutoProfitClosePatch::new)
                .transpose()?,
        })
    }

    fn apply(self, config: &mut RiskConfig) {
        if let Some(value) = self.max_order_notional {
            config.max_order_notional = value;
        }
        if let Some(value) = self.max_open_orders {
            config.max_open_orders = value;
        }
        if let Some(value) = self.max_hedge_imbalance_pct {
            config.max_hedge_imbalance_pct = value;
        }
        if let Some(values) = self.allowed_exchanges {
            config.allowed_exchanges = values;
        }
        if let Some(values) = self.allowed_symbols {
            config.allowed_symbols = values;
        }
        if let Some(values) = self.protected_positions {
            config.protected_positions = values;
        }
        if let Some(patch) = self.auto_profit_close {
            patch.apply(&mut config.auto_profit_close);
        }
    }
}

fn normalized_protected_positions(
    values: Vec<ProtectedPositionFingerprint>,
) -> Result<Vec<ProtectedPositionFingerprint>, AppError> {
    if values.len() > MAX_PROTECTED_POSITIONS {
        return Err(AppError::BadRequest(format!(
            "protectedPositions must contain at most {MAX_PROTECTED_POSITIONS} entries"
        )));
    }
    let mut normalized = Vec::with_capacity(values.len());
    let mut identities = BTreeSet::new();
    for mut value in values {
        value.venue = required_text(
            "protectedPositions.venue",
            normalized_venue_name(&value.venue),
        )?;
        value.canonical_symbol =
            required_lower_text("protectedPositions.canonicalSymbol", value.canonical_symbol)?;
        value.native_symbol =
            required_lower_text("protectedPositions.nativeSymbol", value.native_symbol)?;
        value.side = required_lower_text("protectedPositions.side", value.side)?;
        if !matches!(value.side.as_str(), "long" | "short") {
            return Err(AppError::BadRequest(
                "protectedPositions.side must be long or short".to_owned(),
            ));
        }
        value.quantity = positive_f64("protectedPositions.quantity", value.quantity)?;
        value.entry_price = positive_f64("protectedPositions.entryPrice", value.entry_price)?;
        value.position_mode = value
            .position_mode
            .take()
            .map(|mode| mode.trim().to_ascii_lowercase())
            .filter(|mode| !mode.is_empty());
        value.opening_identity =
            required_text("protectedPositions.openingIdentity", value.opening_identity)?;
        value.source = required_text("protectedPositions.source", value.source)?;
        if value.captured_at_ms <= 0 {
            return Err(AppError::BadRequest(
                "protectedPositions.capturedAtMs must be positive".to_owned(),
            ));
        }
        let identity = format!(
            "{}\0{}\0{}\0{}",
            value.venue, value.native_symbol, value.side, value.opening_identity
        );
        if !identities.insert(identity) {
            return Err(AppError::BadRequest(
                "protectedPositions must not contain duplicate opening identities".to_owned(),
            ));
        }
        normalized.push(value);
    }
    Ok(normalized)
}

fn required_text(field: &str, value: impl Into<String>) -> Result<String, AppError> {
    let value = value.into();
    let value = value.trim();
    if value.is_empty() {
        Err(AppError::BadRequest(format!("{field} must not be empty")))
    } else {
        Ok(value.to_owned())
    }
}

fn required_lower_text(field: &str, value: String) -> Result<String, AppError> {
    required_text(field, value).map(|value| value.to_ascii_lowercase())
}

fn positive_f64(field: &str, value: f64) -> Result<f64, AppError> {
    if value.is_finite() && value > 0.0 {
        Ok(value)
    } else {
        Err(AppError::BadRequest(format!("{field} must be positive")))
    }
}

fn ratio_f64(field: &str, value: f64) -> Result<f64, AppError> {
    if value.is_finite() && (0.0..=1.0).contains(&value) {
        Ok(value)
    } else {
        Err(AppError::BadRequest(format!(
            "{field} must be between 0 and 1"
        )))
    }
}

fn non_zero_usize(field: &str, value: usize) -> Result<usize, AppError> {
    if value > 0 {
        Ok(value)
    } else {
        Err(AppError::BadRequest(format!("{field} must be positive")))
    }
}

fn normalized_set(field: &str, values: Vec<String>) -> Result<BTreeSet<String>, AppError> {
    normalized_set_with(field, values, str::to_ascii_lowercase)
}

fn normalized_venue_set(field: &str, values: Vec<String>) -> Result<BTreeSet<String>, AppError> {
    normalized_set_with(field, values, normalized_venue_name)
}

fn normalized_set_with(
    field: &str,
    values: Vec<String>,
    normalize: impl Fn(&str) -> String,
) -> Result<BTreeSet<String>, AppError> {
    let mut set = BTreeSet::new();
    for value in values {
        let value = normalize(value.trim());
        if value.is_empty() {
            return Err(AppError::BadRequest(format!(
                "{field} must not contain empty entries"
            )));
        }
        set.insert(value);
    }
    Ok(set)
}

#[cfg(test)]
mod tests;
