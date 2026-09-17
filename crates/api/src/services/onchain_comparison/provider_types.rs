use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ProviderRuntime {
    pub(super) configured: bool,
    pub(super) problem: Option<String>,
    pub(super) quote_interval_ms: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct JupiterOrderQuote {
    pub(super) input_mint: String,
    pub(super) output_mint: String,
    pub(super) in_amount: String,
    pub(super) out_amount: String,
    #[serde(default)]
    pub(super) router: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct JupiterOrderBuild {
    pub(super) input_mint: String,
    pub(super) output_mint: String,
    pub(super) in_amount: String,
    pub(super) out_amount: String,
    pub(super) transaction: Option<String>,
    pub(super) request_id: String,
    pub(super) router: String,
    pub(super) mode: String,
    #[serde(default)]
    pub(super) last_valid_block_height: Option<u64>,
    #[serde(default)]
    pub(super) expire_at: Option<serde_json::Value>,
    #[serde(default)]
    pub(super) error_code: Option<i64>,
    #[serde(default)]
    pub(super) error_message: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ZeroExPrice {
    pub(super) buy_amount: Option<String>,
    pub(super) buy_token: String,
    pub(super) sell_amount: Option<String>,
    pub(super) sell_token: String,
    #[serde(default)]
    pub(super) liquidity_available: Option<bool>,
    #[serde(default)]
    pub(super) route: Option<ZeroExRoute>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ZeroExFirmQuote {
    pub(super) buy_amount: Option<String>,
    pub(super) min_buy_amount: Option<String>,
    pub(super) buy_token: String,
    pub(super) sell_amount: Option<String>,
    pub(super) sell_token: String,
    #[serde(default)]
    pub(super) liquidity_available: Option<bool>,
    #[serde(default)]
    pub(super) issues: Option<ZeroExIssues>,
    #[serde(default)]
    pub(super) allowance_target: Option<String>,
    #[serde(default)]
    pub(super) transaction: Option<ZeroExTransaction>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ZeroExIssues {
    #[serde(default)]
    pub(super) allowance: Option<ZeroExAllowanceIssue>,
    #[serde(default)]
    pub(super) balance: Option<serde_json::Value>,
    #[serde(default)]
    pub(super) simulation_incomplete: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ZeroExAllowanceIssue {
    #[serde(default)]
    pub(super) actual: Option<String>,
    pub(super) spender: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ZeroExTransaction {
    pub(super) to: String,
    pub(super) data: String,
    pub(super) value: String,
    pub(super) gas: String,
    #[serde(default)]
    pub(super) gas_price: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ZeroExRoute {
    #[serde(default)]
    pub(super) fills: Vec<ZeroExFill>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ZeroExFill {
    pub(super) source: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct OkxQuoteEnvelope {
    pub(super) code: String,
    #[serde(default)]
    pub(super) msg: String,
    #[serde(default)]
    pub(super) data: Vec<OkxDexQuote>,
}

#[derive(Debug, Deserialize)]
pub(super) struct OkxSwapEnvelope {
    pub(super) code: String,
    #[serde(default)]
    pub(super) msg: String,
    #[serde(default)]
    pub(super) data: Vec<OkxDexSwap>,
}

#[derive(Debug, Deserialize)]
pub(super) struct OkxApprovalEnvelope {
    pub(super) code: String,
    #[serde(default)]
    pub(super) msg: String,
    #[serde(default)]
    pub(super) data: Vec<OkxApprovalTransaction>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct OkxApprovalTransaction {
    pub(super) data: String,
    pub(super) dex_contract_address: String,
    pub(super) gas_limit: String,
    pub(super) gas_price: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct OkxDexSwap {
    pub(super) router_result: OkxSwapRouterResult,
    pub(super) tx: OkxEvmTransaction,
    #[serde(default)]
    pub(super) signature_data: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct OkxSwapRouterResult {
    pub(super) chain_index: String,
    pub(super) from_token_amount: String,
    pub(super) to_token_amount: String,
    pub(super) from_token: OkxTokenIdentity,
    pub(super) to_token: OkxTokenIdentity,
    #[serde(default)]
    pub(super) router: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct OkxEvmTransaction {
    pub(super) from: String,
    pub(super) to: String,
    pub(super) data: String,
    pub(super) value: String,
    pub(super) gas: String,
    #[serde(default)]
    pub(super) gas_price: Option<String>,
    #[serde(default)]
    pub(super) max_priority_fee_per_gas: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct OkxDexQuote {
    pub(super) chain_index: String,
    pub(super) from_token_amount: String,
    pub(super) to_token_amount: String,
    pub(super) from_token: OkxTokenIdentity,
    pub(super) to_token: OkxTokenIdentity,
    #[serde(default)]
    pub(super) router: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct OkxTokenIdentity {
    pub(super) token_contract_address: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct OkxCredentials {
    pub(super) api_key: String,
    pub(super) secret_key: String,
    pub(super) passphrase: String,
}
