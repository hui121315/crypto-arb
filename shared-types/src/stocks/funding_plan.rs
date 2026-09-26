use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StockFundingTarget {
    Backpack,
    Solana,
}
impl StockFundingTarget {
    pub fn label(self) -> &'static str {
        match self {
            Self::Backpack => "Backpack",
            Self::Solana => "Solana",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockFundingPlanRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_plan: Option<StockInventorySource>,
    pub request_id: String,
    pub security_asset: String,
    pub funding_asset: String,
    pub direction: StockChainDirection,
    pub target: StockFundingTarget,
    pub wallet_address: String,
    pub preflight_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockWithdrawalCapacity {
    pub asset: String,
    pub quantity: String,
    pub checked_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockFundingPlanTerms {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_plan: Option<Box<StockExecutionPlan>>,
    pub account_fingerprint: String,
    pub security: StockSecurity,
    pub mint: StockMintEvidence,
    pub token: StockChainToken,
    pub need: StockFundingNeed,
    pub destination: String,
    pub deposit_address: Option<StockDepositAddress>,
    pub withdrawal_capacity: Option<StockWithdrawalCapacity>,
    pub quantity: String,
    pub minimum_credit_raw: String,
    pub source_budget: String,
    pub created_at_ms: i64,
    pub valid_until_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StockFundingPlanPhase {
    Reserved,
    Cancelled,
    Expired,
    Withdrawing,
    Received,
    Transferring,
    DepositPending,
    Deposited,
    TransferFailed,
}

impl StockFundingPlanPhase {
    pub fn holds_funds(self) -> bool {
        matches!(
            self,
            Self::Reserved
                | Self::Withdrawing
                | Self::Received
                | Self::Transferring
                | Self::DepositPending
        )
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Reserved => "已预留 · 未转账",
            Self::Cancelled => "已取消 · 未转账",
            Self::Expired => "预留已到期 · 未转账",
            Self::Withdrawing => "已记录提交 · 到账待核验",
            Self::Received => "链上已到账 · 扣账待核清",
            Self::Transferring => "链上已记录提交 · 原交易待确认",
            Self::DepositPending => "链上已转出 · Backpack 入账待核验",
            Self::Deposited => "Backpack 已确认入账",
            Self::TransferFailed => "链上失败 · 已记录实际网络费",
        }
    }
}

// The issued 2FA token is transient; never derive Debug or put it in a journal.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockFundingSubmitRequest {
    pub plan_id: String,
    pub revision: u64,
    pub confirm_live: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub two_factor_token: Option<String>,
}

impl std::fmt::Debug for StockFundingSubmitRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StockFundingSubmitRequest")
            .field("plan_id", &self.plan_id)
            .field("revision", &self.revision)
            .field("confirm_live", &self.confirm_live)
            .field("two_factor_token_present", &self.two_factor_token.is_some())
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockWithdrawalRecord {
    pub id: i32,
    pub client_id: String,
    pub blockchain: String,
    pub symbol: String,
    pub to_address: String,
    pub quantity: String,
    pub fee: Option<String>,
    pub status: String,
    pub transaction_hash: Option<String>,
    pub created_at: String,
    pub is_internal: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockFundingReceipt {
    pub transaction_hash: String,
    pub destination: String,
    pub mint: String,
    pub decimals: u8,
    pub credited_raw: String,
    pub slot: u64,
    pub block_time_ms: i64,
    pub network_fee_lamports: u64,
    pub fee_payer: String,
    pub checked_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockFundingWithdrawal {
    pub client_id: String,
    pub submitted_at_ms: i64,
    pub query_count: u32,
    pub last_query_at_ms: Option<i64>,
    pub remote: Option<StockWithdrawalRecord>,
    pub receipt: Option<StockFundingReceipt>,
    pub problem: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_conflict: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockFundingFollowup {
    pub attempts: u8,
    pub last_at_ms: i64,
    pub next_at_ms: Option<i64>,
    pub paused: bool,
    pub problem: Option<String>,
}

pub const STOCK_FUNDING_FOLLOWUP_LIMIT: u8 = 6;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockFundingPlan {
    pub plan_id: String,
    pub request: StockFundingPlanRequest,
    pub terms: StockFundingPlanTerms,
    pub phase: StockFundingPlanPhase,
    pub revision: u64,
    pub updated_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub withdrawal: Option<StockFundingWithdrawal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transfer: Option<StockFundingTransfer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub followup: Option<StockFundingFollowup>,
}

impl StockFundingPlan {
    pub fn funding_receipt_pending(&self) -> bool {
        match self.request.target {
            StockFundingTarget::Solana => {
                self.phase == StockFundingPlanPhase::Withdrawing
                    && self
                        .withdrawal
                        .as_ref()
                        .is_some_and(|w| w.receipt.is_none() && w.evidence_conflict.is_none())
            }
            StockFundingTarget::Backpack => {
                self.phase.holds_funds()
                    && self.transfer.as_ref().is_some_and(|t| {
                        t.submitted_at_ms.is_some()
                            && t.evidence_conflict.is_none()
                            && t.receipt
                                .as_ref()
                                .is_none_or(|r| r.within_plan && r.succeeded)
                    })
            }
        }
    }

    pub fn funding_followup_at(&self) -> Option<i64> {
        if !self.funding_receipt_pending() {
            return None;
        }
        let (submitted, last_query) = match self.request.target {
            StockFundingTarget::Solana => {
                let w = self.withdrawal.as_ref()?;
                (w.submitted_at_ms, w.last_query_at_ms)
            }
            StockFundingTarget::Backpack => {
                let t = self.transfer.as_ref()?;
                (t.submitted_at_ms?, t.last_query_at_ms)
            }
        };
        let next = match &self.followup {
            Some(f) if f.paused || f.attempts >= STOCK_FUNDING_FOLLOWUP_LIMIT => return None,
            Some(f) => f.next_at_ms?,
            None => submitted.saturating_add(5_000),
        };
        Some(next.max(last_query.unwrap_or(submitted).saturating_add(5_000)))
    }

    pub fn phase_at(&self, now: i64) -> StockFundingPlanPhase {
        if self.phase == StockFundingPlanPhase::Reserved && now >= self.terms.valid_until_ms {
            StockFundingPlanPhase::Expired
        } else {
            self.phase
        }
    }
}
