use serde::{Deserialize, Serialize};

use super::{
    OnchainComparisonDirection, OnchainTransferDirection, OnchainTransferStatus,
    OnchainUsdValuation,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainReplenishmentBuildRequest {
    pub direction: OnchainComparisonDirection,
    pub expected_quote_observed_at_ms: i64,
    pub expected_cex_observed_at_ms: i64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnchainReplenishmentDestinationStatus {
    ConfiguredUnverified,
    Verified,
    #[default]
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainReplenishmentDestination {
    pub address: Option<String>,
    pub tag: Option<String>,
    pub status: OnchainReplenishmentDestinationStatus,
    pub source: Option<String>,
    pub observed_at_ms: Option<i64>,
    pub problem: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainReplenishmentLeg {
    pub direction: OnchainTransferDirection,
    pub venue: String,
    pub asset: String,
    pub chain: String,
    #[serde(default)]
    pub source_address: Option<String>,
    #[serde(default)]
    pub asset_address: Option<String>,
    #[serde(default)]
    pub asset_decimals: Option<u8>,
    pub transfer_amount: f64,
    #[serde(default)]
    pub transfer_amount_exact: Option<String>,
    pub economics: OnchainReplenishmentLegEconomics,
    pub network_evidence: OnchainReplenishmentNetworkEvidence,
    pub destination: OnchainReplenishmentDestination,
    pub blocker: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainReplenishmentLegEconomics {
    #[serde(default)]
    pub estimated_cost_usd: Option<f64>,
    #[serde(default)]
    pub estimated_network_cost_usd: Option<f64>,
    #[serde(default)]
    pub reconciled_cost_usd: Option<f64>,
    pub fee_amount: Option<f64>,
    #[serde(default)]
    pub fee_amount_exact: Option<String>,
    pub source_debit_upper_bound: Option<f64>,
    #[serde(default)]
    pub source_debit_upper_bound_exact: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainReplenishmentNetworkEvidence {
    pub network: Option<String>,
    pub minimum_amount: Option<f64>,
    #[serde(default)]
    pub minimum_amount_exact: Option<String>,
    #[serde(default)]
    pub amount_step: Option<String>,
    pub credit_confirmations: Option<u64>,
    pub unlock_confirmations: Option<u64>,
    pub transfer_status: OnchainTransferStatus,
    pub evidence_source: Option<String>,
    pub evidence_observed_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnchainReplenishmentPlanStatus {
    ReadyForAuthorization,
    Unprofitable,
    #[default]
    EvidencePending,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainReplenishmentPlanResponse {
    pub plan_id: String,
    pub direction: OnchainComparisonDirection,
    pub status: OnchainReplenishmentPlanStatus,
    pub legs: Vec<OnchainReplenishmentLeg>,
    pub transfer_cost_usd: Option<f64>,
    pub post_transfer_net_profit_usd: Option<f64>,
    pub built_at_ms: i64,
    pub valid_until_ms: i64,
    pub requires_live_authorization: bool,
    pub submit_ready: bool,
    pub blockers: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainReplenishmentPlansResponse {
    pub rows: Vec<OnchainReplenishmentPlanResponse>,
    pub observed_at_ms: i64,
}

pub const ONCHAIN_REPLENISHMENT_AUTHORIZATION_PHRASE: &str = "AUTHORIZE LIVE REPLENISHMENT";
pub const ONCHAIN_REPLENISHMENT_CHAIN_WAIT_MS: i64 = 10 * 60_000;
pub const ONCHAIN_REPLENISHMENT_SOURCE_WAIT_MS: i64 = 2 * 60 * 60_000;
pub const ONCHAIN_REPLENISHMENT_DESTINATION_WAIT_MS: i64 = 2 * 60 * 60_000;
pub const ONCHAIN_REPLENISHMENT_RECOVERY_LIMIT: u8 = 12;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainReplenishmentAuthorizeRequest {
    pub plan_id: String,
    pub idempotency_key: String,
    pub confirmation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainReplenishmentSubmitRequest {
    pub run_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainReplenishmentRecheckRequest {
    pub run_id: String,
    pub expected_client_transfer_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnchainReplenishmentRunStatus {
    AuthorizedAwaitingSubmit,
    AuthorizationExpired,
    ReadyForNextTransfer,
    Submitting,
    AwaitingSourceFinality,
    AwaitingDestinationCredit,
    Completed,
    Paused,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnchainReplenishmentTransferStatus {
    SubmissionClaimed,
    Submitted,
    SourceCompleted,
    DestinationCredited,
    Paused,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainReplenishmentWithdrawalCost {
    pub asset: String,
    pub reported_amount_exact: String,
    pub fee_exact: String,
    pub confirmed: bool,
    pub source: String,
    pub observed_at_ms: i64,
    #[serde(default)]
    pub usd_valuation: Option<OnchainReplenishmentCostValuation>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainReplenishmentCostValuation {
    pub usd_amount_exact: String,
    pub quote: Option<OnchainUsdValuation>,
    pub valued_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainReplenishmentNetworkCost {
    pub chain: String,
    pub transaction_id: String,
    pub block_ref: String,
    pub payer: String,
    pub asset: String,
    pub execution_fee_exact: Option<String>,
    pub additional_fee_exact: Option<String>,
    pub total_fee_exact: Option<String>,
    pub source: String,
    pub observed_at_ms: i64,
    pub problem: Option<String>,
    #[serde(default)]
    pub usd_valuation: Option<OnchainReplenishmentCostValuation>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainReplenishmentTransferProgress {
    pub leg_index: u32,
    pub client_transfer_id: String,
    pub provider_transfer_id: Option<String>,
    pub status: OnchainReplenishmentTransferStatus,
    pub submission_attempted_at_ms: i64,
    pub last_checked_at_ms: Option<i64>,
    pub transaction_id: Option<String>,
    pub confirmations: Option<u64>,
    #[serde(default)]
    pub credited_amount_exact: Option<String>,
    #[serde(default)]
    pub reported_deposit_amount_exact: Option<String>,
    #[serde(default)]
    pub deposit_fee_exact: Option<String>,
    #[serde(default)]
    pub withdrawal_unlocked: Option<bool>,
    #[serde(default)]
    pub withdrawal_cost: Option<OnchainReplenishmentWithdrawalCost>,
    #[serde(default)]
    pub network_cost: Option<OnchainReplenishmentNetworkCost>,
    pub evidence_source: Option<String>,
    pub problem: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainReplenishmentAuthorizationEvidence {
    pub actor: String,
    pub authorized_at_ms: i64,
    pub valid_until_ms: i64,
    pub confirmation_version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainReplenishmentRun {
    pub run_id: String,
    pub plan: OnchainReplenishmentPlanResponse,
    pub idempotency_key: String,
    pub status: OnchainReplenishmentRunStatus,
    pub authorization: OnchainReplenishmentAuthorizationEvidence,
    #[serde(default)]
    pub transfers: Vec<OnchainReplenishmentTransferProgress>,
    #[serde(default)]
    pub revalidated_at_ms: Option<i64>,
    #[serde(default)]
    pub read_only_recovery: bool,
    #[serde(default)]
    pub recovery_checks: u8,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub next_action: String,
    pub problem: Option<String>,
}

impl OnchainReplenishmentRun {
    pub fn automatic_wait_deadline_ms(&self) -> Option<i64> {
        if self.read_only_recovery { return None; }
        let transfer = self.transfers.last()?;
        if transfer.submission_attempted_at_ms <= 0 { return None; }
        let duration = match self.status {
            OnchainReplenishmentRunStatus::Submitting | OnchainReplenishmentRunStatus::AwaitingSourceFinality => {
                match self.plan.legs.get(transfer.leg_index as usize)?.direction {
                    OnchainTransferDirection::WithdrawToChain => ONCHAIN_REPLENISHMENT_SOURCE_WAIT_MS,
                    OnchainTransferDirection::DepositToCex => ONCHAIN_REPLENISHMENT_CHAIN_WAIT_MS,
                }
            }
            OnchainReplenishmentRunStatus::AwaitingDestinationCredit => ONCHAIN_REPLENISHMENT_DESTINATION_WAIT_MS,
            _ => return None,
        };
        Some(transfer.submission_attempted_at_ms.saturating_add(duration))
    }

    pub fn recheck_request(&self) -> Option<OnchainReplenishmentRecheckRequest> {
        if self.status != OnchainReplenishmentRunStatus::Paused { return None; }
        let transfer = self.transfers.last()?;
        let locked_credit = transfer.status == OnchainReplenishmentTransferStatus::DestinationCredited
            && transfer.withdrawal_unlocked == Some(false);
        if !locked_credit && !matches!(transfer.status, OnchainReplenishmentTransferStatus::Paused
            | OnchainReplenishmentTransferStatus::SubmissionClaimed
            | OnchainReplenishmentTransferStatus::Submitted
            | OnchainReplenishmentTransferStatus::SourceCompleted) { return None; }
        let leg = self.plan.legs.get(transfer.leg_index as usize)?;
        if transfer.client_transfer_id.trim().is_empty()
            || (leg.direction == OnchainTransferDirection::DepositToCex
                && transfer.transaction_id.as_deref().is_none_or(|id| id.trim().is_empty())) { return None; }
        if leg.direction == OnchainTransferDirection::WithdrawToChain
            && matches!(crate::venue_family(&leg.venue), "bybit" | "kraken")
            && transfer.status != OnchainReplenishmentTransferStatus::SourceCompleted
            && transfer.provider_transfer_id.as_deref().is_none_or(|id| id.trim().is_empty()) {
            return None;
        }
        Some(OnchainReplenishmentRecheckRequest {
            run_id: self.run_id.clone(),
            expected_client_transfer_id: transfer.client_transfer_id.clone(),
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainReplenishmentRunsResponse {
    pub rows: Vec<OnchainReplenishmentRun>,
    pub observed_at_ms: i64,
    #[serde(default)]
    pub recovery_problem: Option<String>,
}
