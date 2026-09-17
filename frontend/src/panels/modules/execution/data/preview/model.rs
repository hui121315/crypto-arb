use leptos::prelude::*;
use shared_types::{
    HedgeExecutionParams, OpportunityLegMarketEvidence, OrderCompilePlan, OrderType,
};

use super::super::super::selection::ExecutionSelection;

#[path = "model/evidence.rs"]
mod evidence;

pub(in crate::panels::modules::execution) use evidence::{
    PreviewDepth, PreviewFeeEvidence, PreviewFundingWindowEvidence, PreviewLiquidation,
    PreviewOneCycleCost, PreviewProfitEvidence, PreviewRisk,
};

const TICKET_SUBMIT_SAFETY_MS: i64 = 2_000;
const RETRYABLE_MARKET_BLOCKER_MARKERS: [&str; 5] = [
    "orderbook 超过",
    "orderbook 使用短时缓存",
    "orderbook 暂无可用数据",
    "orderbook 触发限频退避",
    "等待双腿 0.05% 盘口深度",
];

#[derive(Clone, PartialEq)]
pub(super) struct PreviewInput {
    pub capital_usd: f64,
    pub leverage: f64,
    pub order_type: OrderType,
    pub limit_offset_bps: f64,
    pub long_price: Option<f64>,
    pub short_price: Option<f64>,
    pub long_notional_usd: f64,
    pub short_notional_usd: f64,
    pub execution_params: HedgeExecutionParams,
}

#[derive(Clone, Copy)]
pub(in crate::panels::modules::execution) struct PreviewSignals {
    pub selection: Memo<ExecutionSelection>,
    pub selection_state: RwSignal<ExecutionSelection>,
    pub capital_usd: RwSignal<String>,
    pub leverage: RwSignal<String>,
    pub order_type: RwSignal<String>,
    pub limit_offset_bps: RwSignal<String>,
    pub long_price: RwSignal<String>,
    pub short_price: RwSignal<String>,
    pub long_notional_usd: RwSignal<String>,
    pub short_notional_usd: RwSignal<String>,
    pub margin_mode: RwSignal<String>,
    pub time_in_force: RwSignal<String>,
}

#[derive(Clone, PartialEq)]
pub(super) struct PreviewQuery {
    pub seed: PreviewSeed,
    pub input: PreviewInput,
}

#[derive(Clone, PartialEq)]
pub(in crate::panels::modules::execution) struct PreviewSeed {
    pub opportunity_id: String,
    pub opportunity_snapshot_id: String,
    pub long_market_evidence: Option<OpportunityLegMarketEvidence>,
    pub short_market_evidence: Option<OpportunityLegMarketEvidence>,
    pub default_capital_usd: f64,
    pub default_leverage: f64,
    pub execution_blockers: Vec<String>,
    pub profit_evidence: PreviewProfitEvidence,
}

impl PreviewSeed {
    pub(in crate::panels::modules::execution) fn from_selection(
        selection: &ExecutionSelection,
    ) -> Self {
        Self {
            opportunity_id: selection.opportunity_id.clone(),
            opportunity_snapshot_id: selection.opportunity_snapshot_id.clone(),
            long_market_evidence: selection.long_market_evidence.clone(),
            short_market_evidence: selection.short_market_evidence.clone(),
            default_capital_usd: selection.default_capital_usd,
            default_leverage: selection.default_leverage,
            execution_blockers: selection.execution_blockers.clone(),
            profit_evidence: PreviewProfitEvidence::from_selection(selection),
        }
    }

    pub(super) fn has_opportunity(&self) -> bool {
        !self.opportunity_id.trim().is_empty()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::panels::modules::execution) enum PreviewReadiness {
    Pending,
    Ready,
    Stale,
    Error,
}

#[derive(Clone, PartialEq)]
pub(in crate::panels::modules::execution) struct ExecutionPreview {
    pub opportunity_id: String,
    pub opportunity_snapshot_id: String,
    pub idempotency_key: Option<String>,
    pub ticket_id: Option<String>,
    pub expires_at_ms: Option<i64>,
    pub readiness: PreviewReadiness,
    pub source: &'static str,
    pub estimated_funding_usd: f64,
    pub open_cost_usd: f64,
    pub close_cost_usd: f64,
    pub slippage_cost_usd: f64,
    pub one_cycle_cost: Option<PreviewOneCycleCost>,
    pub max_loss_usd: f64,
    pub used_capital_usd: f64,
    pub liquidation: PreviewLiquidation,
    pub execution_mode_label: &'static str,
    pub long_allowed: bool,
    pub short_allowed: bool,
    pub long_notional_usd: f64,
    pub short_notional_usd: f64,
    pub long_reference_price: Option<f64>,
    pub short_reference_price: Option<f64>,
    pub long_market_evidence: Option<OpportunityLegMarketEvidence>,
    pub short_market_evidence: Option<OpportunityLegMarketEvidence>,
    pub depth: PreviewDepth,
    pub fee_evidence: Vec<PreviewFeeEvidence>,
    pub profit_evidence: PreviewProfitEvidence,
    pub order_plans: Vec<OrderCompilePlan>,
    pub identity_evidence_required: bool,
    pub risk: PreviewRisk,
}

impl ExecutionPreview {
    pub(crate) fn can_submit(&self) -> bool {
        self.is_ready()
            && self.idempotency_key.is_some()
            && self
                .ticket_id
                .as_deref()
                .is_some_and(|ticket_id| !ticket_id.trim().is_empty())
            && self.long_allowed
            && self.short_allowed
            && self.order_identity_ready()
            && self.risk.is_clear()
    }

    pub(crate) fn is_ready(&self) -> bool {
        self.readiness == PreviewReadiness::Ready
    }

    pub(crate) fn can_submit_at(&self, now_ms: i64) -> bool {
        self.can_submit() && self.ticket_is_fresh_at(now_ms)
    }

    pub(crate) fn ticket_needs_refresh_at(&self, now_ms: i64) -> bool {
        self.is_ready()
            && self
                .ticket_id
                .as_deref()
                .is_some_and(|ticket_id| !ticket_id.trim().is_empty())
            && !self.ticket_is_fresh_at(now_ms)
    }

    pub(crate) fn has_retryable_market_blocker(&self) -> bool {
        self.is_ready()
            && self.risk.blockers.iter().any(|blocker| {
                RETRYABLE_MARKET_BLOCKER_MARKERS
                    .iter()
                    .any(|marker| blocker.contains(marker))
            })
    }

    pub(crate) fn total_cost_usd(&self) -> f64 {
        self.open_cost_usd + self.close_cost_usd + self.slippage_cost_usd
    }

    pub(crate) fn net_edge_usd(&self) -> f64 {
        self.estimated_funding_usd - self.total_cost_usd()
    }

    fn order_identity_ready(&self) -> bool {
        if !self.order_plans.iter().all(|plan| plan.blockers.is_empty()) {
            return false;
        }
        if !self.identity_evidence_required {
            return true;
        }
        self.order_plans.len() == 2
            && [
                shared_types::HedgeLegRole::Long,
                shared_types::HedgeLegRole::Short,
            ]
            .iter()
            .all(|role| {
                self.order_plans
                    .iter()
                    .any(|plan| plan.role == *role && plan.identity_plan().is_execution_ready())
            })
    }

    fn ticket_is_fresh_at(&self, now_ms: i64) -> bool {
        self.expires_at_ms.is_some_and(|expires_at_ms| {
            expires_at_ms.saturating_sub(now_ms) > TICKET_SUBMIT_SAFETY_MS
        })
    }
}
