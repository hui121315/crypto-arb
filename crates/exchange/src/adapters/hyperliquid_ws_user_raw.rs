//! Hyperliquid user WebSocket raw payload DTOs.

use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
pub(super) struct RawEnvelope {
    #[serde(default)]
    pub(super) channel: String,
    #[serde(default)]
    pub(super) data: Value,
}

#[derive(Debug, Deserialize)]
pub(super) struct RawOrderUpdate {
    pub(super) order: RawBasicOrder,
    pub(super) status: String,
    #[serde(rename = "statusTimestamp")]
    pub(super) status_timestamp: i64,
}

#[derive(Debug, Deserialize)]
pub(super) struct RawBasicOrder {
    pub(super) coin: String,
    pub(super) side: String,
    #[serde(rename = "limitPx")]
    pub(super) limit_px: Value,
    pub(super) sz: Value,
    pub(super) oid: i64,
    pub(super) timestamp: i64,
    #[serde(rename = "origSz")]
    pub(super) orig_sz: Value,
    #[serde(default)]
    pub(super) cloid: Option<String>,
    #[serde(default, rename = "reduceOnly")]
    pub(super) reduce_only: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub(super) struct RawUserFills {
    #[serde(default)]
    pub(super) fills: Vec<RawFill>,
}

#[derive(Debug, Deserialize)]
pub(super) struct RawFill {
    #[serde(default)]
    pub(super) coin: String,
    #[serde(default)]
    pub(super) px: Value,
    #[serde(default)]
    pub(super) sz: Value,
    #[serde(default)]
    pub(super) side: String,
    #[serde(default)]
    pub(super) time: i64,
    #[serde(default, rename = "closedPnl")]
    pub(super) closed_pnl: Value,
    #[serde(default)]
    pub(super) liquidation: Option<RawFillLiquidation>,
    #[serde(default)]
    pub(super) hash: String,
    #[serde(default)]
    pub(super) tid: Option<i64>,
    #[serde(default)]
    pub(super) oid: i64,
    #[serde(default)]
    pub(super) crossed: bool,
    #[serde(default)]
    pub(super) fee: Value,
    #[serde(default, rename = "feeToken")]
    pub(super) fee_token: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RawFillLiquidation {
    #[serde(default)]
    pub(super) liquidated_user: Option<String>,
    #[serde(default)]
    pub(super) mark_px: Value,
    #[serde(default)]
    pub(super) method: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct RawUserFundings {
    #[serde(default)]
    pub(super) fundings: Vec<RawFunding>,
}

#[derive(Debug, Deserialize)]
pub(super) struct RawFunding {
    #[serde(default)]
    pub(super) time: i64,
    #[serde(default)]
    pub(super) coin: String,
    #[serde(default)]
    pub(super) usdc: Value,
    #[serde(default)]
    pub(super) szi: Value,
    #[serde(default, rename = "fundingRate")]
    pub(super) funding_rate: Value,
}

#[derive(Debug, Deserialize)]
pub(super) struct RawLiquidation {
    #[serde(default)]
    pub(super) lid: i64,
    #[serde(default)]
    pub(super) liquidator: String,
    #[serde(default, rename = "liquidated_user")]
    pub(super) liquidated_user: String,
    #[serde(default, rename = "liquidated_ntl_pos")]
    pub(super) liquidated_ntl_pos: Value,
    #[serde(default, rename = "liquidated_account_value")]
    pub(super) liquidated_account_value: Value,
}

#[derive(Debug, Deserialize)]
pub(super) struct RawNonUserCancel {
    #[serde(default)]
    pub(super) coin: String,
    #[serde(default)]
    pub(super) oid: i64,
}

#[derive(Debug, Deserialize)]
pub(super) struct RawClearinghouseEnvelope {
    #[serde(default)]
    pub(super) dex: Option<String>,
    pub(super) user: String,
    #[serde(rename = "clearinghouseState")]
    pub(super) clearinghouse_state: RawInnerClearinghouseState,
}

#[derive(Debug, Deserialize)]
pub(super) struct RawAllDexsClearinghouse {
    pub(super) user: String,
    #[serde(rename = "clearinghouseStates")]
    pub(super) clearinghouse_states: Vec<(String, RawInnerClearinghouseState)>,
}

#[derive(Debug, Deserialize)]
pub(super) struct RawOpenOrdersEnvelope {
    #[serde(default)]
    pub(super) dex: String,
    #[serde(default)]
    pub(super) orders: Vec<RawBasicOrder>,
}

#[derive(Debug, Deserialize)]
pub(super) struct RawInnerClearinghouseState {
    #[serde(rename = "assetPositions")]
    pub(super) asset_positions: Vec<RawAssetPositionEntry>,
    #[serde(rename = "marginSummary")]
    pub(super) margin_summary: RawMarginSummary,
    pub(super) withdrawable: Value,
}

#[derive(Debug, Deserialize)]
pub(super) struct RawMarginSummary {
    #[serde(rename = "accountValue")]
    pub(super) account_value: Value,
    #[serde(rename = "totalMarginUsed")]
    pub(super) total_margin_used: Value,
}

#[derive(Debug, Deserialize)]
pub(super) struct RawAssetPositionEntry {
    pub(super) position: RawPosition,
}

#[derive(Debug, Deserialize)]
pub(super) struct RawPosition {
    pub(super) coin: String,
    pub(super) szi: Value,
    #[serde(default, rename = "entryPx")]
    pub(super) entry_px: Value,
    #[serde(default, rename = "liquidationPx")]
    pub(super) liquidation_px: Value,
    #[serde(rename = "marginUsed")]
    pub(super) margin_used: Value,
    #[serde(rename = "unrealizedPnl")]
    pub(super) unrealized_pnl: Value,
    #[serde(default)]
    pub(super) leverage: Option<RawLeverage>,
}

#[derive(Debug, Deserialize)]
pub(super) struct RawLeverage {
    #[serde(default)]
    pub(super) value: Value,
}

#[derive(Debug, Deserialize)]
pub(super) struct RawSpotStateEnvelope {
    #[serde(rename = "spotState")]
    pub(super) spot_state: RawSpotState,
}

#[derive(Debug, Deserialize)]
pub(super) struct RawSpotState {
    pub(super) balances: Vec<RawSpotBalance>,
}

#[derive(Debug, Deserialize)]
pub(super) struct RawSpotBalance {
    pub(super) coin: String,
    pub(super) token: u32,
    pub(super) hold: Value,
    pub(super) total: Value,
    #[serde(rename = "entryNtl")]
    pub(super) entry_ntl: Value,
}
