use super::*;
use pretty_assertions::assert_eq;

#[test]
fn long_call_factory_sets_kind_and_legs() {
    let s = long_call(30000.0, 30000.0, 1.0, 1500.0);
    assert_eq!(s.kind, StrategyKind::LongCall);
    assert_eq!(s.legs.len(), 1);
    assert_eq!(s.legs[0].side, LegSide::Long);
}

#[test]
fn bull_call_spread_max_profit_correct() {
    let s = bull_call_spread(100.0, 100.0, 110.0, 1.0, 5.0, 2.0);
    assert!((s.pnl_at_expiry(120.0) - 7.0).abs() < 1e-12);
    assert!((s.pnl_at_expiry(80.0) - (-3.0)).abs() < 1e-12);
}

#[test]
fn iron_condor_max_profit_at_middle_zone() {
    let s = iron_condor(IronCondorSpec {
        spot: 100.0,
        qty: 1.0,
        strikes: IronCondorStrikes {
            put_long: 80.0,
            put_short: 90.0,
            call_short: 110.0,
            call_long: 120.0,
        },
        premiums: IronCondorPremiums {
            put_long: 0.5,
            put_short: 2.0,
            call_short: 2.0,
            call_long: 0.5,
        },
    });
    assert!((s.net_premium() - (-3.0)).abs() < 1e-12);
    assert!((s.pnl_at_expiry(100.0) - 3.0).abs() < 1e-12);
    assert!((s.pnl_at_expiry(95.0) - 3.0).abs() < 1e-12);
    let pnl_extreme = s.pnl_at_expiry(70.0);
    assert!(
        (pnl_extreme - (-7.0)).abs() < 1e-12,
        "iron condor max loss should be 7, got {pnl_extreme}"
    );
}

#[test]
fn butterfly_max_profit_at_middle_strike() {
    let s = long_call_butterfly(LongCallButterflySpec {
        spot: 100.0,
        qty: 1.0,
        strikes: ButterflyStrikes {
            low: 95.0,
            mid: 100.0,
            high: 105.0,
        },
        premiums: ButterflyPremiums {
            low: 6.0,
            mid: 3.0,
            high: 1.0,
        },
    });
    assert!((s.net_premium() - 1.0).abs() < 1e-12);
    assert!((s.pnl_at_expiry(100.0) - 4.0).abs() < 1e-12);
    assert!((s.pnl_at_expiry(120.0) - (-1.0)).abs() < 1e-12);
}

#[test]
fn long_strangle_legs_correctly_built() {
    let s = long_strangle(100.0, 110.0, 90.0, 1.0, 2.0, 2.0);
    assert_eq!(s.legs.len(), 2);
    let call_leg = s
        .legs
        .iter()
        .find(|l| l.option_type == OptionType::Call)
        .unwrap();
    let put_leg = s
        .legs
        .iter()
        .find(|l| l.option_type == OptionType::Put)
        .unwrap();
    assert_eq!(call_leg.strike, 110.0);
    assert_eq!(put_leg.strike, 90.0);
}
