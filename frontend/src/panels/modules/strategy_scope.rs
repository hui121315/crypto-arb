#[cfg(test)]
use shared_types::{is_p0_executable_strategy, ArbitrageOpportunityDto};
use shared_types::{StrategyKind, P0_EXECUTABLE_STRATEGY_KINDS};

pub(crate) const P0_STRATEGY_KINDS: [StrategyKind; 5] = P0_EXECUTABLE_STRATEGY_KINDS;

#[cfg(test)]
pub(crate) fn p0_strategy_dtos(
    mut rows: Vec<ArbitrageOpportunityDto>,
) -> Vec<ArbitrageOpportunityDto> {
    rows.retain(|row| row.strategy_kind.is_some_and(is_p0_executable_strategy));
    rows
}

pub(crate) const fn p0_strategy_label(kind: StrategyKind) -> Option<&'static str> {
    match kind {
        StrategyKind::PerpCross => Some("永续跨所"),
        StrategyKind::PerpPriceSpread => Some("永续价差"),
        StrategyKind::SpotPerp => Some("现货-永续"),
        StrategyKind::CrossSpotPerp => Some("跨所期现"),
        StrategyKind::SpotCross => Some("现货跨所"),
        _ => None,
    }
}

pub(crate) const fn p0_strategy_label_or_unavailable(kind: Option<StrategyKind>) -> &'static str {
    match kind {
        Some(value) => match p0_strategy_label(value) {
            Some(label) => label,
            None => "未开放策略",
        },
        None => "未开放策略",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn p0_labels_expose_all_five_user_facing_strategies() {
        assert_eq!(
            p0_strategy_label_or_unavailable(Some(StrategyKind::PerpCross)),
            "永续跨所"
        );
        assert_eq!(
            p0_strategy_label_or_unavailable(Some(StrategyKind::SpotPerp)),
            "现货-永续"
        );
        assert_eq!(
            p0_strategy_label_or_unavailable(Some(StrategyKind::CrossSpotPerp)),
            "跨所期现"
        );
        assert_eq!(
            p0_strategy_label_or_unavailable(Some(StrategyKind::PerpPriceSpread)),
            "永续价差"
        );
        assert_eq!(
            p0_strategy_label_or_unavailable(Some(StrategyKind::SpotCross)),
            "现货跨所"
        );
        assert_eq!(p0_strategy_label_or_unavailable(None), "未开放策略");
    }
}
