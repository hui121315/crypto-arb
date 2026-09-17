use super::super::*;
use super::fixtures::*;

#[test]
fn order_capability_guard_passes_and_scopes_double_leg() {
    let long = plan("Gate", "BTC_USDT", Vec::new());
    let short = plan("kucoin", "BTCUSDTM", Vec::new());

    let checks = [
        OrderCapabilityCheck::new(&long, Ok(capabilities())),
        OrderCapabilityCheck::new(&short, Ok(capabilities())),
    ];

    let guard = order_capability_guard(&checks);
    assert!(guard.preflight_outcome.is_some());
    let outcome = guard.preflight_outcome.unwrap_or_default();

    assert!(guard.passed);
    assert_eq!(guard.detail, "通过");
    assert_eq!(outcome.status, HedgePreflightStatus::Passed);
    assert_eq!(outcome.scope.venues, vec!["gate", "kucoin"]);
    assert_eq!(outcome.scope.symbols, vec!["BTC_USDT", "BTCUSDTM"]);
    assert_eq!(
        outcome.scope.operations,
        vec![HedgePreflightOperation::Capability]
    );
    assert_eq!(outcome.observed_venues, vec!["gate", "kucoin"]);
}

#[test]
fn order_capability_guard_blocks_unique_plan_blockers() {
    let long = plan("gate", "ETH_USDT", vec!["Gate futures 不支持 GTX".into()]);
    let short = plan("gate", "ETH_USDT", vec!["Gate futures 不支持 GTX".into()]);

    let checks = [
        OrderCapabilityCheck::new(&long, Ok(capabilities())),
        OrderCapabilityCheck::new(&short, Ok(capabilities())),
    ];

    let guard = order_capability_guard(&checks);
    assert!(guard.preflight_outcome.is_some());
    let outcome = guard.preflight_outcome.unwrap_or_default();

    assert!(!guard.passed);
    assert_eq!(outcome.status, HedgePreflightStatus::Blocked);
    assert_eq!(outcome.scope.venues, vec!["gate"]);
    assert_eq!(outcome.scope.symbols, vec!["ETH_USDT"]);
    assert_eq!(
        outcome.error.as_deref(),
        Some("订单能力阻断: Gate futures 不支持 GTX")
    );
}

#[test]
fn order_capability_guard_blocks_missing_route_and_omits_observed() {
    let long = plan("gate", "ETH_USDT", Vec::new());
    let short = plan("kucoin", "ETHUSDTM", Vec::new());
    let checks = [
        OrderCapabilityCheck::new(&long, Ok(capabilities())),
        OrderCapabilityCheck::new(&short, Err("unsupported exchange".to_owned())),
    ];

    let guard = order_capability_guard(&checks);
    assert!(guard.preflight_outcome.is_some());
    let outcome = guard.preflight_outcome.unwrap_or_default();

    assert!(!guard.passed);
    assert_eq!(outcome.status, HedgePreflightStatus::Blocked);
    assert_eq!(outcome.observed_venues, vec!["gate"]);
    assert!(outcome
        .error
        .unwrap_or_default()
        .contains("kucoin ETHUSDTM"));
}

#[test]
fn order_capability_guard_blocks_unsupported_market_order() {
    let mut long = plan("paper", "BTCUSDT", Vec::new());
    long.effective_order_type = OrderType::Market;
    let mut caps = capabilities();
    caps.supports_market_orders = false;
    let check = OrderCapabilityCheck::new(&long, Ok(caps));

    let guard = order_capability_guard(&[check]);
    assert!(guard.preflight_outcome.is_some());
    let outcome = guard.preflight_outcome.unwrap_or_default();

    assert!(!guard.passed);
    assert_eq!(
        outcome.error.as_deref(),
        Some("订单能力阻断: paper 不支持市价单")
    );
}

#[test]
fn order_capability_guard_blocks_unsupported_spot_leg() {
    let mut long = plan("okx", "BTCUSDT", Vec::new());
    long.product = FeeProduct::Spot;
    let mut caps = capabilities();
    caps.supports_spot = false;

    let guard = order_capability_guard(&[OrderCapabilityCheck::new(&long, Ok(caps))]);
    let outcome = guard.preflight_outcome.unwrap_or_default();

    assert!(!guard.passed);
    assert_eq!(
        outcome.error.as_deref(),
        Some("订单能力阻断: okx 不支持现货腿实盘下单")
    );
}

#[test]
fn order_capability_guard_blocks_unknown_leg_product() {
    let mut long = plan("paper", "BTCUSDT", Vec::new());
    long.product = FeeProduct::Unknown;

    let guard = order_capability_guard(&[OrderCapabilityCheck::new(&long, Ok(capabilities()))]);
    let outcome = guard.preflight_outcome.unwrap_or_default();

    assert!(!guard.passed);
    assert_eq!(
        outcome.error.as_deref(),
        Some("订单能力阻断: paper BTCUSDT 交易产品类型未验证")
    );
}

#[test]
fn order_write_guard_records_live_preflight_scope() {
    let long = plan("okx", "BTC-USDT-SWAP", Vec::new());
    let short = plan("gate", "BTC_USDT", Vec::new());
    let checks = [
        OrderWriteCheck::new(&long, Ok(())),
        OrderWriteCheck::new(&short, Ok(())),
    ];

    let guard = order_write_guard(&checks).unwrap_or_else(missing_guard);
    let outcome = guard.preflight_outcome.unwrap_or_default();

    assert!(guard.passed);
    assert_eq!(outcome.status, HedgePreflightStatus::Passed);
    assert_eq!(outcome.scope.venues, vec!["okx", "gate"]);
    assert_eq!(outcome.scope.symbols, vec!["BTC-USDT-SWAP", "BTC_USDT"]);
    assert_eq!(
        outcome.scope.operations,
        vec![HedgePreflightOperation::OrderWrite]
    );
    assert_eq!(outcome.observed_venues, vec!["okx", "gate"]);
}

#[test]
fn order_write_guard_blocks_disabled_venue_status() {
    let long = plan("bybit", "BTCUSDT", Vec::new());
    let check = OrderWriteCheck::new(&long, Err("api_trading_disabled: IOC".to_owned()));

    let guard = order_write_guard(&[check]).unwrap_or_else(missing_guard);
    let outcome = guard.preflight_outcome.unwrap_or_default();

    assert!(!guard.passed);
    assert_eq!(outcome.status, HedgePreflightStatus::Blocked);
    assert_eq!(outcome.observed_venues, Vec::<String>::new());
    assert!(outcome
        .error
        .unwrap_or_default()
        .contains("bybit BTCUSDT 下单准入不可用"));
}
