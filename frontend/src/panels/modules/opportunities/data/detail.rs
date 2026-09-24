use super::*;
use crate::api::rest::{ApiError, HistoryResponse, OpportunityHistoryRow};
use crate::panels::modules::funding_stats::FundingCycleStatsView;
use crate::panels::modules::index_composition::{
    IndexCompositionDetailView, IndexCompositionSnapshotView, IndexCompositionView,
};
use crate::panels::modules::opportunity_format::missing_quote_label;
use crate::state::context::use_global;
use crate::state::load_state::LoadState;
use leptos::prelude::*;
use leptos::task::spawn_local;
use shared_types::{ApiProblem, IndexCompositionSnapshot, MarketDataEnvelope, OrderBookInfo};

#[derive(Clone, PartialEq)]
pub(crate) struct OpportunityDetail {
    pub id: String,
    pub pair: String,
    pub domain: String,
    pub market_scope: String,
    pub risk: String,
    pub net_edge: String,
    pub gross_one_cycle: String,
    pub round_trip_cost: String,
    pub cost_verified: bool,
    pub execution_eligible: bool,
    pub one_cycle_net: String,
    pub one_cycle_net_bps: f64,
    pub funding_stats: FundingCycleStatsView,
    pub index_composition: IndexCompositionView,
    pub index_composition_detail: IndexCompositionDetailView,
    pub reason: String,
    pub books: Vec<BookLine>,
    pub history_health: String,
    pub history: Vec<HistoryLine>,
    pub section_evidence: Vec<DetailEvidence>,
}

#[derive(Clone, PartialEq)]
pub(crate) enum OpportunityDetailSnapshot {
    Unselected,
    Selected(Box<OpportunityDetail>),
}

impl OpportunityDetailSnapshot {
    pub(crate) fn detail(&self) -> Option<&OpportunityDetail> {
        match self {
            Self::Selected(detail) => Some(detail.as_ref()),
            Self::Unselected => None,
        }
    }
}

pub(crate) type OpportunityDetailState = LoadState<OpportunityDetailSnapshot>;

#[derive(Clone, PartialEq)]
pub(crate) struct BookLine {
    pub venue: String,
    pub bid: String,
    pub ask: String,
    pub spread: String,
    pub health: String,
}

#[derive(Clone, PartialEq)]
pub(crate) struct HistoryLine {
    pub time: String,
    pub route: String,
    pub edge: String,
    pub health: String,
}

#[derive(Clone, PartialEq)]
pub(crate) struct DetailEvidence {
    pub section: String,
    pub source: String,
    pub freshness: String,
    pub request_id: String,
    pub retry_after: String,
    pub problem: Option<ApiProblem>,
}

#[derive(Clone, Copy)]
pub(in crate::panels::modules::opportunities) struct OpportunityDetailData {
    pub state: RwSignal<OpportunityDetailState>,
    pub loading: RwSignal<bool>,
    pub refresh: Callback<()>,
}

pub(in crate::panels::modules::opportunities) fn use_opportunity_detail(
    runtime: OpportunitiesRuntime,
) -> OpportunityDetailData {
    let client = use_global().client;
    let selected_detail = runtime.selected_detail;
    let detail_state = runtime.detail_state;
    let request_version = RwSignal::new(0_u64);
    let refresh_version = RwSignal::new(0_u64);
    let loading = RwSignal::new(false);
    let selected_id = Memo::new(move |_| selected_detail.with(|seed| seed.id.clone()));

    Effect::new(move |_| {
        let seed = selected_detail.get();
        detail_state.update(|state| refresh_detail_seed_fields(state, &seed));
    });

    Effect::new(move |_| {
        let client = client.clone();
        let request_id = selected_id.get();
        let _ = refresh_version.get();
        let request_token = next_detail_request_token(request_version);
        if request_id.is_empty() {
            loading.set(false);
            detail_state.set(LoadState::Ready(OpportunityDetailSnapshot::Unselected));
            return;
        }
        let seed = selected_detail.get_untracked();
        loading.set(true);
        if detail_state_id(&detail_state.get_untracked()).as_deref() != Some(request_id.as_str()) {
            detail_state.set(LoadState::Loading);
        }
        spawn_local(async move {
            let next = load_opportunity_detail(client, seed).await;
            let Some(current_token) = request_version.try_get_untracked() else {
                return;
            };
            let Some(current_seed) = selected_detail.try_get_untracked() else {
                return;
            };
            if detail_request_is_current(
                &current_seed.id,
                &request_id,
                current_token,
                request_token,
            ) {
                detail_state.update(|current| {
                    *current = merge_opportunity_detail_result(current, next);
                    refresh_detail_seed_fields(current, &current_seed);
                });
                loading.set(false);
            }
        });
    });
    OpportunityDetailData {
        state: detail_state,
        loading,
        refresh: Callback::new(move |_| {
            if !loading.get_untracked() && !selected_id.get_untracked().is_empty() {
                refresh_version.update(|value| *value += 1);
            }
        }),
    }
}

fn refresh_detail_seed_fields(state: &mut OpportunityDetailState, seed: &OpportunityDetailSeed) {
    let Some(snapshot) = (match state {
        LoadState::Ready(snapshot)
        | LoadState::Stale {
            value: snapshot, ..
        } => Some(snapshot),
        LoadState::Loading | LoadState::Error(_) => None,
    }) else {
        return;
    };
    let OpportunityDetailSnapshot::Selected(detail) = snapshot else {
        return;
    };
    if detail.id != seed.id {
        return;
    }
    detail.pair.clone_from(&seed.pair);
    detail.domain.clone_from(&seed.domain);
    detail.market_scope.clone_from(&seed.market_scope);
    detail.risk.clone_from(&seed.risk);
    detail.net_edge.clone_from(&seed.net_edge);
    detail.gross_one_cycle.clone_from(&seed.gross_one_cycle);
    detail.round_trip_cost.clone_from(&seed.round_trip_cost);
    detail.cost_verified = seed.cost_verified;
    detail.execution_eligible = seed.execution_eligible;
    detail.one_cycle_net.clone_from(&seed.one_cycle_net);
    detail.one_cycle_net_bps = seed.one_cycle_net_bps;
    detail.reason.clone_from(&seed.reason);
}

fn next_detail_request_token(version: RwSignal<u64>) -> u64 {
    let token = version.get_untracked().wrapping_add(1);
    version.set(token);
    token
}

pub(in crate::panels::modules::opportunities) fn detail_request_is_current(
    current_id: &str,
    requested_id: &str,
    current_token: u64,
    request_token: u64,
) -> bool {
    current_id == requested_id && current_token == request_token
}

pub(in crate::panels::modules::opportunities) fn merge_opportunity_detail_result(
    current: &OpportunityDetailState,
    next: OpportunityDetailState,
) -> OpportunityDetailState {
    let Some(previous) = current
        .value()
        .and_then(OpportunityDetailSnapshot::detail)
        .cloned()
    else {
        return next;
    };
    match next {
        LoadState::Stale {
            value: OpportunityDetailSnapshot::Selected(mut detail),
            problem,
        } if detail.id == previous.id => {
            preserve_stale_detail(&previous, &mut detail);
            LoadState::Stale {
                value: OpportunityDetailSnapshot::Selected(detail),
                problem,
            }
        }
        LoadState::Error(problem) => LoadState::Stale {
            value: OpportunityDetailSnapshot::Selected(Box::new(previous)),
            problem,
        },
        state => state,
    }
}

fn preserve_stale_detail(previous: &OpportunityDetail, next: &mut OpportunityDetail) {
    for next_book in &mut next.books {
        if book_line_has_quotes(next_book) {
            continue;
        }
        let Some(previous_book) = previous
            .books
            .iter()
            .find(|book| book.venue == next_book.venue && book_line_has_quotes(book))
        else {
            continue;
        };
        next_book.bid.clone_from(&previous_book.bid);
        next_book.ask.clone_from(&previous_book.ask);
        next_book.spread.clone_from(&previous_book.spread);
    }
    if next.history.is_empty() && !previous.history.is_empty() {
        next.history.clone_from(&previous.history);
    }
}

fn book_line_has_quotes(book: &BookLine) -> bool {
    book.bid != missing_quote_label() || book.ask != missing_quote_label()
}

pub(in crate::panels::modules::opportunities) async fn load_opportunity_detail(
    client: crate::api::rest::ApiClient,
    seed: OpportunityDetailSeed,
) -> OpportunityDetailState {
    match client.opportunity_detail_read(&seed.id).await {
        Ok(response) => detail_from_response(seed, response),
        Err(error) => detail_from_request_error(seed, error.problem),
    }
}

/// Segment results returned by the single aggregated detail endpoint.
pub(in crate::panels::modules::opportunities) struct DetailSegments {
    pub(in crate::panels::modules::opportunities) long_book:
        Result<MarketDataEnvelope<Option<OrderBookInfo>>, ApiError>,
    pub(in crate::panels::modules::opportunities) short_book:
        Result<MarketDataEnvelope<Option<OrderBookInfo>>, ApiError>,
    pub(in crate::panels::modules::opportunities) history:
        Result<HistoryResponse<OpportunityHistoryRow>, ApiError>,
    pub(in crate::panels::modules::opportunities) long_index:
        Result<MarketDataEnvelope<Option<IndexCompositionSnapshot>>, ApiError>,
    pub(in crate::panels::modules::opportunities) short_index:
        Result<MarketDataEnvelope<Option<IndexCompositionSnapshot>>, ApiError>,
}

pub(in crate::panels::modules::opportunities) struct DetailSections {
    pub(in crate::panels::modules::opportunities) books: Vec<BookLine>,
    pub(in crate::panels::modules::opportunities) history: Vec<HistoryLine>,
    pub(in crate::panels::modules::opportunities) history_health: String,
    pub(in crate::panels::modules::opportunities) long_index: IndexCompositionSnapshotView,
    pub(in crate::panels::modules::opportunities) short_index: IndexCompositionSnapshotView,
    pub(in crate::panels::modules::opportunities) section_evidence: Vec<DetailEvidence>,
}

pub(in crate::panels::modules::opportunities) fn detail_state_id(
    state: &OpportunityDetailState,
) -> Option<String> {
    state
        .value()
        .and_then(OpportunityDetailSnapshot::detail)
        .map(|detail| detail.id.clone())
}

pub(in crate::panels::modules::opportunities) fn detail_state(
    detail: OpportunityDetail,
    problem: Option<ApiProblem>,
) -> OpportunityDetailState {
    match problem {
        Some(problem) => LoadState::Stale {
            value: OpportunityDetailSnapshot::Selected(Box::new(detail)),
            problem,
        },
        None => LoadState::Ready(OpportunityDetailSnapshot::Selected(Box::new(detail))),
    }
}
