use super::*;
use crate::panels::modules::funding_stats::FundingCycleStatsView;
use crate::panels::modules::index_composition::{
    IndexCompositionDetailView, IndexCompositionSnapshotView, IndexCompositionView,
};
use crate::panels::modules::market_evidence::retry_label;
use shared_types::ApiProblem;

pub(in crate::panels::modules::opportunities) fn detail_from_response(
    mut seed: OpportunityDetailSeed,
    response: crate::api::rest::OpportunityDetailResponse,
) -> OpportunityDetailState {
    let mut problem = None;
    seed_response_problem(&response, &mut problem);
    apply_authoritative_opportunity_semantics(&mut seed, &response.opportunity);
    let symbol = response.opportunity.symbol.clone();
    let long_venue = response.opportunity.long_exchange.clone();
    let short_venue = response.opportunity.short_exchange.clone();
    let detail_request_id = response.request_id.clone();
    let segments = DetailSegments {
        long_book: Ok(response.long_orderbook),
        short_book: Ok(response.short_orderbook),
        history: Ok(response.history),
        long_index: Ok(response.long_index_composition),
        short_index: Ok(response.short_index_composition),
    };
    let mut sections = capture_detail_sections(
        segments,
        &symbol,
        &long_venue,
        &short_venue,
        detail_request_id.as_deref(),
        &mut problem,
    );
    sections.section_evidence.push(leg_market_evidence(
        "多腿",
        &long_venue,
        response.opportunity.long_leg_market_evidence.as_ref(),
        detail_request_id.as_deref(),
    ));
    sections.section_evidence.push(leg_market_evidence(
        "空腿",
        &short_venue,
        response.opportunity.short_leg_market_evidence.as_ref(),
        detail_request_id.as_deref(),
    ));
    assemble_detail(seed, sections, problem)
}

fn apply_authoritative_opportunity_semantics(
    seed: &mut OpportunityDetailSeed,
    opportunity: &shared_types::ArbitrageOpportunityDto,
) {
    seed.funding_stats = FundingCycleStatsView::from_dto(opportunity);
    seed.index_composition =
        IndexCompositionView::from_profile(opportunity.index_composition.as_ref());
}

pub(in crate::panels::modules::opportunities) fn capture_detail_sections(
    segments: DetailSegments,
    symbol: &str,
    long_venue: &str,
    short_venue: &str,
    request_id: Option<&str>,
    problem: &mut Option<ApiProblem>,
) -> DetailSections {
    let (long_book, long_book_evidence) =
        capture_book(segments.long_book, "多腿", long_venue, request_id);
    let (short_book, short_book_evidence) =
        capture_book(segments.short_book, "空腿", short_venue, request_id);
    let (history, history_health, history_evidence) =
        capture_history(segments.history, symbol, request_id, problem);
    let (long_index, long_index_evidence) = capture_index(
        segments.long_index,
        "多腿",
        long_venue,
        symbol,
        request_id,
        problem,
    );
    let (short_index, short_index_evidence) = capture_index(
        segments.short_index,
        "空腿",
        short_venue,
        symbol,
        request_id,
        problem,
    );
    DetailSections {
        books: vec![long_book, short_book],
        history,
        history_health,
        long_index,
        short_index,
        section_evidence: vec![
            long_book_evidence,
            short_book_evidence,
            history_evidence,
            long_index_evidence,
            short_index_evidence,
        ],
    }
}

pub(in crate::panels::modules::opportunities) fn assemble_detail(
    seed: OpportunityDetailSeed,
    sections: DetailSections,
    problem: Option<ApiProblem>,
) -> OpportunityDetailState {
    let index_detail = IndexCompositionDetailView::from_views(
        Some(sections.long_index),
        Some(sections.short_index),
    );
    detail_state(
        OpportunityDetail {
            id: seed.id,
            pair: seed.pair,
            domain: seed.domain,
            market_scope: seed.market_scope,
            risk: seed.risk,
            net_edge: seed.net_edge,
            gross_one_cycle: seed.gross_one_cycle,
            round_trip_cost: seed.round_trip_cost,
            cost_verified: seed.cost_verified,
            execution_eligible: seed.execution_eligible,
            one_cycle_net: seed.one_cycle_net,
            one_cycle_net_bps: seed.one_cycle_net_bps,
            funding_stats: seed.funding_stats,
            index_composition: seed.index_composition,
            index_composition_detail: index_detail,
            reason: seed.reason,
            books: sections.books,
            history_health: sections.history_health,
            history: sections.history,
            section_evidence: sections.section_evidence,
        },
        problem,
    )
}

pub(in crate::panels::modules::opportunities) fn seed_response_problem(
    response: &crate::api::rest::OpportunityDetailResponse,
    problem: &mut Option<ApiProblem>,
) {
    if let Some(error) = response.error.clone() {
        problem.get_or_insert(error);
    }
    if let Some(error) = response.partial_failures.first().cloned() {
        problem.get_or_insert(error);
    }
}

pub(in crate::panels::modules::opportunities) fn detail_from_request_error(
    seed: OpportunityDetailSeed,
    problem: ApiProblem,
) -> OpportunityDetailState {
    let pair = seed.pair.clone();
    let long_venue = seed.long_venue.clone();
    let short_venue = seed.short_venue.clone();
    let problem_message = problem.message.clone();
    let message = retry_label(
        &format!("详情错误 · {problem_message}"),
        problem.retry_after_ms,
    );
    let section_evidence = request_error_evidence(&pair, &long_venue, &short_venue, &problem);
    let long_index = IndexCompositionSnapshotView::unavailable(
        "多腿",
        &long_venue,
        &pair,
        message.clone(),
        problem_message.clone(),
    );
    let short_index = IndexCompositionSnapshotView::unavailable(
        "空腿",
        &short_venue,
        &pair,
        message.clone(),
        problem_message,
    );
    let detail = OpportunityDetail {
        id: seed.id,
        pair: seed.pair,
        domain: seed.domain,
        market_scope: seed.market_scope,
        risk: seed.risk,
        net_edge: seed.net_edge,
        gross_one_cycle: seed.gross_one_cycle,
        round_trip_cost: seed.round_trip_cost,
        cost_verified: seed.cost_verified,
        execution_eligible: seed.execution_eligible,
        one_cycle_net: seed.one_cycle_net,
        one_cycle_net_bps: seed.one_cycle_net_bps,
        funding_stats: seed.funding_stats,
        index_composition: seed.index_composition,
        index_composition_detail: IndexCompositionDetailView::from_views(
            Some(long_index),
            Some(short_index),
        ),
        reason: seed.reason,
        books: vec![
            empty_book_line(&long_venue, message.clone()),
            empty_book_line(&short_venue, message.clone()),
        ],
        history_health: message,
        history: Vec::new(),
        section_evidence,
    };
    detail_state(detail, Some(problem))
}
