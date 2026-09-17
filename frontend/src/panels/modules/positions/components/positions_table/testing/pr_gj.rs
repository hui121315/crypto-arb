use super::super::derive::price;
use super::*;
use shared_types::PositionOrigin;

#[test]
fn position_price_keeps_tradeable_precision_for_low_price_assets() {
    assert_eq!(price(0.001745), "$0.00174500");
    assert_eq!(price(0.125), "$0.125000");
    assert_eq!(price(31.58), "$31.5800");
    assert_eq!(price(1_250.0), "$1250");
    assert_eq!(price(f64::NAN), "-");
}

#[test]
fn position_row_health_for_row_keeps_current_position_evidence_only() {
    let target = row("binance", "BTCUSDT");
    let rows = vec![
        account_data_health("binance", "BTCUSDT", "long", "position_cache"),
        account_data_health("binance", "ETHUSDT", "long", "position_cache"),
        account_data_health("okx", "BTCUSDT", "long", "position_cache"),
        account_data_health("binance", "BTCUSDT", "short", "position_cache"),
    ];

    let filtered = position_row_health_for_row(&target, &rows);

    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].source, "position_cache");
    assert_eq!(filtered[0].freshness_ms, Some(500));
}

#[test]
fn execution_ledger_row_ignores_private_account_evidence() {
    let mut target = row("binance", "BTCUSDT");
    target.origin = PositionOrigin::ExecutionLedger;
    let quality = vec![quality(
        "binance",
        "BTCUSDT",
        "long",
        "nextFundingMs",
        AccountFieldQualityStatus::Missing,
    )];
    let health = vec![account_data_health(
        "binance",
        "BTCUSDT",
        "long",
        "position_cache",
    )];

    assert!(position_quality_for_row(&target, &quality).is_empty());
    assert!(position_row_health_for_row(&target, &health).is_empty());
}

#[test]
fn position_data_health_record_keeps_problem_context() {
    let mut health = account_data_health("gate", "BTCUSDT", "long", "position_cache");
    health.retry_after_ms = Some(2_000);
    health.request_id = Some("req-1".into());
    health.last_error = Some(shared_types::ApiProblem::new(
        "POSITION_READ_DEGRADED",
        "rate limited",
    ));

    let title = position_data_health_title(&health);

    assert!(title.contains("gate BTCUSDT long"));
    assert!(title.contains("重试 2s"));
    assert!(title.contains("请求 req-1"));
    assert!(title.contains("POSITION_READ_DEGRADED"));
    assert_eq!(position_data_health_class(&health), "blocked");
}
