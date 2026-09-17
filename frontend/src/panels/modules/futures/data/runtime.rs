use crate::panels::modules::instrument_search::symbol_search_query;
use crate::panels::modules::opportunity_counts::OpportunityCountMeta;
use crate::panels::modules::opportunity_runtime::{
    OpportunityListRuntime as SharedOpportunityListRuntime,
    OpportunitySearchRuntime as SharedOpportunitySearchRuntime, CANONICAL_OPPORTUNITY_PAGE_SIZE,
    CANONICAL_OPPORTUNITY_SEARCH_DEBOUNCE,
};
use crate::panels::modules::opportunity_view_model::OpportunityListViewRow;
use crate::panels::routing::{WorkspaceRoute, WorkspaceRouteOrigin};
use crate::panels::workstation::ModuleId;
use crate::state::load_state::LoadState;
use crate::state::module_runtime::ModuleRuntimeState;
use leptos::prelude::*;
use shared_types::{OpportunityListPage, StrategyKind};
use std::sync::Arc;
use std::time::Duration;

use super::*;

pub(in crate::panels::modules::futures) const SEARCH_DEBOUNCE: Duration =
    CANONICAL_OPPORTUNITY_SEARCH_DEBOUNCE;
pub(in crate::panels::modules::futures) const FUTURES_PAGE_SIZE: usize =
    CANONICAL_OPPORTUNITY_PAGE_SIZE;

pub(in crate::panels::modules::futures) type FuturesOpportunityRow = Arc<FuturesOpportunity>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::panels::modules::futures) enum StrategyFilter {
    PerpCross,
    PerpPriceSpread,
    SpotPerp,
    CrossSpotPerp,
    SpotCross,
}

#[derive(Clone, PartialEq)]
pub(in crate::panels::modules::futures) struct FuturesFilter {
    pub strategy: StrategyFilter,
    pub min_net_pct: f64,
    pub query: String,
}

#[derive(Clone, PartialEq)]
pub(in crate::panels::modules::futures) struct FuturesSummary {
    pub candidates: usize,
    pub filtered_candidates: usize,
    pub executable_candidates: usize,
    pub best_monitor_net_bps: Option<f64>,
    pub best_monitor_pair: String,
    pub best_monitor_profit_detail: String,
    pub best_monitor_preview_ready: bool,
}

#[derive(Clone)]
pub(in crate::panels::modules::futures) struct FuturesOpportunityStore {
    pub(in crate::panels::modules::futures) state: RwSignal<LoadState<()>>,
    pub(in crate::panels::modules::futures) rows: Memo<Vec<FuturesOpportunityRow>>,
    pub(in crate::panels::modules::futures) meta: RwSignal<OpportunityCountMeta>,
    pub(in crate::panels::modules::futures) page: RwSignal<Option<OpportunityListPage>>,
    pub(in crate::panels::modules::futures) loading: RwSignal<bool>,
    pub(in crate::panels::modules::futures) load_cursor: Callback<Option<String>>,
}

#[derive(Clone, Copy)]
pub(in crate::panels) struct FuturesRuntime {
    pub(in crate::panels::modules::futures) filter: RwSignal<FuturesFilter>,
    pub(in crate::panels::modules::futures) position_entry_symbol: RwSignal<Option<String>>,
    pub(in crate::panels::modules::futures) list: FuturesListRuntime,
    pub(in crate::panels::modules::futures) search: FuturesSearchRuntime,
}

pub(in crate::panels::modules::futures) type FuturesListRuntime =
    SharedOpportunityListRuntime<OpportunityListViewRow>;
pub(in crate::panels::modules::futures) type FuturesSearchRuntime =
    SharedOpportunitySearchRuntime<OpportunityListViewRow>;

pub(in crate::panels) fn create_futures_runtime(
    opportunities: crate::panels::modules::opportunities::OpportunitiesRuntime,
) -> FuturesRuntime {
    FuturesRuntime {
        filter: RwSignal::new(FuturesFilter::default()),
        position_entry_symbol: RwSignal::new(None),
        list: opportunities.list,
        search: opportunities.search,
    }
}

impl Default for FuturesFilter {
    fn default() -> Self {
        Self {
            strategy: StrategyFilter::PerpCross,
            min_net_pct: 0.0,
            query: String::new(),
        }
    }
}

impl StrategyFilter {
    pub(in crate::panels::modules::futures) const fn from_kind(kind: StrategyKind) -> Option<Self> {
        match kind {
            StrategyKind::PerpCross => Some(Self::PerpCross),
            StrategyKind::PerpPriceSpread => Some(Self::PerpPriceSpread),
            StrategyKind::SpotPerp => Some(Self::SpotPerp),
            StrategyKind::CrossSpotPerp => Some(Self::CrossSpotPerp),
            StrategyKind::SpotCross => Some(Self::SpotCross),
            _ => None,
        }
    }

    pub(in crate::panels::modules::futures) const fn label(self) -> &'static str {
        self.kind().label_zh()
    }

    pub(in crate::panels::modules::futures) const fn filter_metric_label(self) -> &'static str {
        "本页最低费后边际"
    }

    pub(in crate::panels::modules::futures) fn matches(self, row: &FuturesOpportunity) -> bool {
        row.strategy_kind == Some(self.kind())
    }

    pub(in crate::panels::modules::futures) const fn kind(self) -> StrategyKind {
        match self {
            Self::PerpCross => StrategyKind::PerpCross,
            Self::PerpPriceSpread => StrategyKind::PerpPriceSpread,
            Self::SpotPerp => StrategyKind::SpotPerp,
            Self::CrossSpotPerp => StrategyKind::CrossSpotPerp,
            Self::SpotCross => StrategyKind::SpotCross,
        }
    }
}

impl FuturesRuntime {
    pub(in crate::panels) fn apply_workspace_route(self, route: &WorkspaceRoute) {
        if route.module != ModuleId::Futures {
            return;
        }
        self.position_entry_symbol.set(
            (route.origin == Some(WorkspaceRouteOrigin::Position))
                .then(|| route.symbol.clone())
                .flatten(),
        );
        self.filter.update(|filter| {
            if let Some(symbol) = route.symbol.as_ref() {
                filter.query.clone_from(symbol);
            }
            if let Some(strategy) = route.strategy.and_then(StrategyFilter::from_kind) {
                filter.strategy = strategy;
            }
        });
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
        ModuleRuntimeState::combine(states)
    }
}
