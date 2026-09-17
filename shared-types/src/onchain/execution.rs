use serde::{Deserialize, Serialize};

use super::{OnchainComparisonDirection, OnchainPathReadiness};
use crate::{InstrumentSpec, LiveOrderState, OrderSide, OrderSizingPlan};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnchainInventoryLocation {
    Onchain,
    Cex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnchainInventoryStatus {
    Ready,
    Insufficient,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainInventoryEvidence {
    pub location: OnchainInventoryLocation,
    pub scope: String,
    pub asset: String,
    pub required: f64,
    pub available: Option<f64>,
    pub status: OnchainInventoryStatus,
    pub source: String,
    pub observed_at_ms: Option<i64>,
    pub problem: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainCexInstrumentEvidence {
    pub venue: String,
    pub requested_symbol: String,
    pub native_symbol: Option<String>,
    #[serde(default)]
    pub status: OnchainCexInstrumentStatus,
    pub ready: bool,
    pub source: String,
    pub observed_at_ms: Option<i64>,
    pub problem: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnchainCexInstrumentStatus {
    Ready,
    #[default]
    Syncing,
    Unlisted,
    Incomplete,
    Stale,
    Unavailable,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainDirectionReadiness {
    pub direction: OnchainComparisonDirection,
    #[serde(default)]
    pub path: OnchainPathReadiness,
    pub inventory: Vec<OnchainInventoryEvidence>,
    #[serde(default)]
    pub cex_instrument: OnchainCexInstrumentEvidence,
    pub build_ready: bool,
    pub submit_ready: bool,
    pub blockers: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainExecutionReadiness {
    pub wallet_address_configured: bool,
    #[serde(default)]
    pub chain_submission_ready: bool,
    #[serde(default)]
    pub cex_live_mode_ready: bool,
    #[serde(default)]
    pub global_blockers: Vec<String>,
    pub directions: Vec<OnchainDirectionReadiness>,
    pub observed_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainExecutionBuildRequest {
    pub direction: OnchainComparisonDirection,
    pub expected_quote_observed_at_ms: i64,
    pub expected_cex_observed_at_ms: i64,
    /// Explicit whole-run cost allocation; amounts are resolved by the server.
    #[serde(default)]
    pub replenishment_run_ids: Vec<String>,
    #[serde(default)]
    pub approval_run_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OnchainUnsignedTransaction {
    SolanaVersioned {
        transaction_base64: String,
        request_id: String,
        router: String,
        mode: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        last_valid_block_height: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expire_at_ms: Option<i64>,
    },
    EvmCall {
        chain_id: u64,
        from: String,
        to: String,
        data: String,
        value: String,
        gas: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        gas_price: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_priority_fee_per_gas: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        allowance_spender: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainCexOrderPlan {
    pub venue: String,
    pub native_symbol: String,
    pub client_order_id: String,
    pub side: OrderSide,
    pub base_quantity: f64,
    pub reference_price: f64,
    pub estimated_quote_amount: f64,
    pub instrument_spec: InstrumentSpec,
    pub sizing_plan: OrderSizingPlan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnchainQuoteConversionSequence {
    BeforePrimaryCex,
    AfterPrimaryCex,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainQuoteConversionOrderPlan {
    pub sequence: OnchainQuoteConversionSequence,
    pub from_asset: String,
    pub to_asset: String,
    pub planned_from_amount: f64,
    pub planned_to_amount: f64,
    pub order: OnchainCexOrderPlan,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainExecutionBuildResponse {
    pub build_id: String,
    pub direction: OnchainComparisonDirection,
    pub provider: String,
    pub chain: String,
    pub wallet_address: String,
    pub input_token: String,
    pub output_token: String,
    pub input_amount_raw: String,
    pub output_amount_raw: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settlement_assets: Option<OnchainSwapAssets>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum_output_amount_raw: Option<String>,
    pub chain_transaction: OnchainUnsignedTransaction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chain_input_adjustment: Option<OnchainChainInputAdjustment>,
    pub cex_order: OnchainCexOrderPlan,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote_conversion_order: Option<OnchainQuoteConversionOrderPlan>,
    #[serde(default)]
    pub quote_usd_valuation: Option<super::OnchainUsdValuation>,
    #[serde(default)]
    pub replenishment_costs: Vec<OnchainExecutionReplenishmentCost>,
    #[serde(default)]
    pub approval_costs: Vec<OnchainExecutionApprovalCost>,
    pub estimated_net_profit_usd: f64,
    pub estimated_net_spread_bps: f64,
    pub quote_observed_at_ms: i64,
    pub cex_observed_at_ms: i64,
    pub built_at_ms: i64,
    pub valid_until_ms: i64,
    pub official_docs_url: String,
    pub build_ready: bool,
    pub submit_ready: bool,
    pub blockers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainChainInputAdjustment {
    pub original_input_amount_raw: String,
    pub submitted_input_amount_raw: String,
    pub asset: String,
    pub decimals: u8,
    pub cex_order_id: String,
    pub cex_net_received: String,
    pub residual_base_amount: String,
    /// Estimated at this run's paid acquisition cost, not a live market valuation.
    pub residual_cost_estimate_usd: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainExecutionToken {
    pub symbol: String,
    pub address: String,
    pub decimals: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainSwapAssets {
    pub input: OnchainExecutionToken,
    pub output: OnchainExecutionToken,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnchainChainSettlementStatus {
    Pending,
    Complete,
    ReviewRequired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainChainSettlementBasis {
    pub chain: String,
    pub wallet: String,
    pub transaction_id: String,
    pub assets: OnchainSwapAssets,
    pub maximum_input_raw: String,
    pub minimum_output_raw: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainChainSettlement {
    pub basis: OnchainChainSettlementBasis,
    pub status: OnchainChainSettlementStatus,
    pub input_amount_raw: Option<String>,
    pub output_amount_raw: Option<String>,
    /// Native asset movement excluding the network fee. Only populated when
    /// neither swap asset is native; includes tips without counting gas twice.
    pub additional_native_change_raw: Option<String>,
    pub network_cost: Option<super::OnchainReplenishmentNetworkCost>,
    pub block_ref: Option<String>,
    pub observed_at_ms: Option<i64>,
    pub problem: Option<String>,
    #[serde(default)]
    pub attempts: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainWalletReceiptBasis {
    pub chain: String,
    pub wallet: String,
    pub transaction_id: String,
    pub assets: Vec<OnchainExecutionToken>,
    pub require_sender: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainWalletReceipt {
    pub basis: OnchainWalletReceiptBasis,
    pub status: OnchainChainSettlementStatus,
    /// Signed wallet changes, in the same order as basis.assets; gas is separate.
    pub asset_changes_raw: Vec<Option<String>>,
    pub additional_native_change_raw: Option<String>,
    pub network_cost: Option<super::OnchainReplenishmentNetworkCost>,
    pub block_ref: Option<String>,
    pub observed_at_ms: Option<i64>,
    pub problem: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainTokenApprovalBuildRequest {
    pub direction: OnchainComparisonDirection,
    pub expected_quote_observed_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainTokenApprovalBuildResponse {
    pub approval_id: String,
    pub direction: OnchainComparisonDirection,
    pub provider: String,
    pub chain: String,
    pub wallet_address: String,
    pub token_address: String,
    pub token_symbol: String,
    pub token_decimals: u8,
    pub spender: String,
    pub required_amount_raw: String,
    pub current_allowance_raw: String,
    pub transactions: Vec<OnchainUnsignedTransaction>,
    pub built_at_ms: i64,
    pub valid_until_ms: i64,
    pub official_docs_url: String,
    pub approval_required: bool,
    pub submit_ready: bool,
    pub blockers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainTokenApprovalSubmitRequest {
    pub approval_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnchainTokenApprovalRunStatus {
    Completed,
    AwaitingFinality,
    FinalityUnresolved,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainTokenApprovalSubmitResponse {
    pub run_id: String,
    pub approval_id: String,
    pub status: OnchainTokenApprovalRunStatus,
    pub transaction_ids: Vec<String>,
    pub message: String,
    pub problem: Option<String>,
    pub started_at_ms: i64,
    pub updated_at_ms: i64,
    #[serde(default)]
    pub fee_receipts: Vec<OnchainWalletReceipt>,
    #[serde(default)]
    pub fee_checks_exhausted: bool,
}

impl OnchainTokenApprovalSubmitResponse {
    pub fn receipt_check_pending(&self) -> bool {
        !self.fee_checks_exhausted
            && self.transaction_ids.iter().any(|hash| {
                !self.fee_receipts.iter().any(|r| {
                    r.basis.transaction_id == *hash
                        && r.status != OnchainChainSettlementStatus::Pending
                        && r.additional_native_change_raw.is_some()
                        && r.asset_changes_raw.iter().all(Option::is_some)
                        && r.network_cost
                            .as_ref()
                            .is_some_and(|c| c.total_fee_exact.is_some() && c.problem.is_none())
                })
            })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainTokenApprovalRunsResponse {
    pub rows: Vec<OnchainTokenApprovalSubmitResponse>,
    #[serde(default)]
    pub cost_owners: std::collections::BTreeMap<String, String>,
    pub observed_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainExecutionSubmitRequest {
    pub build_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnchainExecutionRunStatus {
    Executing,
    Completed,
    AwaitingChainFinality,
    FinalityUnresolved,
    Compensated,
    Failed,
    Exposed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnchainExecutionLegKind {
    QuoteConversion,
    PrimaryCex,
    Chain,
    Compensation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnchainExecutionLegStatus {
    Submitted,
    PartiallyFilled,
    Filled,
    Confirmed,
    Pending,
    Cancelled,
    Rejected,
    Failed,
    Compensated,
    Exposed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainExecutionLegResult {
    pub position: u8,
    pub kind: OnchainExecutionLegKind,
    pub status: OnchainExecutionLegStatus,
    pub venue: String,
    pub symbol: Option<String>,
    pub order_id: Option<String>,
    pub transaction_id: Option<String>,
    pub filled_quantity: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settlement: Option<OnchainCexSettlement>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery_residual: Option<OnchainCexRecoveryResidual>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chain_input_adjustment: Option<OnchainChainInputAdjustment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chain_settlement: Option<OnchainChainSettlement>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainCexRecoveryResidual {
    pub original_order_id: String,
    pub asset: String,
    /// Signed inventory change remaining after the original and reverse trades.
    pub amount: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainCexSettlementBasis {
    pub order_id: String,
    pub venue: String,
    pub symbol: String,
    pub side: OrderSide,
    pub base_asset: String,
    pub quote_asset: String,
    pub confirmed_quantity: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnchainCexSettlementStatus {
    PendingFills,
    PendingFees,
    Complete,
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainCexSettlementFee {
    pub asset: String,
    pub amount: String,
}

/// Native-asset settlement from incremental fills, not a USD profit estimate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainCexSettlement {
    pub basis: OnchainCexSettlementBasis,
    pub status: OnchainCexSettlementStatus,
    pub gross_base_amount: Option<String>,
    pub gross_quote_amount: Option<String>,
    pub debit_amount: Option<String>,
    pub credit_amount: Option<String>,
    pub fees: Vec<OnchainCexSettlementFee>,
    pub fill_event_ids: Vec<String>,
    pub observed_at_ms: Option<i64>,
    pub problem: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnchainExecutionRecoveryKind {
    WaitForFinality,
    DoNotResubmit,
    VerifyExposure,
    RebuildPlan,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainExecutionRecoveryAction {
    pub kind: OnchainExecutionRecoveryKind,
    pub automated: bool,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnchainExecutionAccountingStatus {
    PendingReceipts,
    PendingValuation,
    Valued,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainExecutionCashFlow {
    pub location: String,
    pub source_id: String,
    pub asset: String,
    pub amount_exact: String,
    pub kind: OnchainExecutionCashFlowKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnchainExecutionCashFlowKind {
    Trade,
    Fee,
    ReplenishmentFee,
    ApprovalFee,
    OtherNativeChange,
}

/// Server-resolved native costs, frozen at build and claimed durably before any order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainExecutionReplenishmentCost {
    pub run_id: String,
    pub plan_id: String,
    pub completed_at_ms: i64,
    pub transfer_ids: Vec<String>,
    pub fees: Vec<OnchainExecutionCashFlow>,
    pub evidence_sources: Vec<String>,
    pub build_valuation: OnchainExecutionUsdValue,
}

/// Frozen approval receipt evidence; the execution journal claims the whole run once.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainExecutionApprovalCost {
    pub plan: OnchainTokenApprovalBuildResponse,
    pub run: OnchainTokenApprovalSubmitResponse,
    pub build_valuation: OnchainExecutionUsdValue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainExecutionAssetChange {
    pub asset: String,
    pub amount_exact: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainExecutionUsdValue {
    pub net_usd_exact: String,
    pub valued_at_ms: i64,
    pub rates: Vec<super::OnchainUsdValuation>,
}

/// Actual execution cash flows valued at the recorded quotes, not realized USD cash.
/// Separate replenishment and approval runs need an explicit allocation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainExecutionAccounting {
    pub status: OnchainExecutionAccountingStatus,
    pub flows: Vec<OnchainExecutionCashFlow>,
    pub net_assets: Vec<OnchainExecutionAssetChange>,
    pub usd_value: Option<OnchainExecutionUsdValue>,
    pub problems: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainExecutionSubmitResponse {
    pub run_id: String,
    pub build_id: String,
    pub status: OnchainExecutionRunStatus,
    pub cex_order_id: Option<String>,
    pub cex_order_state: Option<LiveOrderState>,
    pub cex_filled_quantity: Option<f64>,
    pub chain_transaction_id: Option<String>,
    pub compensation_order_id: Option<String>,
    #[serde(default)]
    pub legs: Vec<OnchainExecutionLegResult>,
    #[serde(default)]
    pub recovery_actions: Vec<OnchainExecutionRecoveryAction>,
    #[serde(default)]
    pub replenishment_costs: Vec<OnchainExecutionReplenishmentCost>,
    #[serde(default)]
    pub approval_costs: Vec<OnchainExecutionApprovalCost>,
    pub estimated_net_profit_usd: f64,
    pub remaining_exposure_usd: f64,
    /// Confirmed hedge quantity, with separately reported network fees excluded.
    #[serde(default)]
    pub quantity_reconciled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accounting: Option<OnchainExecutionAccounting>,
    pub message: String,
    pub problem: Option<String>,
    pub started_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainExecutionRunsResponse {
    pub rows: Vec<OnchainExecutionSubmitResponse>,
    pub observed_at_ms: i64,
    #[serde(default)]
    pub recovery_problem: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_instrument_evidence_defaults_to_syncing_without_claiming_readiness() {
        let evidence: OnchainCexInstrumentEvidence = serde_json::from_value(serde_json::json!({
            "venue": "binance",
            "requestedSymbol": "SOL/USDC",
            "nativeSymbol": null,
            "ready": false,
            "source": "instrument_registry",
            "observedAtMs": null,
            "problem": "first probe pending"
        }))
        .unwrap_or_default();

        assert_eq!(evidence.status, OnchainCexInstrumentStatus::Syncing);
        assert!(!evidence.ready);
    }
}
