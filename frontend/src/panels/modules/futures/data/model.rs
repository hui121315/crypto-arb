use crate::panels::modules::cost_copy::fee_evidence_label;
use crate::panels::modules::execution::ExecutionSelectionSeed;
use crate::panels::modules::funding_stats::FundingCycleStatsView;
use crate::panels::modules::index_composition::IndexCompositionView;
use crate::panels::modules::opportunity_view_model::{
    OpportunityListViewModel, OpportunityListViewRow,
};
use shared_types::SpotLegMode;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

/// futures 行投影：列表共享字段全部复用 `OpportunityListViewRow`（经 `Deref` 透出），
/// 本结构只保留 futures 专属的展示扩展字段。
#[derive(Clone, PartialEq)]
pub(in crate::panels::modules::futures) struct FuturesOpportunity {
    pub view: OpportunityListViewRow,
    pub funding_curve: Vec<f64>,
    pub funding_stats: FundingCycleStatsView,
    pub index_composition: IndexCompositionView,
    pub borrow_cost_bps_per_day: Option<f64>,
    pub funding_alignment_minutes: Option<i32>,
    pub funding_cap_distance_bps: Option<f64>,
    pub min_hold_hours: Option<f64>,
}

impl Deref for FuturesOpportunity {
    type Target = OpportunityListViewModel;

    fn deref(&self) -> &Self::Target {
        &self.view
    }
}

impl DerefMut for FuturesOpportunity {
    fn deref_mut(&mut self) -> &mut Self::Target {
        Arc::make_mut(&mut self.view)
    }
}

impl FuturesOpportunity {
    pub(in crate::panels::modules::futures) fn settlement_minutes(&self) -> Option<i64> {
        self.settlement_countdown_seconds
            .and_then(positive_seconds_to_minutes)
            .or_else(|| {
                (self.time_to_settlement_ms > 0)
                    .then_some(self.time_to_settlement_ms.saturating_add(59_999) / 60_000)
            })
    }

    pub(in crate::panels::modules::futures) fn execution_seed(&self) -> ExecutionSelectionSeed {
        ExecutionSelectionSeed::from_futures(self.view.as_ref())
    }

    pub(in crate::panels::modules::futures) fn gross_one_cycle_text(&self) -> String {
        self.gross_one_cycle.clone()
    }

    pub(in crate::panels::modules::futures) fn round_trip_cost_text(&self) -> String {
        self.round_trip_cost.clone()
    }

    pub(in crate::panels::modules::futures) fn cost_evidence_label(&self) -> String {
        fee_evidence_label(self.fee_evidence_count, self.fee_evidence_complete)
    }

    pub(in crate::panels::modules::futures) fn one_cycle_net_text(&self) -> String {
        self.one_cycle_net.clone()
    }

    pub(in crate::panels::modules::futures) fn spot_leg_mode_label(&self) -> Option<&'static str> {
        self.spot_leg_mode.map(SpotLegMode::label_zh)
    }
}

fn positive_seconds_to_minutes(seconds: i64) -> Option<i64> {
    (seconds > 0).then_some(seconds.saturating_add(59) / 60)
}
