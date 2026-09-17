use super::{limit_intent, market_intent};
use crate::risk::{RiskConfig, RiskEngine};
use shared_types::{ExecutionMode, ProtectedPositionFingerprint, RiskBlockReason};
use std::collections::BTreeSet;

#[test]
fn kill_switch_blocks_order() {
    let engine = RiskEngine::new(RiskConfig::default());
    engine.set_kill_switch(true);
    let decision = engine.check_order(&limit_intent(), 0);
    assert!(!decision.allowed);
    assert!(decision
        .reasons
        .contains(&RiskBlockReason::KillSwitchActive));
}

#[test]
fn kill_switch_allows_reduce_only_market_close() {
    let engine = RiskEngine::new(RiskConfig {
        kill_switch_active: true,
        max_order_notional: 100.0,
        max_open_orders: 0,
        ..RiskConfig::default()
    });
    let decision = engine.check_order(&market_intent(), 8);

    assert!(decision.allowed, "decision was: {:?}", decision.reasons);
    assert!((decision.computed_notional - 505.0).abs() < f64::EPSILON);
}

#[test]
fn live_mode_requires_live_trading_enabled() {
    let engine = RiskEngine::new(RiskConfig::default());
    let mut intent = limit_intent();
    intent.mode = ExecutionMode::Live;
    let decision = engine.check_order(&intent, 0);
    assert!(!decision.allowed);
    assert!(decision
        .reasons
        .contains(&RiskBlockReason::LiveTradingDisabled));
}

#[test]
fn exchange_not_in_whitelist_is_blocked() {
    let engine = RiskEngine::new(RiskConfig {
        allowed_exchanges: BTreeSet::from(["binance".to_owned()]),
        ..RiskConfig::default()
    });
    let decision = engine.check_order(&limit_intent(), 0);
    assert!(!decision.allowed);
    assert!(decision
        .reasons
        .contains(&RiskBlockReason::ExchangeNotAllowed));
}

#[test]
fn exchange_whitelist_allows_non_hyperliquid_family_route() {
    let engine = RiskEngine::new(RiskConfig {
        allowed_exchanges: BTreeSet::from(["binance".to_owned()]),
        ..RiskConfig::default()
    });
    let mut intent = limit_intent();
    intent.exchange = "binance:um".into();

    let decision = engine.check_order(&intent, 0);

    assert!(decision.allowed, "decision was: {:?}", decision.reasons);
}

#[test]
fn exchange_whitelist_normalizes_configured_venue_names() {
    let engine = RiskEngine::new(RiskConfig {
        allowed_exchanges: BTreeSet::from([" OKX ".to_owned()]),
        ..RiskConfig::default()
    });
    let mut intent = limit_intent();
    intent.exchange = "okx".into();

    let decision = engine.check_order(&intent, 0);

    assert!(decision.allowed, "decision was: {:?}", decision.reasons);
}

#[test]
fn exchange_whitelist_requires_exact_hyperliquid_builder_route() {
    let engine = RiskEngine::new(RiskConfig {
        allowed_exchanges: BTreeSet::from(["hyperliquid".to_owned()]),
        ..RiskConfig::default()
    });
    let mut intent = limit_intent();
    intent.exchange = "hyperliquid:xyz".into();

    let decision = engine.check_order(&intent, 0);

    assert!(!decision.allowed);
    assert!(decision
        .reasons
        .contains(&RiskBlockReason::ExchangeNotAllowed));
}

#[test]
fn symbol_not_in_whitelist_is_blocked() {
    let engine = RiskEngine::new(RiskConfig {
        allowed_symbols: BTreeSet::from(["ETH".to_owned()]),
        ..RiskConfig::default()
    });
    let decision = engine.check_order(&limit_intent(), 0);
    assert!(!decision.allowed);
    assert!(decision
        .reasons
        .contains(&RiskBlockReason::SymbolNotAllowed));
}

#[test]
fn symbol_whitelist_is_case_and_whitespace_insensitive() {
    let engine = RiskEngine::new(RiskConfig {
        allowed_symbols: BTreeSet::from([" solusdt ".to_owned()]),
        ..RiskConfig::default()
    });
    let mut intent = limit_intent();
    intent.symbol = "SOLUSDT".to_owned();

    let decision = engine.check_order(&intent, 0);

    assert!(decision.allowed, "decision was: {:?}", decision.reasons);
    assert_eq!(
        engine.config().allowed_symbols,
        BTreeSet::from(["solusdt".to_owned()])
    );
}

#[test]
fn protected_position_blocks_standard_order_by_canonical_or_native_symbol() {
    let engine = RiskEngine::new(RiskConfig {
        protected_positions: vec![protected_btc_position()],
        ..RiskConfig::default()
    });
    let canonical = engine.check_order(&limit_intent(), 0);
    let mut native_intent = limit_intent();
    native_intent.symbol = "BTCUSDT".to_owned();
    let native = engine.check_order(&native_intent, 0);

    for decision in [canonical, native] {
        assert!(!decision.allowed);
        assert!(decision
            .reasons
            .contains(&RiskBlockReason::ProtectedPosition));
        let evidence = decision
            .evidence
            .iter()
            .find(|item| item.code == RiskBlockReason::ProtectedPosition)
            .expect("protected position evidence");
        assert_eq!(evidence.field, "protected_position_fingerprint");
        assert!(evidence
            .actual
            .as_ref()
            .is_some_and(|actual| { actual["openingIdentity"] == "preexisting-binance-btc-long" }));
    }
}

#[test]
fn protected_position_does_not_block_unrelated_symbol() {
    let engine = RiskEngine::new(RiskConfig {
        protected_positions: vec![protected_btc_position()],
        ..RiskConfig::default()
    });
    let mut intent = limit_intent();
    intent.symbol = "ETHUSDT".to_owned();

    let decision = engine.check_order(&intent, 0);

    assert!(decision.allowed, "decision was: {:?}", decision.reasons);
}

pub(super) fn protected_btc_position() -> ProtectedPositionFingerprint {
    ProtectedPositionFingerprint {
        venue: " mock ".to_owned(),
        canonical_symbol: " BTC ".to_owned(),
        native_symbol: " BTCUSDT ".to_owned(),
        side: " LONG ".to_owned(),
        quantity: 0.232,
        entry_price: 64_456.2,
        position_mode: Some(" BOTH ".to_owned()),
        opening_identity: " preexisting-binance-btc-long ".to_owned(),
        source: " account_position_runtime ".to_owned(),
        captured_at_ms: 42,
    }
}
