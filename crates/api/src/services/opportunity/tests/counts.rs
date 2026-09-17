use super::*;

pub(crate) struct OpportunityCounts {
    pub executable_count: usize,
    pub strategy_counts: std::collections::HashMap<StrategyKind, usize>,
    pub executable_strategy_counts: std::collections::HashMap<StrategyKind, usize>,
}

pub(crate) fn counts(rows: &[ArbitrageOpportunityDto]) -> OpportunityCounts {
    let breakdown = super::super::builders::p0_count_breakdown(rows);
    OpportunityCounts {
        executable_count: breakdown.executable_count,
        strategy_counts: breakdown.strategy_counts,
        executable_strategy_counts: breakdown.executable_strategy_counts,
    }
}
