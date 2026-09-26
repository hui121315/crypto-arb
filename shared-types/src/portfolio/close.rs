use super::*;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClosePositionRequest {
    #[serde(default)]
    pub side: Option<PositionSide>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_leg_count: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloseAllPositionsRequest {
    pub confirmation_phrase: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_leg_count: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloseRunCompensationRequest {
    pub confirmation_phrase: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_index: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_quantity: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit_price: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloseRunManualTerminalRequest {
    pub confirmation_phrase: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_version: Option<String>,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manual_handling_cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloseRunScope {
    Single,
    Pair,
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloseRunStatus {
    Submitted,
    Succeeded,
    PartiallySubmitted,
    UnwindRequired,
    CompensationSubmitted,
    Compensated,
    CompensationFailed,
    ManuallyResolved,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloseLegStatus {
    Submitted,
    Accepted,
    PartiallyFilled,
    Filled,
    CancelRequested,
    Cancelled,
    Rejected,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloseLeg {
    pub venue: String,
    pub symbol: String,
    pub side: PositionSide,
    pub status: CloseLegStatus,
    pub quantity: f64,
    pub mark_price: f64,
    pub notional_usd: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<OrderRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finality_source: Option<OrderUpdateSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmed_filled_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<ApiProblem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pair_evidence: Option<PositionPairEvidence>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cost_events: Vec<CloseRunCostLedgerEvent>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloseRunUnwindPlanStatus {
    BlockedPendingManualRecheck,
    CompensationSubmitted,
    Compensated,
    CompensationFailed,
    ManualTerminalRecorded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloseRunNextActionKind {
    SubmitCompensationOrder,
    CancelCompensationOrder,
    WaitForCompensationFinality,
    ManualIncidentReview,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloseRunNextAction {
    pub kind: CloseRunNextActionKind,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_index: Option<usize>,
    #[serde(default)]
    pub requires_confirmation: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_evidence: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloseRunCompensationAttempt {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_run_id: Option<String>,
    pub venue: String,
    pub symbol: String,
    pub side: PositionSide,
    pub compensation_order_side: OrderSide,
    pub target_quantity: f64,
    pub status: CloseLegStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<OrderRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finality_source: Option<OrderUpdateSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmed_filled_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<ApiProblem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cost_events: Vec<CloseRunCostLedgerEvent>,
    pub submitted_at_ms: i64,
    pub updated_at_ms: i64,
}

impl CloseRunCompensationAttempt {
    pub fn cancellable_order_id(&self) -> Option<&str> {
        use crate::{LiveOrderState, OrderSource};
        let order = self.order.as_ref()?;
        (matches!(
            self.status,
            CloseLegStatus::Submitted | CloseLegStatus::Accepted | CloseLegStatus::PartiallyFilled
        ) && order.intent.source == OrderSource::CloseRunCompensation
            && matches!(
                order.state,
                LiveOrderState::Submitted
                    | LiveOrderState::Accepted
                    | LiveOrderState::PartiallyFilled
                    | LiveOrderState::Unknown
            )
            && !order.intent.id.trim().is_empty())
        .then_some(order.intent.id.as_str())
    }

    pub fn confirmed_filled_quantity(&self) -> Option<f64> {
        self.order
            .as_ref()?
            .filled_quantity
            .filter(|qty| qty.is_finite() && *qty >= 0.0)
    }

    pub fn unfilled_quantity(&self) -> Option<f64> {
        let filled = self.confirmed_filled_quantity()?;
        (self.target_quantity.is_finite()
            && self.target_quantity > 0.0
            && filled <= self.target_quantity)
            .then_some(self.target_quantity - filled)
    }

    pub fn terminal_without_fill(&self) -> bool {
        use crate::LiveOrderState;
        self.order.as_ref().is_some_and(|order| {
            matches!((self.status, order.state),
                (CloseLegStatus::Cancelled, LiveOrderState::Cancelled)
                    | (CloseLegStatus::Rejected, LiveOrderState::Rejected)
                    | (CloseLegStatus::Failed, LiveOrderState::Failed))
                // A cancel ACK with no cumulative quantity is not proof of zero fills.
                && self.confirmed_filled_quantity() == Some(0.0)
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloseRunCostComponent {
    Fee,
    Slippage,
    Funding,
    ManualHandling,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloseRunCostLedgerEvent {
    pub event_id: String,
    pub component: CloseRunCostComponent,
    pub amount_usd: f64,
    pub source: OrderUpdateSource,
    pub quality: ExecutionLedgerQuality,
    pub occurred_at_ms: i64,
    pub captured_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloseRunManualTerminalEvidence {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_run_id: Option<String>,
    pub actor: String,
    pub reason: String,
    pub snapshot_version: String,
    pub recorded_at_ms: i64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remaining_positions: Vec<CloseRunUnwindLegEvidence>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_evidence: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manual_handling_cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manual_handling_event_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloseRunUnwindLegEvidence {
    pub venue: String,
    pub symbol: String,
    pub side: PositionSide,
    pub status: CloseLegStatus,
    pub target_quantity: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmed_quantity: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmed_price: Option<f64>,
    pub mark_price: f64,
    pub notional_usd: f64,
    #[serde(default = "default_unwind_notional_quality")]
    pub notional_quality: ExecutionLedgerQuality,
    #[serde(default = "default_unwind_notional_source")]
    pub notional_source: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notional_missing_fields: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compensation_order_side: Option<OrderSide>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_order_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exchange_order_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finality_source: Option<OrderUpdateSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmed_filled_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<ApiProblem>,
}

fn default_unwind_notional_quality() -> ExecutionLedgerQuality {
    ExecutionLedgerQuality::Estimated
}

fn default_unwind_notional_source() -> String {
    "legacy_unclassified".to_owned()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloseRunUnwindPlan {
    pub status: CloseRunUnwindPlanStatus,
    pub filled_legs: Vec<CloseRunUnwindLegEvidence>,
    pub failed_legs: Vec<CloseRunUnwindLegEvidence>,
    pub compensation_candidates: Vec<CloseRunUnwindLegEvidence>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remaining_positions: Vec<CloseRunUnwindLegEvidence>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub compensation_attempts: Vec<CloseRunCompensationAttempt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manual_terminal_evidence: Option<CloseRunManualTerminalEvidence>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub next_actions: Vec<CloseRunNextAction>,
    pub required_evidence: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloseRunCostReconciliation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub close_fee_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub close_slippage_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compensation_fee_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compensation_slippage_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub funding_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manual_handling_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_actual_cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence_order_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence_event_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub close_fee_event_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub close_slippage_event_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub compensation_fee_event_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub compensation_slippage_event_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub funding_event_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub manual_handling_event_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_fields: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloseRun {
    pub id: String,
    pub scope: CloseRunScope,
    pub status: CloseRunStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    pub snapshot_version: String,
    pub expected_leg_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub legs: Vec<CloseLeg>,
    pub submitted_order_count: usize,
    pub failed_leg_count: usize,
    pub naked_exposure_usd: f64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<ApiProblem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finality_problem: Option<ApiProblem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finality_checked_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unwind_plan: Option<CloseRunUnwindPlan>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cost_events: Vec<CloseRunCostLedgerEvent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_reconciliation: Option<CloseRunCostReconciliation>,
    pub started_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloseRunEvent {
    pub event: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub close_run: Option<CloseRun>,
    pub timestamp_ms: i64,
}
