use super::*;
use pretty_assertions::assert_eq;

fn long_call(strike: f64, premium: f64) -> Strategy {
    Strategy::new(
        StrategyKind::LongCall,
        "Long Call",
        vec![Leg {
            side: LegSide::Long,
            option_type: OptionType::Call,
            strike,
            quantity: 1.0,
            premium,
        }],
        strike,
    )
}

#[test]
fn long_call_pnl_below_strike_equals_neg_premium() {
    let s = long_call(100.0, 5.0);
    assert!((s.pnl_at_expiry(80.0) - (-5.0)).abs() < 1e-12);
    assert!((s.pnl_at_expiry(100.0) - (-5.0)).abs() < 1e-12);
}

#[test]
fn long_call_pnl_above_strike_grows_linearly() {
    let s = long_call(100.0, 5.0);
    assert!((s.pnl_at_expiry(110.0) - 5.0).abs() < 1e-12);
    assert!((s.pnl_at_expiry(200.0) - 95.0).abs() < 1e-12);
}

#[test]
fn short_call_pnl_inverted() {
    let strat = Strategy::new(
        StrategyKind::ShortCall,
        "Short Call",
        vec![Leg {
            side: LegSide::Short,
            option_type: OptionType::Call,
            strike: 100.0,
            quantity: 1.0,
            premium: 5.0,
        }],
        100.0,
    );
    assert!((strat.pnl_at_expiry(80.0) - 5.0).abs() < 1e-12);
    assert!((strat.pnl_at_expiry(110.0) - (-5.0)).abs() < 1e-12);
}

#[test]
fn bull_call_spread_pnl() {
    let strat = Strategy::new(
        StrategyKind::BullCallSpread,
        "Bull Call Spread",
        vec![
            Leg {
                side: LegSide::Long,
                option_type: OptionType::Call,
                strike: 100.0,
                quantity: 1.0,
                premium: 5.0,
            },
            Leg {
                side: LegSide::Short,
                option_type: OptionType::Call,
                strike: 110.0,
                quantity: 1.0,
                premium: 2.0,
            },
        ],
        100.0,
    );
    assert!((strat.net_premium() - 3.0).abs() < 1e-12);
    assert!((strat.pnl_at_expiry(90.0) - (-3.0)).abs() < 1e-12);
    assert!((strat.pnl_at_expiry(120.0) - 7.0).abs() < 1e-12);
}

#[test]
fn long_straddle_pnl_at_strike_max_loss() {
    let strat = Strategy::new(
        StrategyKind::LongStraddle,
        "Long Straddle",
        vec![
            Leg {
                side: LegSide::Long,
                option_type: OptionType::Call,
                strike: 100.0,
                quantity: 1.0,
                premium: 5.0,
            },
            Leg {
                side: LegSide::Long,
                option_type: OptionType::Put,
                strike: 100.0,
                quantity: 1.0,
                premium: 4.0,
            },
        ],
        100.0,
    );
    assert!((strat.pnl_at_expiry(100.0) - (-9.0)).abs() < 1e-12);
    assert!((strat.pnl_at_expiry(130.0) - 21.0).abs() < 1e-12);
    assert!((strat.pnl_at_expiry(70.0) - 21.0).abs() < 1e-12);
}

#[test]
fn quantity_scales_pnl() {
    let strat = Strategy::new(
        StrategyKind::LongCall,
        "Long Call x3",
        vec![Leg {
            side: LegSide::Long,
            option_type: OptionType::Call,
            strike: 100.0,
            quantity: 3.0,
            premium: 5.0,
        }],
        100.0,
    );
    assert!((strat.pnl_at_expiry(110.0) - 15.0).abs() < 1e-12);
}

#[test]
fn aggregate_greeks_long_call_matches_single() {
    let s = long_call(30000.0, 1500.0);
    let g = s.aggregate_greeks(30000.0, 30.0 / 365.0, 0.05, 0.6);
    let direct = crate::all_greeks(30000.0, 30000.0, 30.0 / 365.0, 0.05, 0.6, OptionType::Call);
    assert!((g.delta - direct.delta).abs() < 1e-12);
    assert!((g.gamma - direct.gamma).abs() < 1e-12);
}

#[test]
fn aggregate_greeks_short_inverts_delta() {
    let strat = Strategy::new(
        StrategyKind::ShortCall,
        "Short Call",
        vec![Leg {
            side: LegSide::Short,
            option_type: OptionType::Call,
            strike: 30000.0,
            quantity: 1.0,
            premium: 1500.0,
        }],
        30000.0,
    );
    let g = strat.aggregate_greeks(30000.0, 30.0 / 365.0, 0.05, 0.6);
    let direct = crate::all_greeks(30000.0, 30000.0, 30.0 / 365.0, 0.05, 0.6, OptionType::Call);
    assert!((g.delta + direct.delta).abs() < 1e-12);
}

#[test]
fn pnl_curve_n_points_correct() {
    let s = long_call(100.0, 5.0);
    let curve = s.pnl_curve(80.0, 130.0, 6);
    assert_eq!(curve.len(), 6);
    assert!((curve[0].0 - 80.0).abs() < 1e-12);
    assert!((curve[5].0 - 130.0).abs() < 1e-12);
}

#[test]
fn risk_metrics_long_call_finds_breakeven() {
    let strat = long_call(100.0, 5.0);
    let m = strat.risk_metrics(80.0, 200.0, 100);
    assert!(!m.breakeven_points.is_empty());
    let be = m.breakeven_points[0];
    assert!((be - 105.0).abs() < 2.0);
    assert!((m.max_loss - (-5.0)).abs() < 1e-9);
    assert!(m.max_profit > 90.0);
}
