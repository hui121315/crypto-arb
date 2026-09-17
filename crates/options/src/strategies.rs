//! 常见期权策略工厂函数。

#[path = "strategies/iron_condor.rs"]
mod iron_condor;

use crate::strategy::{Leg, LegSide, Strategy, StrategyKind};
use shared_types::OptionType;

pub use iron_condor::{iron_condor, IronCondorPremiums, IronCondorSpec, IronCondorStrikes};

fn leg(side: LegSide, opt: OptionType, strike: f64, qty: f64, premium: f64) -> Leg {
    Leg {
        side,
        option_type: opt,
        strike,
        quantity: qty,
        premium,
    }
}

pub fn long_call(spot: f64, strike: f64, qty: f64, premium: f64) -> Strategy {
    Strategy::new(
        StrategyKind::LongCall,
        "Long Call",
        vec![leg(LegSide::Long, OptionType::Call, strike, qty, premium)],
        spot,
    )
}

pub fn long_put(spot: f64, strike: f64, qty: f64, premium: f64) -> Strategy {
    Strategy::new(
        StrategyKind::LongPut,
        "Long Put",
        vec![leg(LegSide::Long, OptionType::Put, strike, qty, premium)],
        spot,
    )
}

pub fn short_call(spot: f64, strike: f64, qty: f64, premium: f64) -> Strategy {
    Strategy::new(
        StrategyKind::ShortCall,
        "Short Call",
        vec![leg(LegSide::Short, OptionType::Call, strike, qty, premium)],
        spot,
    )
}

pub fn short_put(spot: f64, strike: f64, qty: f64, premium: f64) -> Strategy {
    Strategy::new(
        StrategyKind::ShortPut,
        "Short Put",
        vec![leg(LegSide::Short, OptionType::Put, strike, qty, premium)],
        spot,
    )
}

/// Bull Call Spread：买低行权价 Call + 卖高行权价 Call。
pub fn bull_call_spread(
    spot: f64,
    long_strike: f64,
    short_strike: f64,
    qty: f64,
    long_premium: f64,
    short_premium: f64,
) -> Strategy {
    debug_assert!(
        long_strike < short_strike,
        "long_strike must be < short_strike"
    );
    Strategy::new(
        StrategyKind::BullCallSpread,
        "Bull Call Spread",
        vec![
            leg(
                LegSide::Long,
                OptionType::Call,
                long_strike,
                qty,
                long_premium,
            ),
            leg(
                LegSide::Short,
                OptionType::Call,
                short_strike,
                qty,
                short_premium,
            ),
        ],
        spot,
    )
}

/// Bear Put Spread：买高行权价 Put + 卖低行权价 Put。
pub fn bear_put_spread(
    spot: f64,
    long_strike: f64,
    short_strike: f64,
    qty: f64,
    long_premium: f64,
    short_premium: f64,
) -> Strategy {
    debug_assert!(
        long_strike > short_strike,
        "long_strike must be > short_strike"
    );
    Strategy::new(
        StrategyKind::BearPutSpread,
        "Bear Put Spread",
        vec![
            leg(
                LegSide::Long,
                OptionType::Put,
                long_strike,
                qty,
                long_premium,
            ),
            leg(
                LegSide::Short,
                OptionType::Put,
                short_strike,
                qty,
                short_premium,
            ),
        ],
        spot,
    )
}

/// Long Straddle：买 ATM Call + 买 ATM Put（同行权价）。
pub fn long_straddle(
    spot: f64,
    strike: f64,
    qty: f64,
    call_premium: f64,
    put_premium: f64,
) -> Strategy {
    Strategy::new(
        StrategyKind::LongStraddle,
        "Long Straddle",
        vec![
            leg(LegSide::Long, OptionType::Call, strike, qty, call_premium),
            leg(LegSide::Long, OptionType::Put, strike, qty, put_premium),
        ],
        spot,
    )
}

/// Long Strangle：买 OTM Call + 买 OTM Put（不同行权价）。
pub fn long_strangle(
    spot: f64,
    call_strike: f64,
    put_strike: f64,
    qty: f64,
    call_premium: f64,
    put_premium: f64,
) -> Strategy {
    debug_assert!(call_strike > put_strike, "call_strike must be > put_strike");
    Strategy::new(
        StrategyKind::LongStrangle,
        "Long Strangle",
        vec![
            leg(
                LegSide::Long,
                OptionType::Call,
                call_strike,
                qty,
                call_premium,
            ),
            leg(LegSide::Long, OptionType::Put, put_strike, qty, put_premium),
        ],
        spot,
    )
}

/// Short Straddle：卖 ATM Call + 卖 ATM Put。
pub fn short_straddle(
    spot: f64,
    strike: f64,
    qty: f64,
    call_premium: f64,
    put_premium: f64,
) -> Strategy {
    Strategy::new(
        StrategyKind::ShortStraddle,
        "Short Straddle",
        vec![
            leg(LegSide::Short, OptionType::Call, strike, qty, call_premium),
            leg(LegSide::Short, OptionType::Put, strike, qty, put_premium),
        ],
        spot,
    )
}

/// Long Call Butterfly：买 1 低行权价 Call + 卖 2 中行权价 Call + 买 1 高行权价 Call。
#[derive(Debug, Clone, Copy)]
pub struct LongCallButterflySpec {
    pub spot: f64,
    pub qty: f64,
    pub strikes: ButterflyStrikes,
    pub premiums: ButterflyPremiums,
}

#[derive(Debug, Clone, Copy)]
pub struct ButterflyStrikes {
    pub low: f64,
    pub mid: f64,
    pub high: f64,
}

#[derive(Debug, Clone, Copy)]
pub struct ButterflyPremiums {
    pub low: f64,
    pub mid: f64,
    pub high: f64,
}

pub fn long_call_butterfly(spec: LongCallButterflySpec) -> Strategy {
    let strikes = spec.strikes;
    let premiums = spec.premiums;
    debug_assert!(
        strikes.low < strikes.mid && strikes.mid < strikes.high,
        "butterfly strikes must be ordered"
    );
    Strategy::new(
        StrategyKind::Butterfly,
        "Long Call Butterfly",
        vec![
            leg(
                LegSide::Long,
                OptionType::Call,
                strikes.low,
                spec.qty,
                premiums.low,
            ),
            leg(
                LegSide::Short,
                OptionType::Call,
                strikes.mid,
                spec.qty * 2.0,
                premiums.mid,
            ),
            leg(
                LegSide::Long,
                OptionType::Call,
                strikes.high,
                spec.qty,
                premiums.high,
            ),
        ],
        spec.spot,
    )
}

#[cfg(test)]
mod tests;
