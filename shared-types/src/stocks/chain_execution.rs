use super::*;

pub fn stock_chain_quantity(raw: &str, decimals: u8) -> Option<String> {
    rust_decimal::Decimal::try_from_i128_with_scale(raw.parse().ok()?, u32::from(decimals))
        .ok()
        .map(|n| n.normalize().to_string())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockChainAssetChange {
    pub mint: String,
    pub decimals: u8,
    pub raw_change: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockChainReceipt {
    pub transaction_id: String,
    pub slot: u64,
    pub succeeded: bool,
    pub fee_payer: String,
    pub network_fee_lamports: String,
    // This is the wallet's complete native delta, already including fees/rent.
    pub wallet_native_change_lamports: String,
    pub asset_changes: Vec<StockChainAssetChange>,
    pub within_plan: bool,
    pub problems: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockChainSubmission {
    pub submitted_at_ms: i64,
    pub wallet_signature: String,
    // Known locally only when the first signature is already present.
    pub transaction_id: Option<String>,
    pub provider_transaction_id: Option<String>,
    pub provider_acknowledged: bool,
    pub receipt: Option<StockChainReceipt>,
    pub recheck_attempts: u32,
    pub next_recheck_at_ms: i64,
    pub search_before: Option<String>,
    pub problem: Option<String>,
}
