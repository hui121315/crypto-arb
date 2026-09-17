use crate::api::rest::ApiError;
use crate::state::load_state::LoadState;
use crate::state::polling::polling_allowed;
use leptos::prelude::*;
use shared_types::ApiProblem;
use shared_types::{StrategyKind, StrategyKindInfo};
use std::sync::Arc;

use super::super::*;
use super::support::*;
use crate::panels::modules::strategy_scope::p0_strategy_dtos;
use leptos::prelude::Owner;

#[test]
fn execution_selection_carries_profit_evidence() {
    let mut row = row(1, StrategyKind::PerpCross, 120.0, 50_000.0);
    let row_mut = Arc::make_mut(&mut row);
    row_mut.fee_evidence_ids = vec!["fee:binance:perp:vip0".into()];
    row_mut.one_cycle_net_bps = -4.0;

    let selection = row.execution_seed().into_selection();

    assert_eq!(selection.fee_evidence_ids, ["fee:binance:perp:vip0"]);
    assert_eq!(selection.one_cycle_net_bps, -4.0);
}

#[test]
fn execution_selection_does_not_invent_a_missing_strategy_size() {
    let row = row(1, StrategyKind::PerpCross, 120.0, 0.0);

    let selection = row.execution_seed().into_selection();

    assert_eq!(selection.default_capital_usd, 0.0);
}

#[test]
fn execution_selection_uses_strategy_size_before_build_time_depth_check() {
    let mut row = row(1, StrategyKind::PerpCross, 120.0, 0.0);
    Arc::make_mut(&mut row).optimal_position = 375.0;

    let selection = row.execution_seed().into_selection();

    assert_eq!(selection.default_capital_usd, 187.5);
}

#[test]
fn execution_selection_keeps_leg_labels_without_venue_seed() {
    let mut row = row(1, StrategyKind::PerpCross, 120.0, 375.0);
    let row = Arc::make_mut(&mut row);
    row.long_venue = "hyperliquid:xyz".into();
    row.short_venue = "kucoin".into();
    row.long_leg = "HYPE 做多".into();
    row.short_leg = "KuCoin 做空".into();

    let selection = row.execution_seed().into_selection();

    assert_eq!(selection.long_leg_label, "HYPE 做多");
    assert_eq!(selection.short_leg_label, "KuCoin 做空");
}

#[test]
fn execution_selection_keeps_raw_market_evidence() {
    let mut row = row(1, StrategyKind::PerpCross, 120.0, 375.0);
    let row_mut = Arc::make_mut(&mut row);
    row_mut.long_market_evidence_raw = Some(leg_evidence("hyperliquid:xyz", "MU"));

    let selection = row.execution_seed().into_selection();

    assert!(selection
        .long_market_evidence
        .as_ref()
        .is_some_and(|evidence| evidence.venue == "hyperliquid:xyz"));
}

#[test]
fn execution_selection_matches_list_view_for_high_risk_and_optimal_position() {
    let mut row = row(1, StrategyKind::PerpCross, 120.0, 50_000.0);
    let row_mut = Arc::make_mut(&mut row);
    row_mut.risk = "高".into();
    row_mut.risk_level = shared_types::RiskLevel::High;
    row_mut.optimal_position = 12_000.0;
    row_mut.settlement_countdown_seconds = Some(120);
    let selection = row.execution_seed().into_selection();

    assert_eq!(selection.source_module, "期货套利");
    assert_eq!(selection.default_capital_usd, 6_000.0);
    assert_eq!(selection.default_leverage, 1.0);
}

#[test]
fn cost_evidence_label_exposes_complete_and_partial_fee_evidence() {
    let complete = row(1, StrategyKind::PerpCross, 120.0, 375.0);
    let mut partial = row(2, StrategyKind::PerpCross, 120.0, 375.0);
    let partial_mut = Arc::make_mut(&mut partial);
    partial_mut.fee_evidence_count = 1;
    partial_mut.fee_evidence_complete = false;

    assert_eq!(complete.cost_evidence_label(), "费率证据 2/2");
    assert_eq!(partial.cost_evidence_label(), "费率证据 1/2 未完整");
}

#[test]
fn merge_futures_rows_keeps_remote_symbol_rows_once() {
    let base = vec![row(1, StrategyKind::PerpCross, 12.0, 50_000.0)];
    let mut extra = vec![row(1, StrategyKind::PerpCross, 12.0, 50_000.0)];
    extra.push(row(2, StrategyKind::PerpCross, 18.0, 80_000.0));

    let merged = merge_futures_rows(&base, &extra);

    assert_eq!(merged.len(), 2);
    assert!(merged.iter().any(|row| row.id == "opp-2"));
}

#[test]
fn merge_futures_rows_reranks_by_verified_net() {
    let mut base = row(1, StrategyKind::PerpCross, 12.0, 50_000.0);
    let base_mut = Arc::make_mut(&mut base);
    base_mut.one_cycle_net_bps = 1.0;
    let mut extra = row(2, StrategyKind::PerpCross, 18.0, 80_000.0);
    let extra_mut = Arc::make_mut(&mut extra);
    extra_mut.one_cycle_net_bps = 9.0;

    let merged = merge_futures_rows(&[base], &[extra]);

    assert_eq!(merged[0].id, "opp-2");
}

#[test]
fn symbol_futures_search_error_keeps_stale_rows() {
    Owner::new().with(|| {
        let state = RwSignal::new(LoadState::Ready(()));
        let rows = RwSignal::new(vec![row(1, StrategyKind::PerpCross, 12.0, 50_000.0)]);
        let page = RwSignal::new(Some(page("snapshot-1")));

        apply_symbol_futures_error(
            ApiError::from_problem(ApiProblem::new("RATE_LIMITED", "rate limited")),
            state,
            "sym1",
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
            Some("SYM1")
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
fn symbol_futures_search_error_without_rows_is_error() {
    Owner::new().with(|| {
        let state = RwSignal::new(LoadState::Loading);

        apply_symbol_futures_error(
            ApiError::from_problem(ApiProblem::new("UPSTREAM", "upstream failed")),
            state,
            "sym1",
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

#[test]
fn stream_patch_replaces_existing_futures_row_handle() {
    let keep = row(1, StrategyKind::PerpCross, 12.0, 50_000.0);
    let mut replacement = row(1, StrategyKind::PerpCross, 42.0, 90_000.0);
    Arc::make_mut(&mut replacement).one_cycle_net_bps = 9.7;
    let mut rows = vec![
        Arc::clone(&keep),
        row(2, StrategyKind::PerpCross, 18.0, 80_000.0),
    ];

    patch_futures_rows(&mut rows, &[], vec![replacement]);

    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].one_cycle_net_bps, 9.7);
    assert!(!Arc::ptr_eq(&rows[0], &keep));
}

#[test]
fn stream_patch_removes_deleted_futures_rows() {
    let mut rows = vec![
        row(1, StrategyKind::PerpCross, 12.0, 50_000.0),
        row(2, StrategyKind::PerpCross, 18.0, 80_000.0),
    ];

    patch_futures_rows(&mut rows, &["opp-1".into()], Vec::new());

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, "opp-2");
}

#[test]
fn live_first_page_poll_tick_waits_for_retry_deadline() {
    assert!(!first_page_fetch_allowed(None, Some(12_000), 11_999));
    assert!(first_page_fetch_allowed(None, Some(12_000), 12_000));

    let cursor = "next-page".to_owned();
    assert!(first_page_fetch_allowed(
        Some(&cursor),
        Some(12_000),
        11_999,
    ));
}

fn first_page_fetch_allowed(
    cursor: Option<&String>,
    retry_until_ms: Option<u64>,
    now_ms: u64,
) -> bool {
    cursor.is_some() || polling_allowed(true, retry_until_ms, now_ms)
}

#[test]
fn p0_strategy_dtos_keeps_all_five_strategy_rows() {
    let rows = vec![
        dto("perp", StrategyKind::PerpCross),
        dto("perp-spread", StrategyKind::PerpPriceSpread),
        dto("spot", StrategyKind::SpotPerp),
        dto("cross", StrategyKind::CrossSpotPerp),
        dto("spot-cross", StrategyKind::SpotCross),
        dto("carry", StrategyKind::FundingCarry),
    ];

    let rows = p0_strategy_dtos(rows);

    assert_eq!(rows.len(), 5);
    assert!(rows.iter().all(|row| row
        .strategy_kind
        .is_some_and(shared_types::is_p0_executable_strategy)));
}

#[test]
fn strategy_chips_require_backend_frontend_exposure() {
    let kinds = vec![
        StrategyKindInfo::from_kind(StrategyKind::PerpCross, true),
        StrategyKindInfo::from_kind(StrategyKind::SpotPerp, true),
        StrategyKindInfo::from_kind(StrategyKind::SpotCross, true),
    ];

    let chips = futures_chips(&kinds);

    assert_eq!(
        chips,
        vec![
            StrategyFilter::PerpCross,
            StrategyFilter::SpotPerp,
            StrategyFilter::SpotCross,
        ]
    );
}
