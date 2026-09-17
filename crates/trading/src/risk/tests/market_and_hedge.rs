use super::{gating::protected_btc_position, limit_intent, market_intent};
use crate::risk::{RiskConfig, RiskEngine};
use shared_types::{OrderSide, OrderSource, RiskBlockReason};
use std::collections::BTreeSet;

#[test]
fn manual_market_order_must_be_reduce_only() {
    let engine = RiskEngine::new(RiskConfig::default());
    let mut intent = market_intent();
    intent.reduce_only = false;
    let decision = engine.check_order(&intent, 0);
    assert!(!decision.allowed);
    assert!(decision
        .reasons
        .contains(&RiskBlockReason::MarketOrderNotReduceOnly));
}

#[test]
fn hedge_preview_market_order_is_allowed_with_reference_price() {
    let engine = RiskEngine::new(RiskConfig::default());
    let mut intent = market_intent();
    intent.source = OrderSource::ArbitragePreview;
    intent.reduce_only = false;

    let decision = engine.check_order(&intent, 0);

    assert!(decision.allowed, "decision was: {:?}", decision.reasons);
    assert!((decision.computed_notional - 505.0).abs() < f64::EPSILON);
}

#[test]
fn testnet_market_order_with_reference_price_is_allowed() {
    let engine = RiskEngine::new(RiskConfig::default());
    let decision = engine.check_order(&market_intent(), 0);
    assert!(decision.allowed, "decision was: {:?}", decision.reasons);
    assert!((decision.computed_notional - 505.0).abs() < f64::EPSILON);
}

#[test]
fn market_order_without_reference_price_is_blocked() {
    let engine = RiskEngine::new(RiskConfig::default());
    let mut intent = market_intent();
    intent.price = None;
    let decision = engine.check_order(&intent, 0);
    assert!(!decision.allowed);
    assert!(decision
        .reasons
        .contains(&RiskBlockReason::NonPositivePrice));
}

#[test]
fn reduce_only_unwind_allows_missing_reference_price() {
    let engine = RiskEngine::new(RiskConfig::default());
    let mut intent = market_intent();
    intent.price = None;

    let decision = engine.check_unwind(&intent);

    assert!(decision.allowed, "decision was: {:?}", decision.reasons);
    assert_eq!(decision.computed_notional, 0.0);
}

#[test]
fn reduce_only_unwind_checks_max_notional_when_price_is_known() {
    let engine = RiskEngine::new(RiskConfig {
        max_order_notional: 100.0,
        ..RiskConfig::default()
    });
    let decision = engine.check_unwind(&market_intent());

    assert!(!decision.allowed);
    assert!(decision
        .reasons
        .contains(&RiskBlockReason::MaxOrderNotionalExceeded));
}

#[test]
fn reduce_only_unwind_uses_normalized_symbol_whitelist() {
    let engine = RiskEngine::new(RiskConfig {
        max_order_notional: 1_000.0,
        allowed_symbols: BTreeSet::from([" solusdt ".to_owned()]),
        ..RiskConfig::default()
    });
    let mut intent = market_intent();
    intent.symbol = "SOLUSDT".to_owned();

    let decision = engine.check_unwind(&intent);

    assert!(decision.allowed, "decision was: {:?}", decision.reasons);
}

#[test]
fn protected_position_blocks_reduce_only_unwind() {
    let engine = RiskEngine::new(RiskConfig {
        max_order_notional: 1_000.0,
        protected_positions: vec![protected_btc_position()],
        ..RiskConfig::default()
    });

    let decision = engine.check_unwind(&market_intent());

    assert!(!decision.allowed);
    assert!(decision
        .reasons
        .contains(&RiskBlockReason::ProtectedPosition));
}

#[test]
fn non_finite_market_price_is_blocked() {
    let engine = RiskEngine::new(RiskConfig::default());
    let mut intent = market_intent();
    intent.price = Some(f64::INFINITY);

    let decision = engine.check_order(&intent, 0);

    assert!(!decision.allowed);
    assert!(decision.computed_notional.is_infinite());
    assert!(decision
        .reasons
        .contains(&RiskBlockReason::NonPositivePrice));
}

#[test]
fn hedge_check_blocks_imbalanced_legs() {
    let engine = RiskEngine::new(RiskConfig::default());
    let long = limit_intent();
    let mut short = limit_intent();
    short.id = "short".into();
    short.client_order_id = "client-short".into();
    short.quantity = 0.005;

    let (long_decision, short_decision) = engine.check_hedge(&long, &short, 0);

    assert!(!long_decision.allowed);
    assert!(!short_decision.allowed);
    assert!(long_decision
        .reasons
        .contains(&RiskBlockReason::HedgeImbalanceExceeded));
    assert!(short_decision
        .reasons
        .contains(&RiskBlockReason::HedgeImbalanceExceeded));
    let long_evidence = long_decision
        .evidence
        .iter()
        .find(|item| item.code == RiskBlockReason::HedgeImbalanceExceeded)
        .expect("long imbalance evidence");
    let short_evidence = short_decision
        .evidence
        .iter()
        .find(|item| item.code == RiskBlockReason::HedgeImbalanceExceeded)
        .expect("short imbalance evidence");

    assert_eq!(long_evidence.field, "hedge_imbalance_ratio");
    assert_eq!(short_evidence.field, "hedge_imbalance_ratio");
    assert_eq!(long_evidence.limit, Some(serde_json::json!(0.01)));
    assert!(long_evidence
        .actual
        .as_ref()
        .is_some_and(|actual| actual["longQuantity"] == 0.01));
}

#[test]
fn hedge_check_allows_balanced_legs() {
    let engine = RiskEngine::new(RiskConfig::default());
    let long = limit_intent();
    let mut short = limit_intent();
    short.id = "short".into();
    short.client_order_id = "client-short".into();
    short.side = OrderSide::Sell;

    let (long_decision, short_decision) = engine.check_hedge(&long, &short, 0);

    assert!(long_decision.allowed, "{:?}", long_decision.reasons);
    assert!(short_decision.allowed, "{:?}", short_decision.reasons);
}

#[test]
fn hedge_check_allows_equal_base_quantity_at_different_prices() {
    let engine = RiskEngine::new(RiskConfig::default());
    let long = limit_intent();
    let mut short = limit_intent();
    short.id = "short".into();
    short.client_order_id = "client-short".into();
    short.side = OrderSide::Sell;
    short.price = Some(55_000.0);

    let (long_decision, short_decision) = engine.check_hedge(&long, &short, 0);

    assert!(long_decision.allowed, "{:?}", long_decision.reasons);
    assert!(short_decision.allowed, "{:?}", short_decision.reasons);
    assert_ne!(
        long_decision.computed_notional,
        short_decision.computed_notional
    );
}
