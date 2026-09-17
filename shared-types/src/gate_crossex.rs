use crate::{ApiProblem, InstrumentListingStatus};
use serde::{Deserialize, Serialize};

pub const GATE_CROSSEX_SELECTED_ROUTE_LIMIT: usize = 64;
pub const DEFAULT_GATE_CROSSEX_MIN_GROSS_SPREAD_PCT: f64 = 0.10;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateCrossExMode {
    #[default]
    Disabled,
    Monitor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateCrossExProduct {
    Spot,
    Future,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateCrossExRuntimeState {
    #[default]
    Disabled,
    WaitingForRegistry,
    Warming,
    Live,
    Degraded,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GateCrossExModeConfig {
    pub mode: GateCrossExMode,
    #[serde(default)]
    pub selected_routes: Vec<String>,
    pub min_gross_spread_pct: f64,
}

impl Default for GateCrossExModeConfig {
    fn default() -> Self {
        Self {
            mode: GateCrossExMode::Disabled,
            selected_routes: Vec::new(),
            min_gross_spread_pct: DEFAULT_GATE_CROSSEX_MIN_GROSS_SPREAD_PCT,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GateCrossExModeConfigPatch {
    #[serde(default)]
    pub mode: Option<GateCrossExMode>,
    #[serde(default)]
    pub selected_routes: Option<Vec<String>>,
    #[serde(default)]
    pub min_gross_spread_pct: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GateCrossExRouteCatalogRow {
    pub native_symbol: String,
    pub underlying_venue: String,
    pub product: GateCrossExProduct,
    pub base_asset: String,
    pub quote_asset: String,
    pub display_symbol: String,
    pub execution_supported: bool,
    pub listing_status: InstrumentListingStatus,
    pub source_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GateCrossExRouteCatalogResponse {
    pub routes: Vec<GateCrossExRouteCatalogRow>,
    pub total: usize,
    pub generated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GateCrossExRouteQuote {
    pub native_symbol: String,
    pub underlying_venue: String,
    pub product: GateCrossExProduct,
    pub base_asset: String,
    pub quote_asset: String,
    pub bid: f64,
    pub ask: f64,
    pub last: f64,
    pub observed_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GateCrossExSpreadCandidate {
    pub product: GateCrossExProduct,
    pub base_asset: String,
    pub quote_asset: String,
    pub long_route: String,
    pub short_route: String,
    pub long_ask: f64,
    pub short_bid: f64,
    pub gross_spread_pct: f64,
    pub synchronized_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GateCrossExModeSnapshot {
    pub config: GateCrossExModeConfig,
    pub runtime_state: GateCrossExRuntimeState,
    pub catalog_count: usize,
    pub selected_count: usize,
    pub live_count: usize,
    #[serde(default)]
    pub routes: Vec<GateCrossExRouteQuote>,
    #[serde(default)]
    pub candidates: Vec<GateCrossExSpreadCandidate>,
    pub observed_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<ApiProblem>,
}

impl Default for GateCrossExModeSnapshot {
    fn default() -> Self {
        Self {
            config: GateCrossExModeConfig::default(),
            runtime_state: GateCrossExRuntimeState::Disabled,
            catalog_count: 0,
            selected_count: 0,
            live_count: 0,
            routes: Vec::new(),
            candidates: Vec::new(),
            observed_at_ms: 0,
            problem: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_disabled_and_use_percent_units() {
        let config = GateCrossExModeConfig::default();

        assert_eq!(config.mode, GateCrossExMode::Disabled);
        assert!(config.selected_routes.is_empty());
        assert_eq!(
            config.min_gross_spread_pct,
            DEFAULT_GATE_CROSSEX_MIN_GROSS_SPREAD_PCT
        );
    }
}
