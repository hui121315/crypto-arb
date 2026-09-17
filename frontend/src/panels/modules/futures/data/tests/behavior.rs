use super::super::*;
use super::support::*;
use crate::panels::modules::instrument_search::symbol_search_query;
use crate::panels::modules::opportunity_counts::OpportunityCountMeta;
use shared_types::{SpotLegMode, StrategyKind};
use std::sync::Arc;

#[test]
fn summary_counts_total_before_visible_cap() {
    let rows: Vec<_> = (0..150)
        .map(|idx| row(idx, StrategyKind::PerpCross, 10.0, 50_000.0))
        .collect();

    let summary = summarize_rows(
        &rows,
        &FuturesFilter::default(),
        &OpportunityCountMeta::default(),
    );
    let visible = filter_rows(&rows, &FuturesFilter::default());

    assert_eq!(summary.candidates, 150);
    assert_eq!(summary.filtered_candidates, 150);
    assert_eq!(visible.len(), 150);
}

#[test]
fn summary_counts_filtered_without_losing_total() {
    let mut best = row(2, StrategyKind::SpotPerp, 30.0, 80_000.0);
    let best_mut = Arc::make_mut(&mut best);
    best_mut.one_cycle_net_bps = 3.0;
    best_mut.one_cycle_net = "+0.030%".into();
    best_mut.execution_eligible = true;
    let rows = vec![
        row(1, StrategyKind::PerpCross, 12.0, 50_000.0),
        best,
        row(3, StrategyKind::SpotPerp, 4.0, 80_000.0),
    ];
    let filter = FuturesFilter {
        strategy: StrategyFilter::SpotPerp,
        min_net_pct: 0.02,
        query: String::new(),
    };

    let summary = summarize_rows(&rows, &filter, &OpportunityCountMeta::default());

    assert_eq!(summary.candidates, 2);
    assert_eq!(summary.filtered_candidates, 1);
    assert_eq!(summary.best_monitor_pair, "SYM2");
}

#[test]
fn summary_prefers_highest_verified_net_profit() {
    let mut lower = row(1, StrategyKind::PerpCross, 12.0, 50_000.0);
    let lower_mut = Arc::make_mut(&mut lower);
    lower_mut.one_cycle_net_bps = 1.0;
    lower_mut.one_cycle_net = "+0.010%".into();
    lower_mut.execution_eligible = true;
    let mut best = row(2, StrategyKind::PerpCross, 18.0, 80_000.0);
    let best_mut = Arc::make_mut(&mut best);
    best_mut.one_cycle_net_bps = 5.0;
    best_mut.one_cycle_net = "+0.050%".into();
    best_mut.execution_eligible = true;

    let summary = summarize_rows(
        &[lower, best],
        &FuturesFilter::default(),
        &OpportunityCountMeta::default(),
    );

    assert_eq!(summary.best_monitor_net_bps, Some(5.0));
    assert_eq!(summary.best_monitor_pair, "SYM2");
    assert!(summary.best_monitor_profit_detail.contains("费率证据 2/2"));
    assert!(summary.best_monitor_profit_detail.contains("+0.050%"));
    assert!(summary.best_monitor_preview_ready);
}

#[test]
fn summary_keeps_inventory_deferred_profit_visible_for_monitoring() {
    let mut observation = row(1, StrategyKind::PerpCross, 12.0, 50_000.0);
    let observation_mut = Arc::make_mut(&mut observation);
    observation_mut.one_cycle_net_bps = 10.0;
    observation_mut.execution_blockers =
        vec![shared_types::DEFERRED_INVENTORY_OR_BORROW_BLOCKER.to_owned()];
    let mut executable = row(2, StrategyKind::PerpCross, 10.0, 50_000.0);
    let executable_mut = Arc::make_mut(&mut executable);
    executable_mut.one_cycle_net_bps = 2.0;
    executable_mut.execution_eligible = true;

    let summary = summarize_rows(
        &[observation, executable],
        &FuturesFilter::default(),
        &OpportunityCountMeta::default(),
    );

    assert_eq!(summary.executable_candidates, 1);
    assert_eq!(summary.best_monitor_net_bps, Some(10.0));
    assert_eq!(summary.best_monitor_pair, "SYM1");
    assert!(!summary.best_monitor_preview_ready);
}

#[test]
fn summary_rejects_non_inventory_monitoring_blockers() {
    let mut observation = row(1, StrategyKind::PerpCross, 12.0, 50_000.0);
    let observation_mut = Arc::make_mut(&mut observation);
    observation_mut.one_cycle_net_bps = 10.0;
    observation_mut.execution_blockers = vec!["双边价格或经济标的身份异常".into()];

    let summary = summarize_rows(
        &[observation],
        &FuturesFilter::default(),
        &OpportunityCountMeta::default(),
    );

    assert_eq!(summary.executable_candidates, 0);
    assert_eq!(summary.best_monitor_net_bps, None);
    assert_eq!(summary.best_monitor_pair, "-");
}

#[test]
fn net_filter_requires_verified_single_cycle_profit() {
    let mut unverified = row(1, StrategyKind::SpotPerp, 0.0, 50_000.0);
    let unverified_mut = Arc::make_mut(&mut unverified);
    unverified_mut.cost_verified = false;
    unverified_mut.one_cycle_net_bps = 100.0;
    let mut verified = row(2, StrategyKind::SpotPerp, 12.0, 50_000.0);
    Arc::make_mut(&mut verified).one_cycle_net_bps = 2.0;
    let rows = vec![unverified, verified];

    let default = FuturesFilter {
        strategy: StrategyFilter::SpotPerp,
        ..FuturesFilter::default()
    };

    assert_eq!(filter_rows(&rows, &default).len(), 2);

    let filter = FuturesFilter {
        strategy: StrategyFilter::SpotPerp,
        min_net_pct: 0.01,
        ..FuturesFilter::default()
    };
    let visible = filter_rows(&rows, &filter);
    let summary = summarize_rows(&rows, &filter, &OpportunityCountMeta::default());

    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].id, "opp-2");
    assert_eq!(summary.filtered_candidates, 1);
}

#[test]
fn every_strategy_uses_the_same_verified_net_filter() {
    let mut spread = row(1, StrategyKind::SpotCross, 100.0, 50_000.0);
    Arc::make_mut(&mut spread).one_cycle_net_bps = 9.0;
    let filter = FuturesFilter {
        strategy: StrategyFilter::SpotCross,
        min_net_pct: 0.10,
        query: String::new(),
    };

    let visible = filter_rows(&[spread], &filter);

    assert!(visible.is_empty());
}

#[test]
fn one_shot_spread_filters_by_single_cycle_net_percent() {
    let mut below = row(1, StrategyKind::SpotCross, 0.0, 50_000.0);
    Arc::make_mut(&mut below).one_cycle_net_bps = 9.0;
    let mut above = row(2, StrategyKind::SpotCross, 0.0, 50_000.0);
    Arc::make_mut(&mut above).one_cycle_net_bps = 11.0;
    let filter = FuturesFilter {
        strategy: StrategyFilter::SpotCross,
        min_net_pct: 0.10,
        query: String::new(),
    };

    let visible = filter_rows(&[below, above], &filter);

    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].id, "opp-2");
}

#[test]
fn missing_explanatory_apr_does_not_hide_a_candidate() {
    let mut missing_apr = row(1, StrategyKind::SpotPerp, 0.0, 50_000.0);
    Arc::make_mut(&mut missing_apr).est_apr_pct = None;
    let rows = vec![missing_apr];
    let filter = FuturesFilter {
        strategy: StrategyFilter::SpotPerp,
        ..FuturesFilter::default()
    };

    let summary = summarize_rows(&rows, &filter, &OpportunityCountMeta::default());

    assert_eq!(summary.filtered_candidates, 1);
}

#[test]
fn search_filters_symbol_and_venue_text() {
    let rows = vec![
        row(1, StrategyKind::PerpCross, 12.0, 50_000.0),
        row(2, StrategyKind::PerpCross, 18.0, 80_000.0),
    ];
    let symbol_filter = FuturesFilter {
        query: "sym2".into(),
        ..FuturesFilter::default()
    };
    let venue_filter = FuturesFilter {
        query: "okx".into(),
        ..FuturesFilter::default()
    };

    assert_eq!(filter_rows(&rows, &symbol_filter).len(), 1);
    assert_eq!(
        summarize_rows(&rows, &venue_filter, &OpportunityCountMeta::default()).filtered_candidates,
        2
    );
}

#[test]
fn search_matches_spot_leg_mode_label() {
    let mut reverse = row(1, StrategyKind::SpotPerp, 12.0, 50_000.0);
    Arc::make_mut(&mut reverse).spot_leg_mode = Some(SpotLegMode::BorrowAndSell);
    let rows = vec![reverse, row(2, StrategyKind::PerpCross, 18.0, 80_000.0)];
    let filter = FuturesFilter {
        strategy: StrategyFilter::SpotPerp,
        query: "借币".into(),
        ..FuturesFilter::default()
    };

    let visible = filter_rows(&rows, &filter);

    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].spot_leg_mode, Some(SpotLegMode::BorrowAndSell));
}

#[test]
fn summary_uses_product_visible_total_separate_from_loaded_page_count() {
    let rows: Vec<_> = (0..100)
        .map(|idx| row(idx, StrategyKind::PerpCross, 10.0, 50_000.0))
        .collect();
    let meta = OpportunityCountMeta {
        filtered_count: 560,
        source: "snapshot".into(),
        ..OpportunityCountMeta::default()
    };

    let summary = summarize_rows(&rows, &FuturesFilter::default(), &meta);

    assert_eq!(summary.candidates, 560);
    assert_eq!(summary.filtered_candidates, 100);
}

#[test]
fn symbol_search_matches_exact_base_symbol() {
    let mut mu = row(1, StrategyKind::PerpCross, 12.0, 50_000.0);
    Arc::make_mut(&mut mu).pair = "MU".into();
    let mut mubarak = row(2, StrategyKind::PerpCross, 18.0, 80_000.0);
    Arc::make_mut(&mut mubarak).pair = "MUBARAK".into();
    let rows = vec![mu, mubarak];
    let filter = FuturesFilter {
        query: "mu".into(),
        ..FuturesFilter::default()
    };

    let visible = filter_rows(&rows, &filter);

    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].pair, "MU");
}

#[test]
fn symbol_search_query_skips_venue_only_terms() {
    assert_eq!(symbol_search_query("mu"), Some("MU".into()));
    assert_eq!(symbol_search_query("MU-USDT"), Some("MU-USDT".into()));
    assert_eq!(symbol_search_query("binance"), None);
    assert_eq!(symbol_search_query("hyperliquid:xyz"), None);
}
