use super::*;
use shared_types::{ReviewPnlEvidence, StrategyKind};

#[test]
fn missing_pnl_field_hides_zero_value() {
    let mut row = trade();
    row.funding_usd = 0.0;
    row.missing_fields = vec![ReviewPnlField::Funding];

    let display = pnl_display(&row, ReviewPnlField::Funding, signed_money(row.funding_usd));

    assert_eq!(display.value, "缺证据");
    assert_eq!(display.class, "muted");
    assert_eq!(display.badge, "缺证据");
}

#[test]
fn actual_zero_fee_keeps_numeric_value() {
    let mut row = trade();
    row.fee_usd = 0.0;
    row.actual_fields = vec![ReviewPnlField::Fee];

    let display = pnl_display(&row, ReviewPnlField::Fee, money(row.fee_usd));

    assert_eq!(display.value, "$0.00");
    assert_eq!(display.class, "");
    assert_eq!(display.badge, "已确认");
}

#[test]
fn estimated_net_keeps_value_with_estimated_badge() {
    let mut row = trade();
    row.net_pnl_usd = 12.0;
    row.estimated_fields = vec![ReviewPnlField::Net];

    let display = pnl_display(&row, ReviewPnlField::Net, signed_money(row.net_pnl_usd));

    assert_eq!(display.value, "+$12.00");
    assert_eq!(display.class, "positive");
    assert_eq!(display.badge, "估算");
}

#[test]
fn unclassified_legacy_field_fails_closed() {
    let row = trade();

    let display = pnl_display(&row, ReviewPnlField::Gross, signed_money(row.gross_pnl_usd));

    assert_eq!(display.value, "缺证据");
    assert_eq!(display.badge, "缺证据");
}

#[test]
fn executed_summary_keeps_actual_estimated_and_missing_net_separate() {
    let mut actual = trade();
    actual.net_pnl_usd = 3.0;
    actual.actual_fields = vec![ReviewPnlField::Net];
    let mut estimated = trade();
    estimated.net_pnl_usd = -1.0;
    estimated.estimated_fields = vec![ReviewPnlField::Net];
    let missing = trade();

    let summary = executed_summary(&[actual, estimated, missing]);

    assert_eq!(summary.rows, 3);
    assert_eq!(summary.actual_count, 1);
    assert_eq!(summary.actual_net_usd, 3.0);
    assert_eq!(summary.estimated_count, 1);
    assert_eq!(summary.estimated_net_usd, -1.0);
    assert_eq!(summary.missing_count, 1);
}

fn trade() -> ExecutedTrade {
    ExecutedTrade {
        id: "hedge-1".into(),
        strategy: StrategyKind::PerpCross,
        symbol: "BTC".into(),
        long_venue: "binance".into(),
        short_venue: "okx".into(),
        opened_at_ms: 0,
        closed_at_ms: Some(1),
        holding_minutes: Some(1),
        gross_pnl_usd: 1.0,
        fee_usd: 0.1,
        funding_usd: 0.0,
        slippage_usd: 0.0,
        net_pnl_usd: 0.9,
        evidence: ReviewPnlEvidence::default(),
        actual_fields: Vec::new(),
        estimated_fields: Vec::new(),
        missing_fields: Vec::new(),
        long_orders: Vec::new(),
        short_orders: Vec::new(),
    }
}
