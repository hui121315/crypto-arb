//! 策略扫描、Paper 与 Live 执行能力边界。

use crate::strategy::StrategyKind;

pub const PHASE2_STRATEGY_KINDS: [StrategyKind; 9] = [
    StrategyKind::PerpCross,
    StrategyKind::PerpPriceSpread,
    StrategyKind::SpotPerp,
    StrategyKind::CrossSpotPerp,
    StrategyKind::SpotCross,
    StrategyKind::Triangular,
    StrategyKind::FundingCarry,
    StrategyKind::CashAndCarry,
    StrategyKind::OptionsPerpBasis,
];

pub const P0_EXECUTABLE_STRATEGY_KINDS: [StrategyKind; 5] = [
    StrategyKind::PerpCross,
    StrategyKind::PerpPriceSpread,
    StrategyKind::SpotPerp,
    StrategyKind::CrossSpotPerp,
    StrategyKind::SpotCross,
];

/// Live execution remains fail-closed unless the strategy has a ticket-bound positive cash-flow
/// proof. `PerpPriceSpread` depends on future convergence and is therefore scanner/Paper only.
pub const LIVE_EXECUTABLE_STRATEGY_KINDS: [StrategyKind; 1] = [StrategyKind::PerpCross];

/// Trading actions whose costs belong to one strategy opportunity.
///
/// Most strategies open two legs and later close both legs. `SpotCross` realizes its spread in
/// the initial buy/sell pair, then restores inventory through the separately proven transfer loop;
/// charging another synthetic buy/sell pair would double-count trading costs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrategyExecutionCycle {
    PairedOpenClose,
    PairedOpenRebalance,
}

const fn contains_strategy(kinds: &[StrategyKind], kind: StrategyKind) -> bool {
    let target = kind as u8;
    let mut index = 0;
    while index < kinds.len() {
        if kinds[index] as u8 == target {
            return true;
        }
        index += 1;
    }
    false
}

#[must_use]
pub const fn is_phase2_strategy(kind: StrategyKind) -> bool {
    contains_strategy(&PHASE2_STRATEGY_KINDS, kind)
}

#[must_use]
pub const fn is_p0_executable_strategy(kind: StrategyKind) -> bool {
    contains_strategy(&P0_EXECUTABLE_STRATEGY_KINDS, kind)
}

#[must_use]
pub const fn is_live_executable_strategy(kind: StrategyKind) -> bool {
    contains_strategy(&LIVE_EXECUTABLE_STRATEGY_KINDS, kind)
}

#[must_use]
pub const fn is_diagnostic_only_strategy(kind: StrategyKind) -> bool {
    matches!(kind, StrategyKind::OptionsPerpBasis)
}

#[must_use]
pub const fn strategy_execution_cycle(kind: Option<StrategyKind>) -> StrategyExecutionCycle {
    match kind {
        Some(StrategyKind::SpotCross) => StrategyExecutionCycle::PairedOpenRebalance,
        _ => StrategyExecutionCycle::PairedOpenClose,
    }
}
