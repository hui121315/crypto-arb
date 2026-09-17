//! `balance_panel` 纯派生的单元测试与 VenueOperationHealth/AccountFieldQuality/AccountDataHealth 夹具。

#[path = "testing/fixtures.rs"]
mod fixtures;

use super::derive::{
    account_level_quality_rows, account_quality_title, balance_data_health_class,
    balance_data_health_title, balance_evidence_title, balance_field_quality_rows, balance_groups,
    balance_health_rows, balance_quality_for_row, balance_row_health_for_row,
};
use super::evidence::bounded_evidence_rows;
use super::workbench::resolved_group_key;
use fixtures::*;

#[test]
fn evidence_rows_keep_four_inline_and_disclose_the_rest() {
    let (inline, overflow) = bounded_evidence_rows(vec![1, 2, 3, 4, 5, 6]);

    assert_eq!(inline, vec![1, 2, 3, 4]);
    assert_eq!(overflow, vec![5, 6]);
}

#[test]
fn balance_workbench_keeps_selection_or_falls_back_to_first_account() {
    let groups = vec![
        super::derive::VenueBalanceGroup {
            venue: "binance".into(),
            rows: Vec::new(),
            summary: None,
            hidden_dust_count: 0,
            unknown_valuation_count: 0,
        },
        super::derive::VenueBalanceGroup {
            venue: "okx".into(),
            rows: Vec::new(),
            summary: None,
            hidden_dust_count: 0,
            unknown_valuation_count: 0,
        },
    ];

    assert_eq!(
        resolved_group_key(&groups, Some("okx")).as_deref(),
        Some("okx")
    );
    assert_eq!(
        resolved_group_key(&groups, Some("gate")).as_deref(),
        Some("binance")
    );
}
use shared_types::{
    AccountFieldQualityStatus, AccountFieldSubjectKind, ApiProblem, VenueBalanceInfo,
    VenueOperationStatus,
};

#[test]
fn balance_groups_hide_only_assets_with_proven_sub_dollar_value() {
    let rows = vec![
        balance("bitget", "USDT", 25.0),
        balance("bitget", "BTC", 0.000_001),
        balance("bitget", "UNKNOWN", 0.25),
    ];
    let valuations = vec![
        valuation("bitget", "USDT", 24.98),
        valuation("bitget", "BTC", 0.07),
    ];

    let groups = balance_groups(rows, valuations, Vec::new());

    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].rows.len(), 2);
    assert_eq!(groups[0].rows[0].balance.currency, "USDT");
    assert_eq!(groups[0].rows[1].balance.currency, "UNKNOWN");
    assert_eq!(groups[0].hidden_dust_count, 1);
    assert_eq!(groups[0].unknown_valuation_count, 1);
}

#[test]
fn balance_groups_hide_exact_zero_assets_without_price_lookup() {
    let rows = vec![
        balance("binance", "USDT", 25.0),
        balance("binance", "BFUSD", 0.0),
    ];

    let groups = balance_groups(rows, Vec::new(), Vec::new());

    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].rows.len(), 1);
    assert_eq!(groups[0].rows[0].balance.currency, "USDT");
    assert_eq!(groups[0].hidden_dust_count, 1);
    assert_eq!(groups[0].unknown_valuation_count, 1);
}

#[test]
fn balance_groups_drop_empty_venue_without_account_summary() {
    let rows = vec![balance("hyperliquid:xyz", "USDC", 0.0)];

    let groups = balance_groups(rows, Vec::new(), Vec::new());

    assert!(groups.is_empty());
}

#[test]
fn balance_health_rows_keeps_only_balance_evidence() {
    let rows = vec![
        health_row("binance", "balance", VenueOperationStatus::Ok),
        health_row(
            "okx",
            "credential_probe:balance_read",
            VenueOperationStatus::Warn,
        ),
        health_row(
            "system",
            "storage:portfolio_nav",
            VenueOperationStatus::Warn,
        ),
    ];

    let filtered = balance_health_rows(rows);

    assert_eq!(filtered.len(), 2);
    assert_eq!(filtered[0].operation, "balance");
    assert_eq!(filtered[1].operation, "credential_probe:balance_read");
}

#[test]
fn balance_evidence_title_keeps_retry_and_error_context() {
    let mut row = health_row("gate", "balance", VenueOperationStatus::Blocked);
    row.retry_after_ms = Some(2_000);
    row.error = Some("rate limited".into());

    let title = balance_evidence_title(&row);

    assert!(title.contains("gate"));
    assert!(title.contains("BLOCK"));
    assert!(title.contains("retry 2s"));
    assert!(title.contains("rate limited"));
}

#[test]
fn balance_field_quality_rows_keeps_account_balance_attention_only() {
    let rows = vec![
        account_quality(
            AccountFieldSubjectKind::Account,
            "binance",
            None,
            "equity",
            AccountFieldQualityStatus::Unknown,
        ),
        account_quality(
            AccountFieldSubjectKind::Balance,
            "okx",
            Some("USDT"),
            "available",
            AccountFieldQualityStatus::Missing,
        ),
        account_quality(
            AccountFieldSubjectKind::Balance,
            "gate",
            Some("USDT"),
            "total",
            AccountFieldQualityStatus::Actual,
        ),
        account_quality(
            AccountFieldSubjectKind::Position,
            "bybit",
            None,
            "markPrice",
            AccountFieldQualityStatus::Invalid,
        ),
    ];

    let filtered = balance_field_quality_rows(rows);

    assert_eq!(filtered.len(), 2);
    assert_eq!(filtered[0].field, "equity");
    assert_eq!(filtered[1].field, "available");
}

#[test]
fn account_quality_title_keeps_problem_context() {
    let row = account_quality(
        AccountFieldSubjectKind::Account,
        "hyperliquid:xyz",
        None,
        "equity",
        AccountFieldQualityStatus::Unknown,
    )
    .with_problem(ApiProblem::new("ACCOUNT_FIELD_UNKNOWN", "equity missing"));

    let title = account_quality_title(&row);

    assert!(title.contains("hyperliquid:xyz"));
    assert!(title.contains("equity"));
    assert!(title.contains("ACCOUNT_FIELD_UNKNOWN"));
    assert!(title.contains("equity missing"));
}

#[test]
fn balance_quality_for_row_keeps_current_balance_attention_only() {
    let row = VenueBalanceInfo {
        venue: "OKX".into(),
        currency: "usdt".into(),
        total: 100.0,
        available: 90.0,
        frozen: 10.0,
        unrealized_pnl: 0.0,
    };
    let rows = vec![
        account_quality(
            AccountFieldSubjectKind::Balance,
            "okx",
            Some("USDT"),
            "available",
            AccountFieldQualityStatus::Missing,
        ),
        account_quality(
            AccountFieldSubjectKind::Balance,
            "binance",
            Some("USDT"),
            "available",
            AccountFieldQualityStatus::Missing,
        ),
        account_quality(
            AccountFieldSubjectKind::Balance,
            "okx",
            Some("USDC"),
            "available",
            AccountFieldQualityStatus::Invalid,
        ),
        account_quality(
            AccountFieldSubjectKind::Account,
            "okx",
            None,
            "equity",
            AccountFieldQualityStatus::Unknown,
        ),
        account_quality(
            AccountFieldSubjectKind::Balance,
            "okx",
            Some("USDT"),
            "total",
            AccountFieldQualityStatus::Actual,
        ),
    ];

    let filtered = balance_quality_for_row(&row, &rows);

    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].field, "available");
    assert_eq!(filtered[0].status, AccountFieldQualityStatus::Missing);
}

#[test]
fn balance_row_health_for_row_keeps_current_balance_evidence_only() {
    let row = VenueBalanceInfo {
        venue: "Gate".into(),
        currency: "usdt".into(),
        total: 100.0,
        available: 90.0,
        frozen: 10.0,
        unrealized_pnl: 0.0,
    };
    let rows = vec![
        account_data_health("gate", "USDT", "account_cache"),
        account_data_health("gate", "USDC", "account_cache"),
        account_data_health("binance", "USDT", "account_cache"),
    ];

    let filtered = balance_row_health_for_row(&row, &rows);

    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].source, "account_cache");
    assert_eq!(filtered[0].freshness_ms, Some(500));
}

#[test]
fn balance_data_health_title_keeps_retry_request_and_problem() {
    let mut row = account_data_health("hyperliquid:xyz", "USDC", "account_cache");
    row.retry_after_ms = Some(2_000);
    row.request_id = Some("req-1".into());
    row.last_error = Some(ApiProblem::new("BALANCE_READ_DEGRADED", "rate limited"));

    let title = balance_data_health_title(&row);

    assert!(title.contains("hyperliquid:xyz USDC"));
    assert!(title.contains("retry 2s"));
    assert!(title.contains("request req-1"));
    assert!(title.contains("BALANCE_READ_DEGRADED"));
    assert_eq!(balance_data_health_class(&row), "blocked");
}

#[test]
fn account_level_quality_rows_keeps_account_attention() {
    let rows = vec![
        account_quality(
            AccountFieldSubjectKind::Account,
            "okx",
            None,
            "equity",
            AccountFieldQualityStatus::Unknown,
        ),
        account_quality(
            AccountFieldSubjectKind::Balance,
            "okx",
            Some("USDT"),
            "available",
            AccountFieldQualityStatus::Missing,
        ),
    ];

    let filtered = account_level_quality_rows(&rows);

    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].field, "equity");
}
