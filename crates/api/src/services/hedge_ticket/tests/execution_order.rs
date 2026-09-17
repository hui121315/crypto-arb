use super::*;

fn quote(role: HedgeLegRole, depth_usd_5bps: Option<f64>) -> HedgeLegQuote {
    let mut quote = empty_leg();
    quote.role = role;
    quote.side = match role {
        HedgeLegRole::Long => OrderSide::Buy,
        HedgeLegRole::Short => OrderSide::Sell,
    };
    quote.depth_usd_5bps = depth_usd_5bps;
    quote
}

#[test]
fn constrained_short_leg_executes_before_deeper_long_hedge() {
    let long = quote(HedgeLegRole::Long, Some(5_000.0));
    let short = quote(HedgeLegRole::Short, Some(750.0));

    let order = execution_order_for_quotes(&long, &short);

    assert_eq!(order.first, HedgeLegRole::Short);
    assert_eq!(order.second, HedgeLegRole::Long);
}

#[test]
fn constrained_long_leg_executes_before_deeper_short_hedge() {
    let long = quote(HedgeLegRole::Long, Some(500.0));
    let short = quote(HedgeLegRole::Short, Some(4_000.0));

    let order = execution_order_for_quotes(&long, &short);

    assert_eq!(order.first, HedgeLegRole::Long);
    assert_eq!(order.second, HedgeLegRole::Short);
}

#[test]
fn missing_depth_blocks_order_evidence_and_keeps_legacy_fallback() {
    let long = quote(HedgeLegRole::Long, None);
    let short = quote(HedgeLegRole::Short, Some(4_000.0));

    let order = execution_order_for_quotes(&long, &short);
    let guard = execution_order_guard(&long, &short);

    assert_eq!(order.first, HedgeLegRole::Long);
    assert!(!guard.passed);
    assert_eq!(guard.key, "execution_order");
}
