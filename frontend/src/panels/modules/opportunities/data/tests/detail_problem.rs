use super::super::*;
use super::support::*;
use crate::api::rest::ApiError;
use crate::state::load_state::LoadState;
use shared_types::{ApiProblem, MarketDataQuality};

fn typed_problem(code: &str, message: &str, request_id: &str) -> ApiProblem {
    ApiProblem::new(code, message)
        .with_source("segment-rest")
        .with_status(502)
        .with_request_id(Some(request_id.into()))
        .with_retry_after_ms(Some(2_500))
}

fn assert_typed_context(label: &str, code: &str, request_id: &str) {
    assert!(label.contains(code));
    assert!(label.contains("source segment-rest"));
    assert!(label.contains("HTTP 502"));
    assert!(label.contains(&format!("request_id {request_id}")));
    assert!(label.contains("retry 2500ms"));
}

#[test]
fn orderbook_success_with_problem_keeps_segment_context() {
    let mut envelope = orderbook_envelope(Some(orderbook("binance")));
    envelope.health.quality = MarketDataQuality::RateLimited;
    envelope.health.problem = Some(typed_problem(
        "ORDERBOOK_DEGRADED",
        "cached orderbook",
        "req-book-degraded",
    ));
    let (book, evidence) = capture_book(Ok(envelope), "多腿", "binance", Some("detail-request"));

    assert_typed_context(&book.health, "ORDERBOOK_DEGRADED", "req-book-degraded");
    assert_eq!(evidence.request_id, "req-book-degraded");
    assert_eq!(evidence.retry_after, "2500ms");
    assert_eq!(
        evidence
            .problem
            .as_ref()
            .map(|problem| (problem.code.as_str(), problem.status)),
        Some(("ORDERBOOK_DEGRADED", Some(502)))
    );
}

#[test]
fn missing_orderbook_synthesizes_visible_typed_problem() {
    let (book, _) = capture_book(Ok(orderbook_envelope(None)), "空腿", "okx", None);

    assert!(book.health.contains("code MARKET_DATA_MISSING"));
    assert!(book.health.contains("source opportunity-detail"));
}

#[test]
fn deferred_orderbook_waits_for_build_without_reporting_an_error() {
    let mut envelope = orderbook_envelope(None);
    envelope.health.quality = MarketDataQuality::Unverified;
    envelope.health.source = shared_types::MarketDataSourceKind::LocalCache;
    envelope.health.freshness_ms = None;
    envelope.health.coverage = Some(shared_types::MarketDataCoverage::new(0, 0));
    envelope.health.problem = None;

    let (book, evidence) = capture_book(Ok(envelope), "多腿", "binance", Some("req-detail"));

    assert_eq!(book.health, "构建时核验 · 未主动读取盘口");
    assert_eq!(evidence.request_id, "req-detail");
    assert!(evidence.problem.is_none());
}

#[test]
fn history_and_index_errors_keep_independent_typed_context() {
    let mut captured_problem = None;
    let (_, history_health, history_evidence) = capture_history(
        Err(ApiError::from_problem(typed_problem(
            "HISTORY_UPSTREAM",
            "history down",
            "req-history-1",
        ))),
        "MU",
        Some("detail-request"),
        &mut captured_problem,
    );
    assert_typed_context(&history_health, "HISTORY_UPSTREAM", "req-history-1");
    assert_eq!(history_evidence.request_id, "req-history-1");

    let (index, index_evidence) = capture_index(
        Err(ApiError::from_problem(typed_problem(
            "INDEX_UPSTREAM",
            "index down",
            "req-index-1",
        ))),
        "空腿",
        "okx",
        "MU",
        Some("detail-request"),
        &mut captured_problem,
    );
    assert_typed_context(&index.health, "INDEX_UPSTREAM", "req-index-1");
    assert_eq!(index_evidence.request_id, "req-index-1");
}

#[test]
fn stale_detail_result_preserves_previous_orderbook_quotes() -> Result<(), String> {
    let mut previous = detail_fixture("opp-1", "MU");
    previous.books = vec![BookLine {
        venue: "binance".into(),
        bid: "100.0000".into(),
        ask: "101.0000".into(),
        spread: "1.0000".into(),
        health: "新鲜".into(),
    }];
    let mut degraded = detail_fixture("opp-1", "MU");
    degraded.books = vec![empty_book_line("binance", "错误 · upstream".into())];
    let next = LoadState::Stale {
        value: OpportunityDetailSnapshot::Selected(Box::new(degraded)),
        problem: typed_problem("ORDERBOOK_UPSTREAM", "upstream", "req-book"),
    };

    let merged = merge_opportunity_detail_result(
        &LoadState::Ready(OpportunityDetailSnapshot::Selected(Box::new(previous))),
        next,
    );
    let detail = merged
        .value()
        .and_then(OpportunityDetailSnapshot::detail)
        .ok_or_else(|| "merged stale detail should remain available".to_owned())?;

    assert!(matches!(merged, LoadState::Stale { .. }));
    assert_eq!(detail.books[0].bid, "100.0000");
    assert_eq!(detail.books[0].ask, "101.0000");
    assert_eq!(detail.books[0].spread, "1.0000");
    assert!(detail.books[0].health.contains("upstream"));
    Ok(())
}

#[test]
fn same_opportunity_late_request_token_is_rejected() {
    assert!(detail_request_is_current("opp-1", "opp-1", 3, 3));
    assert!(!detail_request_is_current("opp-1", "opp-1", 3, 2));
    assert!(!detail_request_is_current("opp-2", "opp-1", 3, 3));
}
