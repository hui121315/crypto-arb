//! Iron Condor 策略工厂与输入规格。

use super::{leg, LegSide, OptionType, Strategy, StrategyKind};

/// Iron Condor：卖 OTM Put 价差 + 卖 OTM Call 价差。
///
/// 行权价从低到高：`put_long < put_short < call_short < call_long`。
#[derive(Debug, Clone, Copy)]
pub struct IronCondorSpec {
    pub spot: f64,
    pub qty: f64,
    pub strikes: IronCondorStrikes,
    pub premiums: IronCondorPremiums,
}

#[derive(Debug, Clone, Copy)]
pub struct IronCondorStrikes {
    pub put_long: f64,
    pub put_short: f64,
    pub call_short: f64,
    pub call_long: f64,
}

#[derive(Debug, Clone, Copy)]
pub struct IronCondorPremiums {
    pub put_long: f64,
    pub put_short: f64,
    pub call_short: f64,
    pub call_long: f64,
}

pub fn iron_condor(spec: IronCondorSpec) -> Strategy {
    let strikes = spec.strikes;
    let premiums = spec.premiums;
    debug_assert!(
        strikes.put_long < strikes.put_short
            && strikes.put_short < strikes.call_short
            && strikes.call_short < strikes.call_long,
        "iron condor strikes must be ordered"
    );
    Strategy::new(
        StrategyKind::IronCondor,
        "Iron Condor",
        vec![
            leg(
                LegSide::Long,
                OptionType::Put,
                strikes.put_long,
                spec.qty,
                premiums.put_long,
            ),
            leg(
                LegSide::Short,
                OptionType::Put,
                strikes.put_short,
                spec.qty,
                premiums.put_short,
            ),
            leg(
                LegSide::Short,
                OptionType::Call,
                strikes.call_short,
                spec.qty,
                premiums.call_short,
            ),
            leg(
                LegSide::Long,
                OptionType::Call,
                strikes.call_long,
                spec.qty,
                premiums.call_long,
            ),
        ],
        spec.spot,
    )
}
