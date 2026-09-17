use super::super::*;
use super::fixtures::*;

#[test]
fn instrument_sizing_guard_skips_non_live_mode() {
    let long = plan("binance", "BTCUSDT", Vec::new());
    let checks = [InstrumentSizingCheck::new(
        &long,
        true,
        Ok(sizing_plan_fixture()),
    )];
    assert!(instrument_sizing_guard(ExecutionMode::DryRun, &checks).is_none());
}

#[test]
fn instrument_sizing_guard_skips_when_no_venue_covered() {
    // 尚未接线 instrument feed 的 venue 不挂闸门（仍由其它闸门把关）。
    let long = plan("okx", "BTC-USDT-SWAP", Vec::new());
    let checks = [InstrumentSizingCheck::new(
        &long,
        false,
        Err(SizingBlock::SpecMissing),
    )];
    assert!(instrument_sizing_guard(ExecutionMode::Live, &checks).is_none());
}

#[test]
fn instrument_sizing_guard_passes_for_constructible_leg() {
    let long = plan("binance", "BTCUSDT", Vec::new());
    let checks = [InstrumentSizingCheck::new(
        &long,
        true,
        Ok(sizing_plan_fixture()),
    )];
    let guard = instrument_sizing_guard(ExecutionMode::Live, &checks).unwrap_or_else(missing_guard);
    assert!(guard.passed);
    assert_eq!(guard.detail, "通过");
    let outcome = guard.preflight_outcome.unwrap_or_default();
    assert_eq!(outcome.status, HedgePreflightStatus::Passed);
    assert_eq!(outcome.scope.venues, vec!["binance"]);
    assert_eq!(outcome.observed_venues, vec!["binance"]);
}

#[test]
fn instrument_sizing_guard_blocks_missing_spec_fail_closed() {
    let long = plan("binance", "BTCUSDT", Vec::new());
    let checks = [InstrumentSizingCheck::new(
        &long,
        true,
        Err(SizingBlock::SpecMissing),
    )];
    let guard = instrument_sizing_guard(ExecutionMode::Live, &checks).unwrap_or_else(missing_guard);
    assert!(!guard.passed);
    assert!(guard.detail.contains("缺官方核验下单规格"));
    let outcome = guard.preflight_outcome.unwrap_or_default();
    assert_eq!(outcome.status, HedgePreflightStatus::Blocked);
    assert!(outcome.observed_venues.is_empty());
}

#[test]
fn instrument_sizing_guard_blocks_below_min_notional() {
    let long = plan("binance", "BTCUSDT", Vec::new());
    let checks = [InstrumentSizingCheck::new(
        &long,
        true,
        Err(SizingBlock::BelowMinNotional),
    )];
    let guard = instrument_sizing_guard(ExecutionMode::Live, &checks).unwrap_or_else(missing_guard);
    assert!(!guard.passed);
    assert!(guard.detail.contains("min_notional"));
}

#[test]
fn instrument_sizing_guard_ignores_uncovered_leg_but_governs_covered() {
    // 一腿 binance（受治理、合法），一腿 okx（未接线）→ 整体通过、scope 仅含 binance。
    let covered = plan("binance", "BTCUSDT", Vec::new());
    let uncovered = plan("okx", "BTC-USDT-SWAP", Vec::new());
    let checks = [
        InstrumentSizingCheck::new(&covered, true, Ok(sizing_plan_fixture())),
        InstrumentSizingCheck::new(&uncovered, false, Err(SizingBlock::SpecMissing)),
    ];
    let guard = instrument_sizing_guard(ExecutionMode::Live, &checks).unwrap_or_else(missing_guard);
    assert!(guard.passed);
    let outcome = guard.preflight_outcome.unwrap_or_default();
    assert_eq!(outcome.scope.venues, vec!["binance"]);
}
