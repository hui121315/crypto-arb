use crate::panels::modules::instrument_search::symbol_search_query;
use crate::panels::modules::opportunity_runtime::{
    clean_opportunity_cursor, OpportunityListRuntime as SharedOpportunityListRuntime,
    OpportunitySearchRuntime as SharedOpportunitySearchRuntime,
    OpportunityStore as SharedOpportunityStore, CANONICAL_OPPORTUNITY_PAGE_SIZE,
    CANONICAL_OPPORTUNITY_SEARCH_DEBOUNCE,
};
use crate::panels::modules::opportunity_view_model::OpportunityListViewRow;
use crate::panels::routing::WorkspaceRoute;
use crate::panels::workstation::ModuleId;
use crate::state::load_state::LoadState;
use crate::state::module_runtime::ModuleRuntimeState;
use leptos::prelude::*;
use shared_types::StrategyKind;
use std::time::Duration;

use super::detail::{OpportunityDetailSnapshot, OpportunityDetailState};
use super::detail_seed::OpportunityDetailSeed;

pub(in crate::panels::modules::opportunities) const SEARCH_DEBOUNCE: Duration =
    CANONICAL_OPPORTUNITY_SEARCH_DEBOUNCE;
pub(in crate::panels::modules::opportunities) const OPPORTUNITY_PAGE_SIZE: usize =
    CANONICAL_OPPORTUNITY_PAGE_SIZE;

pub(crate) type OpportunityRow = OpportunityListViewRow;

#[derive(Clone, Default, PartialEq)]
pub(in crate::panels::modules::opportunities) struct OpportunityFilter {
    pub strategy: Option<StrategyKind>,
    pub min_net_pct: f64,
    pub query: String,
}

#[derive(Clone, PartialEq)]
pub(in crate::panels::modules::opportunities) struct OpportunitySummary {
    pub candidates: usize,
    pub filtered_candidates: usize,
    pub executable_candidates: usize,
    pub best_executable_net_bps: Option<f64>,
    pub best_executable_pair: String,
}

pub(in crate::panels::modules::opportunities) type OpportunityStore =
    SharedOpportunityStore<OpportunityRow>;

#[derive(Clone, Copy)]
pub(in crate::panels) struct OpportunitiesRuntime {
    pub(in crate::panels::modules::opportunities) filter: RwSignal<OpportunityFilter>,
    pub(in crate::panels::modules::opportunities) selected_idx: RwSignal<usize>,
    pub(in crate::panels::modules::opportunities) selected_opp_id: RwSignal<String>,
    pub(in crate::panels::modules::opportunities) selected_detail: RwSignal<OpportunityDetailSeed>,
    pub(in crate::panels::modules::opportunities) detail_state: RwSignal<OpportunityDetailState>,
    pub(crate) list: OpportunityListRuntime,
    pub(crate) search: OpportunitySearchRuntime,
}

pub(crate) type OpportunityListRuntime = SharedOpportunityListRuntime<OpportunityRow>;
pub(crate) type OpportunitySearchRuntime = SharedOpportunitySearchRuntime<OpportunityRow>;

pub(in crate::panels) fn create_opportunities_runtime() -> OpportunitiesRuntime {
    OpportunitiesRuntime {
        filter: RwSignal::new(OpportunityFilter::default()),
        selected_idx: RwSignal::new(0),
        selected_opp_id: RwSignal::new(String::new()),
        selected_detail: RwSignal::new(OpportunityDetailSeed::empty()),
        detail_state: RwSignal::new(LoadState::Ready(OpportunityDetailSnapshot::Unselected)),
        list: OpportunityListRuntime::new(),
        search: OpportunitySearchRuntime::new(),
    }
}

impl OpportunitiesRuntime {
    pub(in crate::panels) fn apply_workspace_route(self, route: &WorkspaceRoute) {
        if route.module != ModuleId::Opportunities {
            return;
        }
        self.filter.update(|filter| {
            if let Some(symbol) = route.symbol.as_ref() {
                filter.query.clone_from(symbol);
            }
            if let Some(strategy) = route.strategy {
                filter.strategy = Some(strategy);
            }
        });
        if let Some(opportunity_id) = route.opportunity_id.as_ref() {
            self.selected_opp_id.set(opportunity_id.clone());
        }
        if let Some(cursor) = route.page.as_ref() {
            if route.symbol.is_some() {
                if let Some(query) = route.symbol.as_deref().and_then(symbol_search_query) {
                    self.search.last_query.set(query);
                }
                self.search.cursor.set(Some(cursor.clone()));
            } else {
                self.list.cursor.set(Some(cursor.clone()));
            }
        }
    }

    pub(in crate::panels) fn module_runtime_state(self) -> ModuleRuntimeState {
        let mut states = vec![self.list.state.with(ModuleRuntimeState::from_load_state)];
        if !self.filter.with(|filter| filter.query.trim().is_empty()) {
            states.push(self.search.state.with(ModuleRuntimeState::from_load_state));
        }
        if !self.selected_opp_id.with(|id| id.trim().is_empty()) {
            states.push(self.detail_state.with(ModuleRuntimeState::from_load_state));
        }
        ModuleRuntimeState::combine(states)
    }
}

pub(in crate::panels::modules::opportunities) fn clean_cursor(
    cursor: Option<String>,
) -> Option<String> {
    clean_opportunity_cursor(cursor)
}
