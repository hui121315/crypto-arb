use super::super::*;
use super::support::*;
use crate::api::rest::ApiError;
use crate::panels::modules::market_evidence::market_health_label;
use crate::panels::modules::opportunity_format::missing_quote_label;
use crate::state::load_state::LoadState;
use leptos::prelude::*;
use shared_types::MarketDataQuality;
use shared_types::{ApiProblem, RowCapEvidence};

#[test]
fn detail_state_keeps_partial_detail_with_problem() {
    let state = detail_state(
        detail_fixture("1", "MU"),
        Some(ApiProblem::new("RATE_LIMITED", "rate limited")),
    );

    assert!(matches!(state, LoadState::Stale { .. }));
    assert_eq!(
        state
            .value()
            .and_then(OpportunityDetailSnapshot::detail)
            .map(|detail| detail.pair.as_str()),
        Some("MU")
    );
    assert_eq!(
        state.problem().map(|problem| problem.code.as_str()),
        Some("RATE_LIMITED")
    );
}

#[test]
fn detail_market_health_label_reuses_coverage_formatter() {
    let label = market_health_label(&shared_types::MarketDataHealth {
        quality: shared_types::MarketDataQuality::Fresh,
        source: shared_types::MarketDataSourceKind::LocalCache,
        freshness_ms: Some(10),
        retry_after_ms: None,
        last_error: None,
        observed_at_ms: 1,
        coverage: Some(shared_types::MarketDataCoverage::new(2, 1)),
        problem: None,
    });

    assert_eq!(label, "新鲜 · 本地缓存 · 10ms · 覆盖 1/2 (50%)");
}

#[test]
fn detail_request_error_populates_all_section_evidence() -> Result<(), String> {
    let problem = ApiProblem::new("RATE_LIMITED", "rate limited")
        .with_request_id(Some("req-1".into()))
        .with_retry_after_ms(Some(2_000))
        .with_source("rest");

    let state = detail_from_request_error(seed("opp-1", "MU"), problem);
    let detail = state
        .value()
        .and_then(OpportunityDetailSnapshot::detail)
        .ok_or_else(|| "stale detail should be present".to_owned())?;

    assert_eq!(detail.section_evidence.len(), 7);
    assert_eq!(detail.section_evidence[0].section, "订单簿 多腿 · binance");
    assert_eq!(detail.section_evidence[2].section, "历史 · MU");
    assert_eq!(detail.section_evidence[5].section, "行情 多腿 · binance");
    for evidence in &detail.section_evidence {
        assert_eq!(evidence.request_id, "req-1");
        assert_eq!(evidence.retry_after, "2000ms");
        assert_eq!(evidence.source, "rest");
    }
    Ok(())
}

#[test]
fn detail_request_error_preserves_cost_evidence_state() -> Result<(), String> {
    let mut seed = seed("opp-1", "MU");
    seed.cost_verified = false;

    let state = detail_from_request_error(seed, ApiProblem::new("UPSTREAM", "upstream"));
    let detail = state
        .value()
        .and_then(OpportunityDetailSnapshot::detail)
        .ok_or_else(|| "stale detail should be present".to_owned())?;

    assert!(!detail.cost_verified);
    Ok(())
}

#[test]
fn orderbook_error_health_keeps_typed_problem_context() {
    let mut problem = ApiProblem::new("ORDERBOOK_UPSTREAM", "orderbook down")
        .with_source("rest-orderbook")
        .with_status(502)
        .with_request_id(Some("req-book-1".into()))
        .with_retry_after_ms(Some(2_500))
        .with_recovery_action(shared_types::ApiRecoveryAction::CheckRuntimeHealth);
    problem.details = Some(serde_json::json!({
        "venue": "binance",
        "operation": "rest_orderbook",
        "method": "GET",
        "path": "/fapi/v1/depth",
        "symbol": "MUUSDT",
        "upstreamCode": "-1003"
    }));
    let (book, evidence) = capture_book(
        Err(ApiError::from_problem(problem)),
        "多腿",
        "binance",
        Some("detail-request"),
    );

    assert!(book.health.contains("orderbook down"));
    assert!(book.health.contains("code ORDERBOOK_UPSTREAM"));
    assert!(book.health.contains("source rest-orderbook"));
    assert!(book.health.contains("HTTP 502"));
    assert!(book.health.contains("request_id req-book-1"));
    assert!(book.health.contains("retry 2500ms"));
    assert!(book.health.contains("venue binance"));
    assert!(book.health.contains("operation rest_orderbook"));
    assert!(book.health.contains("GET /fapi/v1/depth"));
    assert!(book.health.contains("symbol MUUSDT"));
    assert!(book.health.contains("upstream_code -1003"));
    assert!(book.health.contains("下一步 检查运行状态"));
    assert_eq!(evidence.source, "rest-orderbook");
    assert_eq!(evidence.request_id, "req-book-1");
    assert_eq!(evidence.retry_after, "2500ms");
}

#[test]
fn leg_market_evidence_uses_dto_health_and_flags_missing_evidence() {
    let evidence = shared_types::OpportunityLegMarketEvidence {
        venue: "binance".into(),
        symbol: "MUUSDT".into(),
        price: Some(100.0),
        health: market_health(MarketDataQuality::Fresh),
    };

    let present = leg_market_evidence("多腿", "binance", Some(&evidence), Some("req-9"));
    assert_eq!(present.section, "行情 多腿 · binance");
    assert_eq!(present.source, "本地缓存");
    assert_eq!(present.freshness, "10ms");
    assert_eq!(present.request_id, "req-9");

    let missing = leg_market_evidence("空腿", "okx", None, None);
    assert_eq!(missing.section, "行情 空腿 · okx");
    assert_eq!(missing.source, "缺证据");
    assert_eq!(missing.freshness, "未知");
}

#[test]
fn market_section_evidence_prefers_segment_problem_request_id() {
    let mut health = market_health(MarketDataQuality::RateLimited);
    health.problem = Some(
        ApiProblem::new("RATE_LIMITED", "rate limited")
            .with_request_id(Some("segment-req".into()))
            .with_retry_after_ms(Some(1_000)),
    );

    let evidence =
        market_section_evidence("订单簿 多腿 · binance".into(), &health, Some("detail-req"));

    assert_eq!(evidence.source, "本地缓存");
    assert_eq!(evidence.freshness, "10ms");
    assert_eq!(evidence.request_id, "segment-req");
    assert_eq!(evidence.retry_after, "1000ms");
}

#[test]
fn row_cap_label_marks_lower_bound_totals() {
    let label = row_cap_label(&RowCapEvidence::lower_bound(24, 24, 25, "history:memory"));

    assert!(label.contains("至少25条"));
    assert!(label.contains("已截断至少1条"));
    assert!(label.contains("history:memory"));
}

#[test]
fn price_hides_non_finite_or_empty_orderbook_values() {
    assert_eq!(price(None), missing_quote_label());
    assert_eq!(price(Some(0.0)), missing_quote_label());
    assert_eq!(price(Some(f64::NAN)), missing_quote_label());
    assert_eq!(price(Some(f64::INFINITY)), missing_quote_label());
    assert_eq!(price(Some(123.45678)), "123.4568");
}

#[test]
fn spread_hides_non_finite_orderbook_values() {
    assert_eq!(spread(None), "待价差");
    assert_eq!(spread(Some(f64::NAN)), "待价差");
    assert_eq!(spread(Some(f64::INFINITY)), "待价差");
    assert_eq!(spread(Some(-0.01)), "待价差");
    assert_eq!(spread(Some(0.0)), "0.0000");
    assert_eq!(spread(Some(0.12345)), "0.1235");
}
