use parking_lot::RwLock;
use shared_types::{
    is_hyperliquid_builder_venue, normalized_venue_name, venue_family, AutoProfitCloseConfig,
    ProtectedPositionFingerprint,
};
use std::collections::BTreeSet;
use std::sync::Arc;

mod checks;
mod evidence;
mod hedge;
mod protected_positions;
mod unwind;

const MARKET_NOTIONAL_BUFFER: f64 = 1.01;

#[derive(Debug, Clone)]
pub struct RiskConfig {
    pub live_trading_enabled: bool,
    pub kill_switch_active: bool,
    pub max_order_notional: f64,
    pub max_open_orders: usize,
    pub max_hedge_imbalance_pct: f64,
    pub liquidation_warn_pct: f64,
    pub liquidation_danger_pct: f64,
    pub allowed_exchanges: BTreeSet<String>,
    pub allowed_symbols: BTreeSet<String>,
    pub protected_positions: Vec<ProtectedPositionFingerprint>,
    pub auto_profit_close: AutoProfitCloseConfig,
}

impl Default for RiskConfig {
    fn default() -> Self {
        Self {
            live_trading_enabled: false,
            kill_switch_active: false,
            max_order_notional: 1_000.0,
            max_open_orders: 10,
            max_hedge_imbalance_pct: 0.01,
            liquidation_warn_pct: 15.0,
            liquidation_danger_pct: 8.0,
            allowed_exchanges: BTreeSet::new(),
            allowed_symbols: BTreeSet::new(),
            protected_positions: Vec::new(),
            auto_profit_close: AutoProfitCloseConfig::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RiskEngine {
    config: Arc<RwLock<RiskConfig>>,
}

impl RiskEngine {
    pub fn new(mut config: RiskConfig) -> Self {
        normalize_allowed_exchanges(&mut config.allowed_exchanges);
        normalize_allowed_symbols(&mut config.allowed_symbols);
        protected_positions::normalize(&mut config.protected_positions);
        Self {
            config: Arc::new(RwLock::new(config)),
        }
    }

    pub fn config(&self) -> RiskConfig {
        self.config.read().clone()
    }

    pub fn set_kill_switch(&self, active: bool) -> RiskConfig {
        let mut config = self.config.write();
        config.kill_switch_active = active;
        config.clone()
    }

    pub fn update_config(&self, update: impl FnOnce(&mut RiskConfig)) -> RiskConfig {
        let mut config = self.config.write();
        update(&mut config);
        normalize_allowed_exchanges(&mut config.allowed_exchanges);
        normalize_allowed_symbols(&mut config.allowed_symbols);
        protected_positions::normalize(&mut config.protected_positions);
        config.clone()
    }
}

pub fn exchange_allowed(allowed_exchanges: &BTreeSet<String>, exchange: &str) -> bool {
    if allowed_exchanges.is_empty() {
        return true;
    }

    let exchange = normalized_venue_name(exchange);
    if allowed_exchanges.contains(exchange.as_str()) {
        return true;
    }
    if is_hyperliquid_builder_venue(&exchange) {
        return false;
    }

    let family = venue_family(&exchange);
    family != exchange && allowed_exchanges.contains(family)
}

pub fn symbol_allowed(allowed_symbols: &BTreeSet<String>, symbol: &str) -> bool {
    allowed_symbols.is_empty() || allowed_symbols.contains(&symbol.trim().to_ascii_lowercase())
}

fn normalize_allowed_exchanges(values: &mut BTreeSet<String>) {
    *values = std::mem::take(values)
        .into_iter()
        .map(|value| normalized_venue_name(&value))
        .collect();
}

fn normalize_allowed_symbols(values: &mut BTreeSet<String>) {
    *values = std::mem::take(values)
        .into_iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .collect();
}

#[cfg(test)]
mod tests;
