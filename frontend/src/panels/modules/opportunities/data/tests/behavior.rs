use super::super::*;
use super::support::*;
use crate::api::rest::ApiError;
use crate::panels::modules::opportunity_counts::OpportunityCountMeta;
use crate::state::load_state::LoadState;
use leptos::prelude::Owner;
use leptos::prelude::*;
use shared_types::ApiProblem;
use std::sync::Arc;

#[test]
fn symbol_search_matches_exact_base_symbol() {
    let rows = vec![
        row_ref("1", "MU", "hyperliquid:xyz", true),
        row_ref("2", "MUBARAK", "kucoin", true),
    ];
    let filter = OpportunityFilter {
        query: "mu".into(),
        ..OpportunityFilter::default()
    };

    let visible = filter_rows(&rows, &filter);

    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].pair, "MU");
}

#[test]
fn venue_search_remains_text_match() {
    let rows = vec![
        row_ref("1", "MU", "hyperliquid:xyz", true),
        row_ref("2", "BTC", "kucoin", true),
    ];
    let filter = OpportunityFilter {
        query: "hyperliquid:xyz".into(),
        ..OpportunityFilter::default()
    };

    let visible = filter_rows(&rows, &filter);

    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].long_venue, "hyperliquid:xyz");
}

#[test]
fn search_matches_spot_leg_mode_label() {
    let rows = vec![
        row_ref_with_spot_leg_mode(
            "spot-mode",
            "1",
            "MU",
            "binance",
            shared_types::SpotLegMode::BorrowAndSell,
        ),
        row_ref("2", "BTC", "kucoin", true),
    ];
    let filter = OpportunityFilter {
        query: "借币".into(),
        ..OpportunityFilter::default()
    };

    let visible = filter_rows(&rows, &filter);

    assert_eq!(visible.len(), 1);
    assert_eq!(
        visible[0].spot_leg_mode,
        Some(shared_types::SpotLegMode::BorrowAndSell)
    );
}

#[test]
fn filter_rows_reuses_opportunity_row_handles() {
    let keep = row_ref("1", "MU", "hyperliquid:xyz", true);
    let rows = vec![Arc::clone(&keep), row_ref("2", "BTC", "kucoin", true)];
    let filter = OpportunityFilter {
        query: "mu".into(),
        ..OpportunityFilter::default()
    };

    let visible = filter_rows(&rows, &filter);

    assert_eq!(visible.len(), 1);
    assert!(Arc::ptr_eq(&visible[0], &keep));
}

#[test]
fn summary_counts_filtered_and_executable_rows() {
    let rows = vec![
        row_ref("1", "MU", "gate", true),
        row_ref("2", "BTC", "kucoin", false),
    ];
    let filter = OpportunityFilter {
        min_net_pct: 0.01,
        ..OpportunityFilter::default()
    };

    let summary = summarize_rows(&rows, &filter, &OpportunityCountMeta::default());

    assert_eq!(summary.candidates, 2);
    assert_eq!(summary.filtered_candidates, 2);
    assert_eq!(summary.executable_candidates, 1);
    assert_eq!(summary.best_executable_pair, "MU");
}

#[test]
fn summary_never_promotes_observation_profit_as_executable_profit() {
    let rows = vec![
        row_ref_with_net("ready", "1", "MU", "gate", true, 8.0),
        row_ref_with_net("observe", "2", "VELODROME", "kucoin", false, 1_900.0),
    ];

    let summary = summarize_rows(
        &rows,
        &OpportunityFilter::default(),
        &OpportunityCountMeta::default(),
    );

    assert_eq!(summary.best_executable_net_bps, Some(8.0));
    assert_eq!(summary.best_executable_pair, "MU");
}

#[test]
fn summary_uses_backend_total_before_visible_limit() {
    let rows = vec![
        row_ref("1", "MU", "gate", true),
        row_ref("2", "BTC", "kucoin", true),
    ];
    let meta = OpportunityCountMeta {
        total_count: 840,
        filtered_count: 520,
        executable_count: 410,
        strategy_counts: Default::default(),
        executable_strategy_counts: Default::default(),
        scan: Default::default(),
        source: String::new(),
        cached_at: None,
        ..Default::default()
    };

    let summary = summarize_rows(&rows, &OpportunityFilter::default(), &meta);

    assert_eq!(summary.candidates, 520);
    assert_eq!(summary.filtered_candidates, 520);
    assert_eq!(summary.executable_candidates, 410);
}

#[test]
fn merge_opportunity_rows_keeps_remote_symbol_once() {
    let base = vec![row_ref("1", "MU", "gate", true)];
    let extra = vec![
        row_ref("1", "MU", "gate", true),
        row_ref("2", "MU", "hyperliquid:km", true),
    ];

    let merged = merge_opportunity_rows(&base, &extra);

    assert_eq!(merged.len(), 2);
    assert_eq!(merged[1].long_venue, "hyperliquid:km");
}

#[test]
fn merge_opportunity_rows_reranks_by_verified_net() {
    let base = row_ref_with_net("rank-base", "1", "MU", "gate", true, 1.0);
    let extra = row_ref_with_net("rank-extra", "2", "MU", "hyperliquid:km", true, 9.0);

    let merged = merge_opportunity_rows(&[base], &[extra]);

    assert_eq!(merged[0].id, "2");
}

#[test]
fn symbol_opportunity_search_error_keeps_stale_rows() {
    Owner::new().with(|| {
        let state = RwSignal::new(LoadState::Ready(()));
        let rows = RwSignal::new(vec![row_ref("1", "MU", "gate", true)]);
        let page = RwSignal::new(Some(page("snapshot-1")));

        apply_symbol_opportunities_error(
            ApiError::from_problem(ApiProblem::new("RATE_LIMITED", "rate limited")),
            state,
            "mu-usdt",
            Some("cursor-1"),
        );

        assert_eq!(rows.get_untracked().len(), 1);
        assert_eq!(
            page.get_untracked()
                .as_ref()
                .map(|page| page.snapshot_id.as_str()),
            Some("snapshot-1")
        );
        assert!(matches!(state.get_untracked(), LoadState::Stale { .. }));
        assert_eq!(
            state
                .get_untracked()
                .problem()
                .map(|problem| problem.code.as_str()),
            Some("RATE_LIMITED")
        );
        assert_eq!(
            state
                .get_untracked()
                .problem()
                .and_then(|problem| problem.details.as_ref())
                .and_then(|details| details.get("symbolSearch"))
                .and_then(|details| details.get("query"))
                .and_then(serde_json::Value::as_str),
            Some("MU-USDT")
        );
        assert_eq!(
            state
                .get_untracked()
                .problem()
                .and_then(|problem| problem.details.as_ref())
                .and_then(|details| details.get("symbolSearch"))
                .and_then(|details| details.get("cursor"))
                .and_then(serde_json::Value::as_str),
            Some("cursor-1")
        );
    });
}

#[test]
fn symbol_opportunity_search_error_without_rows_is_error() {
    Owner::new().with(|| {
        let state = RwSignal::new(LoadState::Loading);

        apply_symbol_opportunities_error(
            ApiError::from_problem(ApiProblem::new("UPSTREAM", "upstream failed")),
            state,
            "mu",
            None,
        );

        assert!(matches!(state.get_untracked(), LoadState::Error(_)));
        assert_eq!(
            state
                .get_untracked()
                .problem()
                .map(|problem| problem.code.as_str()),
            Some("UPSTREAM")
        );
    });
}
