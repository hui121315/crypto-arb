use shared_types::{ExecutionCostProfile, ExecutionGuard, HedgeDepthStatus, MarketDataHealth};

use crate::panels::modules::execution::selection::ExecutionSelection;

#[derive(Clone, PartialEq)]
pub(in crate::panels::modules::execution) struct PreviewLiquidation {
    pub current_account_pct: Option<f64>,
    pub after_hedge_pct: Option<f64>,
    pub positions_evidence: Option<shared_types::HedgePreviewPositionsEvidence>,
}

#[derive(Clone, PartialEq)]
pub(in crate::panels::modules::execution) struct PreviewDepth {
    pub long_5bps: Option<f64>,
    pub long_10bps: Option<f64>,
    pub long_20bps: Option<f64>,
    pub short_5bps: Option<f64>,
    pub short_10bps: Option<f64>,
    pub short_20bps: Option<f64>,
    pub executable_status: HedgeDepthStatus,
    pub executable_amount_usd: Option<f64>,
    pub executable_reason: Option<String>,
    pub long_reason: Option<String>,
    pub short_reason: Option<String>,
    pub long_depth_health: Option<MarketDataHealth>,
    pub short_depth_health: Option<MarketDataHealth>,
}

#[derive(Clone, PartialEq)]
pub(in crate::panels::modules::execution) struct PreviewRisk {
    pub note: String,
    pub guards: Vec<ExecutionGuard>,
    pub blockers: Vec<String>,
}

impl PreviewRisk {
    pub(super) fn is_clear(&self) -> bool {
        self.blockers.is_empty() && self.guards.iter().all(|guard| guard.passed)
    }
}

#[derive(Clone, PartialEq)]
pub(in crate::panels::modules::execution) struct PreviewFeeEvidence {
    pub label: String,
    pub rate: String,
    pub source: String,
    pub health: String,
}

#[derive(Clone, Copy, PartialEq)]
pub(in crate::panels::modules::execution) struct PreviewFundingWindowEvidence {
    pub yield_basis: shared_types::YieldBasis,
    pub buffer_bps: f64,
    pub long_next_settlement_ms: i64,
    pub short_next_settlement_ms: i64,
}

#[derive(Clone, Copy, PartialEq)]
pub(in crate::panels::modules::execution) struct PreviewOneCycleCost {
    pub gross_edge_bps: f64,
    pub total_cost_bps: f64,
    pub open_fee_bps: f64,
    pub close_fee_bps: f64,
    pub open_slippage_bps: f64,
    pub close_slippage_bps: f64,
    pub funding_window_mismatch_evidence: Option<PreviewFundingWindowEvidence>,
    pub profitability_status: shared_types::ProfitabilityEvidenceStatus,
    pub funding_history_health: Option<shared_types::FundingDiffSampleHealth>,
    pub funding_history_sample_count: usize,
    pub net_bps: f64,
    pub covers_round_trip_cost: bool,
}

impl PreviewOneCycleCost {
    pub(in crate::panels::modules::execution::data::preview) fn from_cost(
        cost: &ExecutionCostProfile,
    ) -> Self {
        let profitability = cost
            .round_trip
            .as_ref()
            .map(|round_trip| &round_trip.profitability_evidence);
        Self {
            gross_edge_bps: cost.one_cycle.gross_edge_bps,
            total_cost_bps: cost.total_cost_bps,
            open_fee_bps: cost.one_cycle.open_fee_bps,
            close_fee_bps: cost.one_cycle.close_fee_bps,
            open_slippage_bps: cost.one_cycle.open_slippage_bps,
            close_slippage_bps: cost.one_cycle.close_slippage_bps,
            funding_window_mismatch_evidence: match (
                cost.one_cycle.yield_basis,
                cost.one_cycle.long_next_settlement_ms,
                cost.one_cycle.short_next_settlement_ms,
            ) {
                (Some(yield_basis), Some(long), Some(short)) => {
                    Some(PreviewFundingWindowEvidence {
                        yield_basis,
                        buffer_bps: cost.one_cycle.funding_window_mismatch_buffer_bps,
                        long_next_settlement_ms: long,
                        short_next_settlement_ms: short,
                    })
                }
                _ => None,
            },
            profitability_status: profitability
                .map(|evidence| evidence.status)
                .unwrap_or_default(),
            funding_history_health: profitability
                .and_then(|evidence| evidence.funding_history.as_ref())
                .map(|history| history.sample_health),
            funding_history_sample_count: profitability
                .and_then(|evidence| evidence.funding_history.as_ref())
                .map_or(0, |history| history.sample_count),
            net_bps: cost.one_cycle.net_bps,
            covers_round_trip_cost: cost.one_cycle.covers_round_trip_cost,
        }
    }
}

#[derive(Clone, PartialEq)]
pub(in crate::panels::modules::execution) struct PreviewProfitEvidence {
    pub one_cycle_net_bps: f64,
    pub fee_evidence_ids: Vec<String>,
    pub fee_evidence_complete: bool,
}

impl PreviewProfitEvidence {
    pub(super) fn from_selection(selection: &ExecutionSelection) -> Self {
        Self {
            one_cycle_net_bps: selection.one_cycle_net_bps,
            fee_evidence_ids: selection.fee_evidence_ids.clone(),
            fee_evidence_complete: selection.fee_evidence_complete,
        }
    }

    pub(in crate::panels::modules::execution) fn has_evidence(&self) -> bool {
        self.one_cycle_net_bps.abs() > f64::EPSILON
            || !self.fee_evidence_ids.is_empty()
            || self.fee_evidence_complete
    }
}
