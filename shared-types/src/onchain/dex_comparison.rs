use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainDexComparisonConfig {
    pub enabled: bool,
    pub peer_provider: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainDexComparisonConfigPatch {
    pub enabled: Option<bool>,
    pub peer_provider: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnchainDexComparisonQuality {
    #[default]
    Disabled,
    Pending,
    Fresh,
    NoNetProfit,
    Stale,
    DuplicateRoute,
    EvidencePending,
    UpstreamUnavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnchainDexComparisonDirection {
    BuyPrimarySellPeer,
    BuyPeerSellPrimary,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnchainDexRouteIdentity {
    ProvenDistinct,
    Duplicate,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainDexRouteComparison {
    pub direction: OnchainDexComparisonDirection,
    pub buy_provider: String,
    pub sell_provider: String,
    pub input_quote_amount_raw: String,
    pub acquired_base_amount_raw: String,
    pub output_quote_amount_raw: String,
    pub gross_return_bps: f64,
    pub execution_buffer_bps: f64,
    pub gas_usd: f64,
    pub gas_bps: Option<f64>,
    pub total_cost_bps: Option<f64>,
    pub net_return_bps: Option<f64>,
    pub buy_router: Option<String>,
    pub sell_router: Option<String>,
    pub route_identity: OnchainDexRouteIdentity,
    pub executable: bool,
    pub problem: Option<String>,
    pub observed_at_ms: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainDexComparisonSnapshot {
    pub primary_provider: String,
    pub peer_provider: String,
    pub quality: OnchainDexComparisonQuality,
    pub routes: Vec<OnchainDexRouteComparison>,
    pub quote_observed_at_ms: Option<i64>,
    pub quote_latency_ms: Option<i64>,
    pub problem: Option<String>,
    pub observed_at_ms: i64,
}
