use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnchainPathKind {
    #[default]
    DirectTwoLeg,
    QuoteConvertedThreeLeg,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnchainPathAvailability {
    ReadyToBuild,
    SetupRequired,
    Replenishable,
    TransferUnprofitable,
    InventoryRequired,
    #[default]
    EvidencePending,
    MonitoringOnly,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnchainTransferDirection {
    WithdrawToChain,
    DepositToCex,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnchainTransferStatus {
    Ready,
    Refreshing,
    #[default]
    Unknown,
    Blocked,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainTransferEvidence {
    pub direction: OnchainTransferDirection,
    pub venue: String,
    pub asset: String,
    pub chain: String,
    pub network: Option<String>,
    pub amount: f64,
    /// Compiled once with network minimums and asset precision; fees use this amount.
    #[serde(default)]
    pub amount_exact: Option<String>,
    pub status: OnchainTransferStatus,
    pub fee: Option<f64>,
    #[serde(default)]
    pub fee_exact: Option<String>,
    pub minimum: Option<f64>,
    #[serde(default)]
    pub minimum_exact: Option<String>,
    #[serde(default)]
    pub amount_step: Option<String>,
    pub requires_tag: bool,
    pub contract_verified: bool,
    #[serde(default)]
    pub credit_confirmations: Option<u64>,
    #[serde(default)]
    pub unlock_confirmations: Option<u64>,
    #[serde(default)]
    pub network_status: Option<String>,
    pub source: Option<String>,
    pub observed_at_ms: Option<i64>,
    pub problem: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnchainPathLegKind {
    OnchainSwap,
    CexSpot,
    CexQuoteConversion,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainPathLeg {
    pub kind: OnchainPathLegKind,
    pub venue: String,
    pub from_asset: String,
    pub to_asset: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainPathReadiness {
    pub kind: OnchainPathKind,
    pub availability: OnchainPathAvailability,
    pub legs: Vec<OnchainPathLeg>,
    #[serde(default)]
    pub replenishment: Vec<OnchainTransferEvidence>,
    #[serde(default)]
    pub transfer_cost_usd: Option<f64>,
    #[serde(default)]
    pub post_transfer_net_profit_usd: Option<f64>,
    pub summary: String,
}
