use super::*;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockPlanRequest {
    pub request_id: String,
    pub asset: String,
    pub direction: StockChainDirection,
    pub wallet_address: String,
    pub preflight_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build: Option<StockPlanBuildRequest>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockPlanBuildRequest {
    pub request_id: String,
    pub asset: String,
    pub direction: StockChainDirection,
    pub wallet_address: String,
    pub input_raw: String,
    pub keyed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockPlanCancelRequest {
    pub plan_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StockPlanPhase {
    Reserved,
    Cancelled,
    Expired,
    SubmissionUnknown,
    Settled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockPlanRevisionRequest {
    pub plan_id: String,
    pub revision: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StockExecutionAction {
    Pair,
    NativeTopup { index: usize },
    Recovery { index: usize },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockPlanExecutionRequest {
    pub plan_id: String,
    pub revision: u64,
    pub action: StockExecutionAction,
    pub confirm_live: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockTopupRecheckRequest {
    pub plan_id: String,
    pub index: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockNativeTopup {
    pub source_revision: u64,
    pub prepared_at_ms: i64,
    pub valuation: StockNativeValuation,
    pub wallet: StockWalletEvidence,
    pub submission: Option<StockChainSubmission>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockPlanSettlement {
    pub source_revision: u64,
    pub settled_at_ms: i64,
    pub accounting: StockPlanAccounting,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockPlanAllocation {
    pub location: String,
    pub asset: String,
    pub quantity: String,
    pub available_at_reservation: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockPlanRfq {
    pub request_id: String,
    pub rfq_id: String,
    pub candidate: StockRfqCandidate,
    pub expiry_time_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum StockCexInstruction {
    OrderBook {
        client_id: u32,
        symbol: String,
        side: StockRfqSide,
        quantity: String,
        limit_price: String,
    },
    AcceptRfq {
        rfq_id: String,
        quote_id: String,
        symbol: String,
        side: StockRfqSide,
        quantity: String,
        taker_price: String,
    },
}

impl StockCexInstruction {
    pub fn path(&self) -> &'static str {
        match self {
            Self::OrderBook { .. } => "/api/v1/order",
            Self::AcceptRfq { .. } => "/api/v1/rfq/accept",
        }
    }

    pub fn signing_instruction(&self) -> &'static str {
        match self {
            Self::OrderBook { .. } => "orderExecute",
            Self::AcceptRfq { .. } => "quoteAccept",
        }
    }

    pub fn request_body(&self) -> serde_json::Value {
        match self {
            Self::OrderBook {
                client_id,
                symbol,
                side,
                quantity,
                limit_price,
            } => serde_json::json!({
                "clientId":client_id,"symbol":symbol,"side":side,"quantity":quantity,"price":limit_price,
                "orderType":"Limit","timeInForce":"FOK","postOnly":false,"selfTradePrevention":"RejectTaker",
                "autoBorrow":false,"autoBorrowRepay":false,"autoLend":false,"autoLendRedeem":false
            }),
            Self::AcceptRfq {
                rfq_id, quote_id, ..
            } => serde_json::json!({"rfqId":rfq_id,"quoteId":quote_id}),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockPlanTerms {
    pub account_fingerprint: String,
    pub security: StockSecurity,
    pub chain_cost: StockChainCost,
    pub route: StockTradingRoute,
    pub cex_shares: String,
    pub cex_notional_usdc: String,
    pub rfq: Option<StockPlanRfq>,
    pub allocations: Vec<StockPlanAllocation>,
    pub after_known_costs_usdc: String,
    pub created_at_ms: i64,
    pub market_valid_until_ms: i64,
    pub reserved_until_ms: i64,
    // Omitting this for legacy records preserves the serialized plan identity on replay.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cex_instruction: Option<StockCexInstruction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cex_fee_budget: Option<StockCexFeeBudget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preflight_evidence: Option<StockPreflight>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockExecutionPlan {
    pub plan_id: String,
    pub request: StockPlanRequest,
    pub terms: StockPlanTerms,
    pub phase: StockPlanPhase,
    pub revision: u64,
    pub updated_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cex_order: Option<StockCexOrder>,
    // Once accepted, this journal is authoritative for the RFQ receipt too.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rfq_acceptance: Option<StockRfq>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chain_submission: Option<StockChainSubmission>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub two_leg_started_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub native_topups: Vec<StockNativeTopup>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recoveries: Vec<StockRecovery>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settlement: Option<StockPlanSettlement>,
}

impl StockExecutionPlan {
    pub fn phase_at(&self, now: i64) -> StockPlanPhase {
        if self.phase == StockPlanPhase::Reserved && now >= self.terms.reserved_until_ms {
            StockPlanPhase::Expired
        } else {
            self.phase
        }
    }

    pub fn holds_funds(&self, now: i64) -> bool {
        matches!(
            self.phase_at(now),
            StockPlanPhase::Reserved | StockPlanPhase::SubmissionUnknown
        )
    }
}
