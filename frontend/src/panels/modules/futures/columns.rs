use super::data::StrategyFilter;
use shared_types::StrategyKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum ColumnId {
    StrategyKind,
    Symbol,
    LongLeg,
    ShortLeg,
    NetBasisBps,
    GrossOneCycleBps,
    OneCycleNetBps,
    RoundTripCostBps,
    PredictedFunding,
    CostBreakeven,
    IndexComposition,
    Action,
    FundingCyclePercentile,
    BorrowCost,
    FundingAlignment,
    FundingCapDistance,
    MinHold,
    SettlementCountdown,
}

pub(super) const DEFAULT_VISIBLE: &[ColumnId] = &[
    ColumnId::Symbol,
    ColumnId::LongLeg,
    ColumnId::ShortLeg,
    ColumnId::NetBasisBps,
    ColumnId::GrossOneCycleBps,
    ColumnId::OneCycleNetBps,
    ColumnId::RoundTripCostBps,
    ColumnId::PredictedFunding,
    ColumnId::CostBreakeven,
    ColumnId::Action,
];

pub(super) const EXPANDABLE: &[ColumnId] = &[
    ColumnId::FundingCyclePercentile,
    ColumnId::IndexComposition,
    ColumnId::BorrowCost,
    ColumnId::FundingAlignment,
    ColumnId::FundingCapDistance,
    ColumnId::MinHold,
    ColumnId::SettlementCountdown,
];

impl ColumnId {
    pub(super) fn label(self) -> &'static str {
        match self {
            ColumnId::StrategyKind => "策略",
            ColumnId::Symbol => "品种",
            ColumnId::LongLeg => "做多腿",
            ColumnId::ShortLeg => "做空腿",
            ColumnId::NetBasisBps => "扫描净边际",
            ColumnId::GrossOneCycleBps => "毛边际",
            ColumnId::OneCycleNetBps => "费后净边际",
            ColumnId::RoundTripCostBps => "完整成本",
            ColumnId::PredictedFunding => "本次 Funding",
            ColumnId::CostBreakeven => "兑现条件",
            ColumnId::IndexComposition => "成分",
            ColumnId::Action => "动作",
            ColumnId::FundingCyclePercentile => "近期周期",
            ColumnId::BorrowCost => "借币成本",
            ColumnId::FundingAlignment => "窗口偏移",
            ColumnId::FundingCapDistance => "距上限",
            ColumnId::MinHold => "最短持有",
            ColumnId::SettlementCountdown => "结算倒计时",
        }
    }

    pub(super) fn width_class(self) -> &'static str {
        match self {
            ColumnId::StrategyKind => "futures-col-strategy",
            ColumnId::Symbol => "futures-col-symbol",
            ColumnId::LongLeg | ColumnId::ShortLeg => "futures-col-leg",
            ColumnId::RoundTripCostBps => "futures-col-cost",
            ColumnId::PredictedFunding => "futures-col-funding",
            ColumnId::CostBreakeven => "futures-col-breakeven",
            ColumnId::IndexComposition => "futures-col-composition",
            ColumnId::Action => "futures-col-action",
            ColumnId::NetBasisBps | ColumnId::GrossOneCycleBps | ColumnId::OneCycleNetBps => {
                "futures-col-metric"
            }
            ColumnId::FundingCyclePercentile
            | ColumnId::BorrowCost
            | ColumnId::FundingAlignment
            | ColumnId::FundingCapDistance
            | ColumnId::MinHold
            | ColumnId::SettlementCountdown => "futures-col-compact",
        }
    }

    pub(super) fn cell_class(self) -> &'static str {
        match self {
            ColumnId::StrategyKind => "futures-cell-text futures-strategy-cell",
            ColumnId::Action => "futures-cell-action",
            ColumnId::Symbol
            | ColumnId::LongLeg
            | ColumnId::ShortLeg
            | ColumnId::IndexComposition => "futures-cell-text",
            ColumnId::NetBasisBps
            | ColumnId::GrossOneCycleBps
            | ColumnId::OneCycleNetBps
            | ColumnId::RoundTripCostBps
            | ColumnId::PredictedFunding
            | ColumnId::CostBreakeven
            | ColumnId::FundingCyclePercentile
            | ColumnId::BorrowCost
            | ColumnId::FundingAlignment
            | ColumnId::FundingCapDistance
            | ColumnId::MinHold
            | ColumnId::SettlementCountdown => "futures-cell-number",
        }
    }
}

pub(super) fn default_visible_for_strategy(strategy: StrategyFilter) -> Vec<ColumnId> {
    if strategy == StrategyFilter::PerpCross {
        return vec![
            ColumnId::Symbol,
            ColumnId::LongLeg,
            ColumnId::ShortLeg,
            ColumnId::GrossOneCycleBps,
            ColumnId::RoundTripCostBps,
            ColumnId::OneCycleNetBps,
            ColumnId::CostBreakeven,
            ColumnId::Action,
        ];
    }
    if matches!(
        strategy,
        StrategyFilter::PerpPriceSpread | StrategyFilter::SpotCross
    ) {
        let mut columns = vec![
            ColumnId::Symbol,
            ColumnId::LongLeg,
            ColumnId::ShortLeg,
            ColumnId::GrossOneCycleBps,
            ColumnId::RoundTripCostBps,
            ColumnId::OneCycleNetBps,
            ColumnId::CostBreakeven,
        ];
        columns.push(ColumnId::Action);
        return columns;
    }
    if matches!(
        strategy,
        StrategyFilter::SpotPerp | StrategyFilter::CrossSpotPerp
    ) {
        return vec![
            ColumnId::Symbol,
            ColumnId::LongLeg,
            ColumnId::ShortLeg,
            ColumnId::GrossOneCycleBps,
            ColumnId::RoundTripCostBps,
            ColumnId::OneCycleNetBps,
            ColumnId::CostBreakeven,
            ColumnId::Action,
        ];
    }
    DEFAULT_VISIBLE.to_vec()
}

pub(super) fn expandable_for_strategy(strategy: Option<StrategyKind>) -> &'static [ColumnId] {
    const FUNDING: &[ColumnId] = &[
        ColumnId::FundingCyclePercentile,
        ColumnId::IndexComposition,
        ColumnId::FundingAlignment,
        ColumnId::FundingCapDistance,
        ColumnId::MinHold,
        ColumnId::SettlementCountdown,
    ];
    const SPOT_FUNDING: &[ColumnId] = &[
        ColumnId::FundingCyclePercentile,
        ColumnId::IndexComposition,
        ColumnId::BorrowCost,
        ColumnId::FundingAlignment,
        ColumnId::FundingCapDistance,
        ColumnId::MinHold,
        ColumnId::SettlementCountdown,
    ];
    const PRICE_SPREAD: &[ColumnId] = &[ColumnId::IndexComposition];

    match strategy {
        Some(StrategyKind::PerpCross) => FUNDING,
        Some(StrategyKind::SpotPerp | StrategyKind::CrossSpotPerp) => SPOT_FUNDING,
        Some(StrategyKind::PerpPriceSpread | StrategyKind::SpotCross) => PRICE_SPREAD,
        _ => EXPANDABLE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expandable_columns_do_not_reintroduce_non_p0_product_copy() {
        let labels: Vec<_> = EXPANDABLE.iter().map(|col| col.label()).collect();

        assert!(!labels.contains(&"标的事件"));
        assert!(!labels.contains(&"链"));
        assert!(!labels.contains(&"池 / 标的"));
        assert!(!labels.iter().any(|label| label.contains("90d")));
        assert!(!labels.iter().any(|label| label.contains("90D")));
    }

    #[test]
    fn table_layout_classes_follow_column_semantics() {
        assert_eq!(ColumnId::LongLeg.width_class(), "futures-col-leg");
        assert_eq!(ColumnId::OneCycleNetBps.width_class(), "futures-col-metric");
        assert_eq!(ColumnId::NetBasisBps.cell_class(), "futures-cell-number");
        assert_eq!(ColumnId::IndexComposition.cell_class(), "futures-cell-text");
        assert_eq!(ColumnId::Action.cell_class(), "futures-cell-action");
    }

    #[test]
    fn one_shot_spreads_hide_funding_and_annualized_columns() {
        for strategy in [StrategyFilter::PerpPriceSpread, StrategyFilter::SpotCross] {
            let columns = default_visible_for_strategy(strategy);
            assert!(columns.contains(&ColumnId::GrossOneCycleBps));
            assert!(columns.contains(&ColumnId::RoundTripCostBps));
            assert!(columns.contains(&ColumnId::OneCycleNetBps));
            assert!(!columns.contains(&ColumnId::NetBasisBps));
            assert!(!columns.contains(&ColumnId::PredictedFunding));
            assert_eq!(columns.last(), Some(&ColumnId::Action));
        }
    }

    #[test]
    fn perp_cross_shows_gross_cost_and_net_without_duplicate_funding_columns() {
        let columns = default_visible_for_strategy(StrategyFilter::PerpCross);

        assert!(columns.contains(&ColumnId::GrossOneCycleBps));
        assert!(columns.contains(&ColumnId::RoundTripCostBps));
        assert!(columns.contains(&ColumnId::OneCycleNetBps));
        assert!(!columns.contains(&ColumnId::NetBasisBps));
        assert!(!columns.contains(&ColumnId::PredictedFunding));
    }

    #[test]
    fn carry_strategies_prioritize_verified_single_cycle_profit() {
        for strategy in [StrategyFilter::SpotPerp, StrategyFilter::CrossSpotPerp] {
            let columns = default_visible_for_strategy(strategy);
            assert!(columns.contains(&ColumnId::GrossOneCycleBps));
            assert!(columns.contains(&ColumnId::RoundTripCostBps));
            assert!(columns.contains(&ColumnId::OneCycleNetBps));
            assert!(!columns.contains(&ColumnId::NetBasisBps));
        }
    }

    #[test]
    fn active_strategy_is_not_repeated_in_every_default_row() {
        for strategy in [
            StrategyFilter::PerpCross,
            StrategyFilter::PerpPriceSpread,
            StrategyFilter::SpotPerp,
            StrategyFilter::CrossSpotPerp,
            StrategyFilter::SpotCross,
        ] {
            let columns = default_visible_for_strategy(strategy);
            assert!(!columns.contains(&ColumnId::StrategyKind));
            assert!(!columns.contains(&ColumnId::IndexComposition));
        }
    }
}
