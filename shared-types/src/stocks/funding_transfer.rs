use super::*;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockFundingTransferPreparation {
    pub transaction_base64: String,
    pub blockhash: String,
    pub last_valid_block_height: u64,
    pub source_token_account: Option<String>,
    pub destination_token_account: Option<String>,
    pub token_program: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_creation: Option<StockFundingAccountCreation>,
    pub network_fee_lamports: u64,
    pub retained_sol_lamports: u64,
    pub slot: u64,
    pub prepared_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockFundingAccountCreation {
    pub account_size: u32,
    pub rent_budget_lamports: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockFundingTransferReceipt {
    pub transaction_hash: String,
    pub succeeded: bool,
    pub within_plan: bool,
    pub source_debit_raw: u64,
    pub destination_credit_raw: u64,
    pub wallet_debit_lamports: u64,
    pub network_fee_lamports: u64,
    #[serde(default)]
    pub account_creation_lamports: u64,
    pub slot: u64,
    pub block_time_ms: i64,
    pub checked_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockFundingDepositRecord {
    pub id: i32,
    pub source: String,
    pub status: String,
    pub symbol: String,
    pub quantity: String,
    pub created_at: String,
    pub transaction_hash: String,
    pub to_address: Option<String>,
    pub from_address: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockFundingTransfer {
    pub preparation: StockFundingTransferPreparation,
    pub submitted_at_ms: Option<i64>,
    pub transaction_hash: Option<String>,
    pub acknowledged: bool,
    pub query_count: u32,
    pub last_query_at_ms: Option<i64>,
    pub receipt: Option<StockFundingTransferReceipt>,
    pub deposit: Option<StockFundingDepositRecord>,
    pub problem: Option<String>,
}
