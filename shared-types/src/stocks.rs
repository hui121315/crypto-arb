use serde::{Deserialize, Serialize};

pub mod comparison;
pub mod identity;
pub mod peers;
pub use peers::*;
pub mod preflight;
pub mod peer_preflight;
pub use peer_preflight::*;
pub mod peer_funding;
pub use peer_funding::*;
pub mod peer_order;
pub use peer_order::*;
pub mod peer_receipt;
pub use peer_receipt::*;
pub mod peer_plan;
pub use peer_plan::*;
pub mod peer_accounting;
pub use peer_accounting::*;
pub mod peer_recovery;
pub use peer_recovery::*;
pub mod peer_conversion;
pub mod peer_inventory;
pub use peer_inventory::*;
pub mod peer_native_topup;
pub use peer_native_topup::*;
pub use peer_conversion::*;
pub mod chain_cost;
pub mod rfq;
pub mod alerts;
pub mod plan;
mod plan_check;
pub mod funding;
mod restock;
mod batch;
pub use batch::*;
pub use restock::*;
pub use funding::*;
mod funding_plan;
pub use funding_plan::*;
mod funding_transfer;
pub use funding_transfer::*;
pub mod order;
pub mod chain_execution;
pub mod fees;
pub use fees::*;
pub mod stablecoin;
pub use stablecoin::*;
pub mod exchange_conversion;
pub use exchange_conversion::*;
mod conversion_costs;
pub use conversion_costs::STOCK_CONVERSION_COST_LIMIT;
pub mod accounting;
pub use accounting::*;
pub mod recovery;
pub use recovery::*;
pub use chain_execution::*;
pub use order::*;
pub use plan::*;
pub use alerts::*;
pub use preflight::*;
pub use chain_cost::*;
pub use rfq::*;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockSession {
    pub name: String,
    pub min_quantity: String,
    pub max_quantity: Option<String>,
    pub step_size: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockOrderBookMarket {
    pub symbol: String,
    pub quote: String,
    pub state: String,
    pub tick_size: String,
    pub min_quantity: String,
    pub step_size: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockSecurity {
    pub asset: String,
    pub ticker: String,
    pub name: String,
    pub cusip: Option<String>,
    pub sessions: Vec<StockSession>,
    pub order_books: Vec<StockOrderBookMarket>,
    pub rfq_symbol: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockCatalog {
    pub rows: Vec<StockSecurity>,
    pub observed_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockChainToken {
    pub blockchain: String,
    pub contract_address: Option<String>,
    pub native_decimals: Option<u8>,
    pub deposit_enabled: Option<bool>,
    pub withdraw_enabled: Option<bool>,
    pub minimum_deposit: Option<String>,
    pub minimum_withdrawal: Option<String>,
    pub maximum_withdrawal: Option<String>,
    pub withdrawal_fee: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockBookQuote {
    pub symbol: String,
    pub bid: Option<String>,
    pub bid_quantity: Option<String>,
    pub ask: Option<String>,
    pub ask_quantity: Option<String>,
    pub update_id: u64,
    pub source_at_ms: i64,
    pub received_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockReferenceQuote {
    pub ticker: String,
    pub bid: Option<String>,
    pub ask: Option<String>,
    pub mid: String,
    pub session: Option<String>,
    pub source_at_ms: i64,
    pub received_at_ms: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockMarketSnapshot {
    #[serde(default)]
    pub batch: StockBatchStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer: Option<StockPeerComparison>,
    pub security: Option<StockSecurity>,
    pub tokens: Vec<StockChainToken>,
    pub token_metadata_at_ms: Option<i64>,
    pub token_metadata_problem: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub funding_assets: Vec<StockFundingAsset>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deposit_address: Option<StockDepositAddress>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub funding_plans: Vec<StockFundingPlan>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub funding_problem: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stablecoin_plans: Vec<StockStablecoinPlan>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stablecoin_problem: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exchange_conversions: Vec<StockExchangeConversionPlan>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exchange_conversion_problem: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub claimed_conversion_cost_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversion_book: Option<StockBookQuote>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversion_book_problem: Option<String>,
    pub books: Vec<StockBookQuote>,
    pub reference: Option<StockReferenceQuote>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference_problem: Option<String>,
    pub connected: bool,
    pub problem: Option<String>,
    #[serde(default)]
    pub comparison: Option<StockComparison>,
    #[serde(default)]
    pub trading_route: Option<StockTradingRoute>,
    #[serde(default)]
    pub monitor: StockMonitorStatus,
    #[serde(default)]
    pub alerts: StockAlertRuntime,
    #[serde(default)]
    pub rfqs: Vec<StockRfq>,
    #[serde(default)]
    pub rfq_connected: bool,
    #[serde(default)]
    pub rfq_problem: Option<String>,
    #[serde(default)]
    pub preflight: Option<StockPreflight>,
    #[serde(default)]
    pub peer_preflight: Option<StockPeerPreflight>,
    #[serde(default)]
    pub peer_funding: Option<StockPeerFunding>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub peer_order_checks: Vec<StockPeerOrderCheck>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub peer_plans: Vec<StockPeerPlan>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub peer_accounting: Vec<StockPeerAccounting>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer_plan_problem: Option<String>,
    #[serde(default)]
    pub chain_costs: Vec<StockChainCost>,
    #[serde(default)]
    pub plans: Vec<StockExecutionPlan>,
    #[serde(default)]
    pub plan_problem: Option<String>,
    pub observed_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockWatchRequest {
    pub asset: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockQuoteRequest {
    pub asset: String,
    pub budget_usdc: String,
    #[serde(default)]
    pub keyed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StockRouteKind {
    OrderBook,
    Rfq,
    Closed,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockTradingRoute {
    pub kind: StockRouteKind,
    pub session: Option<StockSession>,
    pub symbol: Option<String>,
    pub reason: String,
    pub timezone: Option<String>,
    pub calendar_at_ms: Option<i64>,
    pub valid_until_ms: i64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StockMonitorPhase {
    #[default]
    Disabled,
    WaitingForViewers,
    Refreshing,
    Watching,
    QuantityLimited,
    Backoff,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockMonitorStatus {
    #[serde(default)]
    pub revision: String,
    pub enabled: bool,
    #[serde(default)]
    pub alerts: StockAlertConfig,
    pub request: Option<StockQuoteRequest>,
    pub phase: StockMonitorPhase,
    pub completed_quotes: u64,
    pub consecutive_failures: u32,
    pub last_success_at_ms: Option<i64>,
    pub next_attempt_at_ms: Option<i64>,
    pub problem: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockMonitorRequest {
    pub enabled: bool,
    pub quote: StockQuoteRequest,
    #[serde(default)]
    pub alerts: StockAlertConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockMonitorUpdateRequest {
    pub expected_revision: String,
    pub request: StockMonitorRequest,
}

/// Saved configuration only, never a live quote or account snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockMonitorReceipt {
    pub asset: String,
    pub revision: String,
    pub enabled: bool,
    pub request: Option<StockQuoteRequest>,
    pub alerts: StockAlertConfig,
    pub observed_at_ms: i64,
}

impl StockMonitorReceipt {
    pub fn valid_for(&self, asset: &str) -> bool {
        self.asset == asset && !asset.is_empty() && !self.revision.is_empty()
            && self.observed_at_ms > 0
            && self.request.as_ref().is_none_or(|r| r.asset == asset)
            && (!self.enabled || self.request.is_some())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockMintEvidence {
    pub address: String,
    pub decimals: u8,
    pub ui_multiplier: String,
    pub slot: u64,
    pub chain_time_ms: i64,
    pub checked_at_ms: i64,
    pub next_change_at_ms: Option<i64>,
    pub extensions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockDexQuote {
    pub input_mint: String,
    pub output_mint: String,
    pub input_raw: String,
    pub output_raw: String,
    pub minimum_output_raw: String,
    pub router: String,
    pub fee_bps: Option<u16>,
    pub fee_mint: Option<String>,
    pub requested_at_ms: i64,
    pub received_at_ms: i64,
    pub expires_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockComparison {
    pub asset: String,
    pub issuer_docs: String,
    pub budget_usdc: String,
    pub keyed: bool,
    pub mint: StockMintEvidence,
    pub buy: StockDexQuote,
    pub sell: Option<StockDexQuote>,
    pub sell_problem: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quantity_limit: Option<StockQuoteQuantityLimit>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockQuoteQuantityLimit {
    pub quoted_shares: String,
    pub min_quantity: String,
    pub max_quantity: Option<String>,
    pub step_size: String,
}
