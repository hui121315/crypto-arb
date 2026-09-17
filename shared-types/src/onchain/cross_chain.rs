use serde::{Deserialize, Serialize};

use super::execution::OnchainUnsignedTransaction;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainCrossChainConfig {
    pub enabled: bool,
    pub peer_item_id: String,
    pub provider: String,
    #[serde(default = "default_stablecoin_risk_bps")]
    pub stablecoin_risk_bps: u16,
}

impl Default for OnchainCrossChainConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            peer_item_id: String::new(),
            provider: "lifi".to_owned(),
            stablecoin_risk_bps: default_stablecoin_risk_bps(),
        }
    }
}

const fn default_stablecoin_risk_bps() -> u16 {
    50
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainCrossChainConfigPatch {
    pub enabled: Option<bool>,
    pub peer_item_id: Option<String>,
    pub provider: Option<String>,
    pub stablecoin_risk_bps: Option<u16>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnchainCrossChainQuality {
    #[default]
    Disabled,
    Pending,
    Fresh,
    NoNetProfit,
    Stale,
    PeerMissing,
    EvidencePending,
    UpstreamUnavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnchainCrossChainLegKind {
    SourceSwap,
    OutboundBridge,
    TargetSwap,
    ReturnBridge,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainCrossChainLeg {
    pub position: u8,
    pub kind: OnchainCrossChainLegKind,
    pub provider: String,
    pub from_chain: String,
    pub to_chain: String,
    pub from_asset: String,
    pub to_asset: String,
    pub from_token: String,
    pub to_token: String,
    pub input_amount_raw: String,
    pub expected_output_amount_raw: String,
    pub minimum_output_amount_raw: Option<String>,
    pub input_decimals: u8,
    pub output_decimals: u8,
    pub fee_usd: Option<f64>,
    pub gas_usd: Option<f64>,
    pub estimated_duration_seconds: Option<u64>,
    pub route_id: Option<String>,
    #[serde(default)]
    pub route_tools: Vec<String>,
    pub official_docs_url: String,
    pub observed_at_ms: i64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnchainCrossChainInventoryStatus {
    Ready,
    Insufficient,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainCrossChainInventoryRequirement {
    pub chain: String,
    pub wallet_address: String,
    pub asset: String,
    pub token_address: String,
    pub required_amount_raw: Option<String>,
    pub available_amount_raw: Option<String>,
    pub status: OnchainCrossChainInventoryStatus,
    pub problem: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainCrossChainSnapshot {
    pub provider: String,
    pub peer_item_id: String,
    pub peer_chain: Option<String>,
    pub quality: OnchainCrossChainQuality,
    pub legs: Vec<OnchainCrossChainLeg>,
    pub initial_quote_amount_raw: Option<String>,
    pub final_quote_amount_raw: Option<String>,
    pub gross_return_bps: Option<f64>,
    pub execution_buffer_bps: Option<f64>,
    #[serde(default)]
    pub stablecoin_risk_bps: u16,
    pub bridge_fee_usd: Option<f64>,
    pub gas_usd: Option<f64>,
    #[serde(default)]
    pub quote_usd_valuation: Option<super::OnchainUsdValuation>,
    pub total_cost_bps: Option<f64>,
    pub net_return_bps: Option<f64>,
    pub estimated_duration_seconds: Option<u64>,
    #[serde(default)]
    pub inventory: Vec<OnchainCrossChainInventoryRequirement>,
    pub atomic: bool,
    pub preview_ready: bool,
    pub submit_ready: bool,
    pub problem: Option<String>,
    pub quote_observed_at_ms: Option<i64>,
    pub quote_latency_ms: Option<i64>,
    pub observed_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainCrossChainBuildRequest {
    pub expected_quote_observed_at_ms: i64,
    #[serde(default)]
    pub approval_run_ids: Vec<String>,
    #[serde(default)]
    pub replenishment_run_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainCrossChainBridgeExecution {
    pub position: u8,
    pub kind: OnchainCrossChainLegKind,
    pub provider: String,
    pub route_id: String,
    pub transaction_id: String,
    pub tool: String,
    pub from_chain_id: u64,
    pub to_chain_id: u64,
    pub from_address: String,
    pub to_address: String,
    pub from_token: String,
    pub to_token: String,
    pub from_amount_raw: String,
    pub to_amount_min_raw: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_address: Option<String>,
    pub transaction: OnchainUnsignedTransaction,
    pub quote_observed_at_ms: i64,
    pub valid_until_ms: i64,
    pub rebuild_after_position: u8,
    pub official_docs_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainCrossChainSwapExecution {
    pub execution_id: String,
    pub position: u8,
    pub kind: OnchainCrossChainLegKind,
    pub provider: String,
    pub chain: String,
    pub wallet_address: String,
    pub input_token: String,
    pub output_token: String,
    pub input_amount_raw: String,
    pub quoted_output_amount_raw: String,
    pub minimum_output_amount_raw: String,
    pub transaction: OnchainUnsignedTransaction,
    pub quote_observed_at_ms: i64,
    pub valid_until_ms: i64,
    pub rebuild_after_position: u8,
    pub official_docs_url: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainCrossChainBuildResponse {
    pub build_id: String,
    #[serde(default)]
    pub approval_costs: Vec<super::OnchainExecutionApprovalCost>,
    #[serde(default)]
    pub replenishment_costs: Vec<super::OnchainExecutionReplenishmentCost>,
    pub provider: String,
    pub source_chain: String,
    pub peer_chain: String,
    pub legs: Vec<OnchainCrossChainLeg>,
    #[serde(default)]
    pub bridge_executions: Vec<OnchainCrossChainBridgeExecution>,
    #[serde(default)]
    pub swap_executions: Vec<OnchainCrossChainSwapExecution>,
    #[serde(default)]
    pub inventory: Vec<OnchainCrossChainInventoryRequirement>,
    pub initial_quote_amount_raw: String,
    pub final_quote_amount_raw: String,
    pub gross_return_bps: Option<f64>,
    #[serde(default)]
    pub stablecoin_risk_bps: u16,
    pub bridge_fee_usd: Option<f64>,
    pub gas_usd: Option<f64>,
    #[serde(default)]
    pub quote_usd_valuation: Option<super::OnchainUsdValuation>,
    pub total_cost_bps: Option<f64>,
    pub net_return_bps: Option<f64>,
    pub estimated_duration_seconds: Option<u64>,
    pub quote_observed_at_ms: i64,
    pub quote_latency_ms: Option<i64>,
    pub built_at_ms: i64,
    pub valid_until_ms: i64,
    pub atomic: bool,
    pub monitor_only: bool,
    pub preview_ready: bool,
    pub submit_ready: bool,
    #[serde(default)]
    pub blockers: Vec<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

pub const ONCHAIN_CROSS_CHAIN_AUTHORIZATION_PHRASE: &str = "AUTHORIZE LIVE CROSS CHAIN";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainCrossChainAuthorizeRequest {
    pub build_id: String,
    pub idempotency_key: String,
    pub confirmation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainCrossChainSubmitRequest {
    pub run_id: String,
    pub expected_position: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainCrossChainRecheckRequest {
    pub run_id: String,
    pub expected_position: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnchainCrossChainRunStatus {
    AuthorizedAwaitingSubmit,
    AuthorizationExpired,
    Running,
    AwaitingSourceFinality,
    AwaitingDestinationEvidence,
    Paused,
    Compensating,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnchainCrossChainLegRunStatus {
    RequoteRequired,
    SubmissionClaimed,
    Submitted,
    SourceConfirmed,
    Completed,
    Paused,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainCrossChainLegProgress {
    pub position: u8,
    pub kind: OnchainCrossChainLegKind,
    pub client_action_id: String,
    pub status: OnchainCrossChainLegRunStatus,
    pub attempts: u32,
    pub planned_input_amount_raw: String,
    pub minimum_output_amount_raw: Option<String>,
    #[serde(default)]
    pub submitted_input_amount_raw: Option<String>,
    pub actual_input_amount_raw: Option<String>,
    pub actual_output_amount_raw: Option<String>,
    #[serde(default)]
    pub source_receipt: Option<super::OnchainWalletReceipt>,
    #[serde(default)]
    pub destination_receipt: Option<super::OnchainWalletReceipt>,
    #[serde(default)]
    pub receipt_checks: u8,
    #[serde(default)]
    pub recovery_started_at_ms: Option<i64>,
    #[serde(default)]
    pub recovery_checks: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bridge_recovery: Option<OnchainCrossChainRecovery>,
    #[serde(default)]
    pub bridge_reported_output_amount_raw: Option<String>,
    pub provider_transaction_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub swap_execution: Option<OnchainCrossChainSwapExecution>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bridge_execution: Option<OnchainCrossChainBridgeExecution>,
    pub source_transaction_id: Option<String>,
    pub source_submitted_at_ms: Option<i64>,
    pub destination_transaction_id: Option<String>,
    pub quote_observed_at_ms: Option<i64>,
    pub quote_valid_until_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub projected_final_quote_amount_raw: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_final_quote_amount_raw: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub projected_net_return_bps: Option<String>,
    pub last_checked_at_ms: Option<i64>,
    pub evidence_source: Option<String>,
    pub problem: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainCrossChainAuthorizationEvidence {
    pub actor: String,
    pub authorized_at_ms: i64,
    pub valid_until_ms: i64,
    pub confirmation_version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainCrossChainRun {
    pub run_id: String,
    pub build: OnchainCrossChainBuildResponse,
    pub idempotency_key: String,
    pub status: OnchainCrossChainRunStatus,
    pub authorization: OnchainCrossChainAuthorizationEvidence,
    pub active_position: Option<u8>,
    #[serde(default)]
    pub legs: Vec<OnchainCrossChainLegProgress>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub next_action: String,
    pub problem: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accounting: Option<OnchainCrossChainAccounting>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnchainCrossChainFlowKind {
    Swap,
    Bridge,
    Recovery,
    NetworkFee,
    OtherNativeChange,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainCrossChainRecovery {
    pub provider_status: String,
    pub substatus: Option<String>,
    pub message: Option<String>,
    pub provider_transaction_id: Option<String>,
    pub sending_transaction_id: Option<String>,
    pub sending_chain_id: Option<u64>,
    pub receiving_transaction_id: Option<String>,
    pub receiving_chain_id: Option<u64>,
    pub receiving_token_chain_id: Option<u64>,
    pub receiving_token: Option<String>,
    pub reported_amount_raw: Option<String>,
    pub reported_receiver: Option<String>,
    pub observed_at_ms: i64,
    pub official_docs_url: String,
    #[serde(default)]
    pub token_resolution: Option<super::OnchainTokenResolution>,
    #[serde(default)]
    pub receipt: Option<super::OnchainWalletReceipt>,
    pub problem: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainCrossChainAssetChange {
    pub chain: String,
    pub wallet: String,
    pub asset: super::OnchainExecutionToken,
    pub amount_exact: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainCrossChainCashFlow {
    pub position: u8,
    pub transaction_id: String,
    pub kind: OnchainCrossChainFlowKind,
    pub change: OnchainCrossChainAssetChange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnchainCrossChainDispositionAction {
    Keep,
    QuoteSwap,
    QuoteBridge,
    ReviewWallet,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainCrossChainRemainingAsset {
    pub change: OnchainCrossChainAssetChange,
    pub action: OnchainCrossChainDispositionAction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainCrossChainDisposition {
    pub source_run_id: String,
    pub receipts_observed_at_ms: Option<i64>,
    pub original_capital: Option<OnchainCrossChainAssetChange>,
    /// Run-attributable capital, not a current wallet balance or an executable quote.
    pub remaining_assets: Vec<OnchainCrossChainRemainingAsset>,
    pub blockers: Vec<String>,
    pub submit_ready: bool,
    pub requires_live_authorization: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainCrossChainRecoveryPreviewRequest {
    pub run_id: String,
    pub expected_run_updated_at_ms: i64,
    pub asset_index: usize,
    pub amount_exact: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainCrossChainRecoveryPreview {
    #[serde(default)]
    pub plan_id: Option<String>,
    pub source_run_id: String,
    pub source_run_updated_at_ms: i64,
    pub asset_index: usize,
    pub input: OnchainCrossChainAssetChange,
    pub target: OnchainCrossChainAssetChange,
    pub input_amount_raw: String,
    pub balance_amount_raw: Option<String>,
    pub balance_source: Option<String>,
    pub balance_checked_at_ms: Option<i64>,
    pub route_id: Option<String>,
    pub provider: String,
    pub expected_output_amount_raw: Option<String>,
    pub minimum_output_amount_raw: Option<String>,
    pub fee_usd: Option<f64>,
    pub gas_usd: Option<f64>,
    pub estimated_duration_seconds: Option<u64>,
    pub quote_observed_at_ms: Option<i64>,
    pub valid_until_ms: Option<i64>,
    pub blockers: Vec<String>,
    pub quote_ready: bool,
    pub submit_ready: bool,
    pub requires_live_authorization: bool,
    pub official_docs_url: String,
}

pub const ONCHAIN_RECOVERY_RESERVATION_PHRASE: &str = "RESERVE CROSS CHAIN RECOVERY";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnchainCrossChainRecoveryPlanStatus { AwaitingAuthorization, Reserved, Expired, Cancelled }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainCrossChainRecoveryPlan {
    pub plan_id: String,
    pub preview: OnchainCrossChainRecoveryPreview,
    pub status: OnchainCrossChainRecoveryPlanStatus,
    pub authorization: Option<OnchainCrossChainAuthorizationEvidence>,
    pub idempotency_key: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

impl OnchainCrossChainRecoveryPlan {
    pub fn reservation_active(&self, now_ms: i64) -> bool {
        self.status == OnchainCrossChainRecoveryPlanStatus::Reserved
            && self.authorization.as_ref().is_some_and(|auth| now_ms < auth.valid_until_ms)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainCrossChainRecoveryAuthorizeRequest {
    pub plan_id: String,
    pub idempotency_key: String,
    pub confirmation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainCrossChainRecoveryCancelRequest { pub plan_id: String }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainCrossChainAccounting {
    pub status: super::OnchainExecutionAccountingStatus,
    pub flows: Vec<OnchainCrossChainCashFlow>,
    /// Independent actual costs; not mixed into the four-leg wallet transfer amounts.
    #[serde(default)]
    pub external_flows: Vec<super::OnchainExecutionCashFlow>,
    /// Kept by chain, wallet and contract; a shared symbol does not merge assets.
    pub net_assets: Vec<OnchainCrossChainAssetChange>,
    pub usd_value: Option<super::OnchainExecutionUsdValue>,
    pub problems: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disposition: Option<OnchainCrossChainDisposition>,
}

impl OnchainCrossChainLegProgress {
    pub const RECOVERY_CHECK_LIMIT: u8 = 12;
}

impl OnchainCrossChainRun {
    pub fn active_leg(&self) -> Option<&OnchainCrossChainLegProgress> {
        let position = self.active_position?;
        self.legs.iter().find(|leg| leg.position == position)
    }

    pub fn automatic_check_deadline_ms(&self) -> Option<i64> {
        let leg = self.active_leg()?;
        if leg.recovery_started_at_ms.is_some() {
            return None;
        }
        let window = match self.status {
            OnchainCrossChainRunStatus::AwaitingSourceFinality => 10 * 60_000,
            OnchainCrossChainRunStatus::AwaitingDestinationEvidence => 2 * 60 * 60_000,
            _ => return None,
        };
        leg.source_submitted_at_ms
            .filter(|time| *time > 0)
            .map(|time| time.saturating_add(window))
    }

    pub fn reconciliation_due(&self, now_ms: i64) -> bool {
        if !matches!(
            self.status,
            OnchainCrossChainRunStatus::AwaitingSourceFinality
                | OnchainCrossChainRunStatus::AwaitingDestinationEvidence
        ) {
            return false;
        }
        let Some(leg) = self.active_leg() else {
            return false;
        };
        if leg.recovery_started_at_ms.is_none()
            && self.automatic_check_deadline_ms().is_none_or(|deadline| now_ms >= deadline)
        {
            return true;
        }
        let interval = if leg.recovery_started_at_ms.is_some() {
            60_000
        } else {
            10_000
        };
        leg.last_checked_at_ms.is_none_or(|time| now_ms.saturating_sub(time) >= interval)
    }

    pub fn accounting_refresh_due(&self, now_ms: i64) -> bool {
        self.status == OnchainCrossChainRunStatus::Completed
            && self.accounting.as_ref().is_some_and(|a| {
                a.status == super::OnchainExecutionAccountingStatus::PendingValuation
            })
            && self
                .legs
                .iter()
                .flat_map(|leg| {
                    [
                        leg.source_receipt.as_ref(),
                        leg.destination_receipt.as_ref(),
                    ]
                })
                .flatten()
                .filter_map(|receipt| receipt.observed_at_ms)
                .max()
                .is_some_and(|last| now_ms >= last && now_ms.saturating_sub(last) <= 600_000)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainCrossChainRunsResponse {
    pub rows: Vec<OnchainCrossChainRun>,
    pub observed_at_ms: i64,
    #[serde(default)]
    pub recovery_problem: Option<String>,
    #[serde(default)]
    pub recovery_plans: Vec<OnchainCrossChainRecoveryPlan>,
}
