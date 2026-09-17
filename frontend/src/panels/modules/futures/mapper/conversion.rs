use crate::panels::modules::funding_stats::FundingCycleStatsView;
use crate::panels::modules::futures::data::{FuturesOpportunity, FuturesOpportunityRow};
use crate::panels::modules::index_composition::IndexCompositionView;
use crate::panels::modules::opportunity_view_model::OpportunityListViewRow;
use std::sync::Arc;

pub(in crate::panels::modules::futures) fn to_futures_opps_from_list_views(
    rows: Vec<OpportunityListViewRow>,
) -> Vec<FuturesOpportunityRow> {
    rows.into_iter()
        .map(|row| Arc::new(FuturesOpportunity::from_list_view(&row)))
        .collect()
}

impl FuturesOpportunity {
    fn from_list_view(row: &OpportunityListViewRow) -> Self {
        Self {
            view: Arc::clone(row),
            funding_curve: funding_curve(),
            funding_stats: FundingCycleStatsView::default(),
            index_composition: IndexCompositionView::default(),
            borrow_cost_bps_per_day: None,
            funding_alignment_minutes: None,
            funding_cap_distance_bps: None,
            min_hold_hours: None,
        }
    }
}

pub(in crate::panels::modules::futures) fn funding_curve() -> Vec<f64> {
    Vec::new()
}
