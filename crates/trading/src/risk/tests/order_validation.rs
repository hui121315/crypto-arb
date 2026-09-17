use super::limit_intent;
use crate::risk::{RiskConfig, RiskEngine};
use shared_types::RiskBlockReason;

#[test]
fn default_allows_safe_limit_order() {
    let engine = RiskEngine::new(RiskConfig::default());
    let decision = engine.check_order(&limit_intent(), 0);
    assert!(decision.allowed, "decision was: {:?}", decision.reasons);
    assert!((decision.computed_notional - 500.0).abs() < f64::EPSILON);
}

#[test]
fn non_positive_quantity_is_blocked() {
    let engine = RiskEngine::new(RiskConfig::default());
    let mut intent = limit_intent();
    intent.quantity = 0.0;
    let decision = engine.check_order(&intent, 0);
    assert!(!decision.allowed);
    assert!(decision
        .reasons
        .contains(&RiskBlockReason::NonPositiveQuantity));
}

#[test]
fn non_finite_quantity_is_blocked_and_notional_is_infinite() {
    let engine = RiskEngine::new(RiskConfig::default());
    let mut intent = limit_intent();
    intent.quantity = f64::NAN;

    let decision = engine.check_order(&intent, 0);

    assert!(!decision.allowed);
    assert!(decision.computed_notional.is_infinite());
    assert!(decision
        .reasons
        .contains(&RiskBlockReason::NonPositiveQuantity));
    let evidence = decision
        .evidence
        .iter()
        .find(|item| item.code == RiskBlockReason::NonPositiveQuantity)
        .expect("quantity evidence");
    assert_eq!(evidence.actual, Some(serde_json::json!("NaN")));
}

#[test]
fn missing_limit_price_is_blocked() {
    let engine = RiskEngine::new(RiskConfig::default());
    let mut intent = limit_intent();
    intent.price = None;
    let decision = engine.check_order(&intent, 0);
    assert!(!decision.allowed);
    assert!(decision
        .reasons
        .contains(&RiskBlockReason::MissingLimitPrice));
}

#[test]
fn non_positive_limit_price_is_blocked() {
    let engine = RiskEngine::new(RiskConfig::default());
    let mut intent = limit_intent();
    intent.price = Some(-1.0);
    let decision = engine.check_order(&intent, 0);
    assert!(!decision.allowed);
    assert!(decision
        .reasons
        .contains(&RiskBlockReason::NonPositivePrice));
}

#[test]
fn max_order_notional_exceeded_is_blocked() {
    let engine = RiskEngine::new(RiskConfig {
        max_order_notional: 100.0,
        ..RiskConfig::default()
    });
    let decision = engine.check_order(&limit_intent(), 0);
    assert!(!decision.allowed);
    assert!(decision
        .reasons
        .contains(&RiskBlockReason::MaxOrderNotionalExceeded));
    let evidence = decision
        .evidence
        .iter()
        .find(|item| item.code == RiskBlockReason::MaxOrderNotionalExceeded)
        .expect("notional evidence");
    assert_eq!(evidence.field, "computed_notional");
    assert_eq!(evidence.actual, Some(serde_json::json!(500.0)));
    assert_eq!(evidence.limit, Some(serde_json::json!(100.0)));
    assert_eq!(evidence.source, "trading.risk_engine");
}

#[test]
fn max_open_orders_exceeded_is_blocked() {
    let engine = RiskEngine::new(RiskConfig {
        max_open_orders: 2,
        ..RiskConfig::default()
    });
    let decision = engine.check_order(&limit_intent(), 2);
    assert!(!decision.allowed);
    assert!(decision
        .reasons
        .contains(&RiskBlockReason::MaxOpenOrdersExceeded));
    let evidence = decision
        .evidence
        .iter()
        .find(|item| item.code == RiskBlockReason::MaxOpenOrdersExceeded)
        .expect("open order evidence");
    assert_eq!(evidence.field, "open_orders_before_submit");
    assert_eq!(evidence.actual, Some(serde_json::json!(2)));
    assert!(evidence
        .limit
        .as_ref()
        .is_some_and(|limit| limit["maxOpenOrders"] == 2));
}
