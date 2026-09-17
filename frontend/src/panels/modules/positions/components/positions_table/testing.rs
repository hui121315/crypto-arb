//! `positions_table` 表格/质量派生的单元测试与 PositionRow/字段质量夹具。

use super::super::section_state::SectionData;
use super::derive::{
    filtered_sorted_rows, funding_display, pnl_display, severity_class, stable_render_rows,
    table_empty_text, table_status_label, value_or_missing, TablePage,
};
use super::quality::{
    field_quality_label, funding_quality_rows, liquidation_quality_rows,
    position_data_health_class, position_data_health_title, position_quality_for_row,
    position_row_health_for_row,
};
use super::row::{position_close_enabled, position_close_requires_live};
use super::{
    table_requires_account_setup, POSITIONS_PAGE_STORAGE_KEY, POSITIONS_QUERY_STORAGE_KEY,
};
use crate::panels::modules::positions::data::PortfolioAccountAccess;
use shared_types::{
    AccountDataHealth, AccountFieldQuality, AccountFieldQualityStatus, AccountFieldSubject,
    PositionRow, PositionSeverity, PositionSide,
};
use std::sync::Arc;

#[path = "testing/account_quality.rs"]
mod account_quality;
#[path = "testing/position_quality.rs"]
mod position_quality;
#[path = "testing/pr_dz.rs"]
mod pr_dz;
#[path = "testing/pr_gj.rs"]
mod pr_gj;

#[test]
fn loading_empty_text_is_not_ready_empty() {
    let page = page_for_test(SectionData::<Vec<PositionRow>>::loading(), "");

    assert_eq!(table_empty_text(&page), "读取持仓中");
    assert_eq!(
        table_status_label(SectionData::<Vec<PositionRow>>::loading(), 0),
        "读取中"
    );
}

#[test]
fn error_empty_text_keeps_problem() {
    let problem = shared_types::ApiProblem::new("UPSTREAM", "portfolio failed");
    let page = page_for_test(SectionData::<Vec<PositionRow>>::error(&problem), "");

    assert_eq!(table_empty_text(&page), "持仓读取失败：portfolio failed");
}

#[test]
fn ready_search_empty_is_distinct_from_no_positions() {
    let page = page_for_test(SectionData::ready(vec![row("binance", "BTCUSDT")]), "eth");

    assert_eq!(table_empty_text(&page), "没有匹配持仓");
}

#[test]
fn projected_snapshot_rows_remain_visible_without_private_credentials() {
    let access = PortfolioAccountAccess {
        configured_venues: Vec::new(),
        unconfigured_venues: vec!["binance".into()],
    };

    assert!(!table_requires_account_setup(
        &access,
        &SectionData::ready(vec![row("binance", "BTCUSDT")]),
        true,
    ));
    assert!(table_requires_account_setup(
        &access,
        &SectionData::ready(Vec::new()),
        false,
    ));
    assert!(!table_requires_account_setup(
        &access,
        &SectionData::ready(Vec::new()),
        true,
    ));
}

#[test]
fn account_position_close_waits_for_live_mode_but_ledger_close_does_not() {
    let account = row("binance", "SOLUSDT");
    let mut ledger = row("binance", "SOLUSDT");
    ledger.origin = shared_types::PositionOrigin::ExecutionLedger;

    assert!(position_close_requires_live(&account));
    assert!(!position_close_enabled(true, false));
    assert!(position_close_enabled(true, true));
    assert!(!position_close_requires_live(&ledger));
    assert!(position_close_enabled(false, false));
}

#[test]
fn funding_display_distinguishes_verified_zero_from_missing_evidence() {
    let mut target = row("binance", "BTCUSDT");
    target.seconds_until_funding = Some(600);
    target.funding_rate_verified = true;
    target.funding_rate_8h = 0.0;
    let missing = quality(
        "binance",
        "BTCUSDT",
        "long",
        "fundingRate8h",
        AccountFieldQualityStatus::Missing,
    );

    let verified = funding_display(&target, None);
    let blocked = funding_display(&target, Some(&missing));

    assert_eq!(verified.window, "10m");
    assert_eq!(verified.detail.as_deref(), Some("持平 +0.0000%"));
    assert_eq!(blocked.window, "缺证据");
    assert_eq!(blocked.detail, None);
    assert_eq!(blocked.class, "muted");
}

#[test]
fn funding_display_explains_cashflow_from_position_side() {
    let mut long = row("binance", "BTCUSDT");
    long.funding_rate_8h = 0.000_039;
    let mut short = long.clone();
    short.side = PositionSide::Short;

    assert_eq!(
        funding_display(&long, None).detail.as_deref(),
        Some("将付 +0.0039%")
    );
    assert_eq!(
        funding_display(&short, None).detail.as_deref(),
        Some("将收 +0.0039%")
    );

    long.funding_rate_8h = -0.000_039;
    short.funding_rate_8h = -0.000_039;
    assert_eq!(
        funding_display(&long, None).detail.as_deref(),
        Some("将收 -0.0039%")
    );
    assert_eq!(
        funding_display(&short, None).detail.as_deref(),
        Some("将付 -0.0039%")
    );
}

#[test]
fn funding_display_labels_the_settlement_rollover_window() {
    let mut target = row("binance", "ETHUSDT");
    target.seconds_until_funding = Some(0);
    target.funding_rate_verified = true;

    let display = funding_display(&target, None);

    assert_eq!(display.window, "结算中");
    assert_eq!(display.detail.as_deref(), Some("持平 +0.0000%"));
}

#[test]
fn stable_render_rows_reuses_dom_identity_within_the_same_displayed_minute() {
    let mut initial = row("binance", "ETHUSDT");
    initial.seconds_until_funding = Some(119);
    let first = stable_render_rows(None, &[initial.clone()]);

    initial.seconds_until_funding = Some(118);
    let second = stable_render_rows(Some(&first), &[initial.clone()]);
    assert!(Arc::ptr_eq(&first[0], &second[0]));

    initial.mark_price = 2.0;
    let changed = stable_render_rows(Some(&second), &[initial]);
    assert!(!Arc::ptr_eq(&second[0], &changed[0]));
}

#[test]
fn positions_runtime_storage_keys_are_namespaced() {
    assert!(POSITIONS_QUERY_STORAGE_KEY.starts_with("crossline.positions."));
    assert!(POSITIONS_PAGE_STORAGE_KEY.starts_with("crossline.positions."));
}

fn row(venue: &str, symbol: &str) -> PositionRow {
    PositionRow {
        venue: venue.to_owned(),
        symbol: symbol.to_owned(),
        origin: Default::default(),
        side: PositionSide::Long,
        quantity: 1.0,
        entry_price: 1.0,
        mark_price: 1.0,
        leverage: 1.0,
        unrealized_pnl_usd: 0.0,
        liquidation_price: None,
        liquidation_distance_pct: None,
        next_funding_ms: None,
        funding_rate_8h: 0.0,
        funding_rate_verified: true,
        maintenance_margin_ratio: 0.0,
        pair_evidence: None,
        paired_with: None,
        margin_usd: 1.0,
        severity: PositionSeverity::Ok,
        seconds_until_funding: None,
    }
}

fn page_for_test(section: SectionData<Vec<PositionRow>>, query: &str) -> TablePage {
    let source_total = section.value.len();
    let status = section.status.clone();
    let rows = filtered_sorted_rows(section, query);
    TablePage {
        source_total,
        total: rows.len(),
        rows,
        status,
    }
}

fn quality(
    venue: &str,
    symbol: &str,
    side: &str,
    field: &str,
    status: AccountFieldQualityStatus,
) -> AccountFieldQuality {
    AccountFieldQuality::new(
        AccountFieldSubject::position(venue, symbol, side),
        field,
        status,
        "account_position_runtime",
        Some(1),
    )
}

fn account_data_health(venue: &str, symbol: &str, side: &str, source: &str) -> AccountDataHealth {
    let mut health = AccountDataHealth::new(
        AccountFieldSubject::position(venue, symbol, side),
        source,
        1_000,
    );
    health.freshness_ms = Some(500);
    health.last_success_ms = Some(500);
    health
}
