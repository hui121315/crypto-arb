use shared_types::{ArbitrageOpportunityDto, StrategyKind};
use std::collections::HashSet;

pub(super) fn apply_limit(
    list: &mut Vec<ArbitrageOpportunityDto>,
    limit: usize,
    kinds: Option<&[StrategyKind]>,
) {
    if list.len() <= limit {
        return;
    }
    match kinds {
        Some(kinds) if !kinds.is_empty() => apply_strategy_limit(list, limit, kinds),
        _ => list.truncate(limit),
    }
}

fn apply_strategy_limit(
    list: &mut Vec<ArbitrageOpportunityDto>,
    limit: usize,
    kinds: &[StrategyKind],
) {
    let quota = per_strategy_quota(limit, kinds.len());
    let mut seen = HashSet::with_capacity(limit);
    let mut out = Vec::with_capacity(limit);
    for kind in kinds {
        if out.len() >= limit {
            break;
        }
        let remaining = limit - out.len();
        take_kind_rows(list, *kind, quota.min(remaining), &mut seen, &mut out);
    }
    fill_limit_rows(list, limit, &mut seen, &mut out);
    *list = out;
}

fn per_strategy_quota(limit: usize, strategy_count: usize) -> usize {
    (limit / strategy_count.max(1)).max(1)
}

fn take_kind_rows<'a>(
    list: &'a [ArbitrageOpportunityDto],
    kind: StrategyKind,
    max_rows: usize,
    seen: &mut HashSet<&'a str>,
    out: &mut Vec<ArbitrageOpportunityDto>,
) {
    for row in list
        .iter()
        .filter(|row| row.strategy_kind == Some(kind))
        .take(max_rows)
    {
        push_once(row, seen, out);
    }
}

fn fill_limit_rows<'a>(
    list: &'a [ArbitrageOpportunityDto],
    limit: usize,
    seen: &mut HashSet<&'a str>,
    out: &mut Vec<ArbitrageOpportunityDto>,
) {
    for row in list {
        if out.len() >= limit {
            return;
        }
        push_once(row, seen, out);
    }
}

fn push_once<'a>(
    row: &'a ArbitrageOpportunityDto,
    seen: &mut HashSet<&'a str>,
    out: &mut Vec<ArbitrageOpportunityDto>,
) {
    if !seen.insert(row.id.as_str()) {
        return;
    }
    out.push(row.clone());
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::{ArbitrageType, Recommendation, RiskLevel, StrategyCategory};

    #[test]
    fn strategy_limit_keeps_late_requested_rows() {
        let mut rows = (0..40)
            .map(|i| row(format!("perp-{i}"), StrategyKind::PerpCross))
            .chain([row("spot-cross-1".into(), StrategyKind::SpotCross)])
            .collect();
        apply_limit(
            &mut rows,
            10,
            Some(&[StrategyKind::PerpCross, StrategyKind::SpotCross]),
        );
        assert!(rows
            .iter()
            .any(|row| row.strategy_kind == Some(StrategyKind::SpotCross)));
    }

    #[test]
    fn strategy_limit_uses_equal_quota_then_fills() {
        let mut rows: Vec<_> = [
            StrategyKind::PerpCross,
            StrategyKind::SpotPerp,
            StrategyKind::CrossSpotPerp,
            StrategyKind::SpotCross,
        ]
        .into_iter()
        .flat_map(|kind| (0..80).map(move |i| row(format!("{kind:?}-{i}"), kind)))
        .collect();

        apply_limit(
            &mut rows,
            200,
            Some(&[
                StrategyKind::PerpCross,
                StrategyKind::SpotPerp,
                StrategyKind::CrossSpotPerp,
                StrategyKind::SpotCross,
            ]),
        );

        assert_eq!(rows.len(), 200);
        for kind in [
            StrategyKind::PerpCross,
            StrategyKind::SpotPerp,
            StrategyKind::CrossSpotPerp,
            StrategyKind::SpotCross,
        ] {
            let count = rows
                .iter()
                .filter(|row| row.strategy_kind == Some(kind))
                .count();
            assert_eq!(count, 50);
        }
    }

    fn row(id: String, kind: StrategyKind) -> ArbitrageOpportunityDto {
        ArbitrageOpportunityDto {
            id,
            symbol: "BTC".into(),
            arb_type: ArbitrageType::CrossExchange,
            type_label: kind.label_zh().into(),
            long_exchange: "a".into(),
            short_exchange: "b".into(),
            spread_8h: 0.0,
            long_rate_8h: 0.0,
            short_rate_8h: 0.0,
            long_rate: 0.0,
            short_rate: 0.0,
            single_yield: 0.0,
            net_single_yield: 0.0,
            raw_single_yield: 0.0,
            settlement_interval: 8,
            risk_adjusted_yield: 0.0,
            trading_cost_rate: 0.0,
            min_holding_periods: 1,
            risk_level: RiskLevel::Low,
            volatility: 0.0,
            sharpe_ratio: 0.0,
            score: 0.0,
            score_breakdown: None,
            ranking_key: None,
            recommendation: Recommendation::Hold,
            optimal_position: 0.0,
            max_position: 0.0,
            liquidity_score: 0.0,
            volume_24h: 0.0,
            long_volume_24h: 0.0,
            short_volume_24h: 0.0,
            data_source: "test".into(),
            confidence: 0.0,
            updated_at: chrono::Utc::now(),
            long_funding_interval: 8,
            short_funding_interval: 8,
            settlement_time_diff: false,
            strategy_description: String::new(),
            long_action: String::new(),
            short_action: String::new(),
            long_next_funding_time: 0,
            short_next_funding_time: 0,
            time_to_settlement_ms: 0,
            is_snipe_ready: false,
            long_price: None,
            short_price: None,
            long_leg_market_evidence: None,
            short_leg_market_evidence: None,
            quote_conversions: Vec::new(),
            price_deviation: None,
            basis_spread: None,
            basis_annual_cost: None,
            risk_warnings: Vec::new(),
            execution_eligible: false,
            execution_blockers: Vec::new(),
            execution_cost: None,
            index_composition: None,
            strategy_kind: Some(kind),
            strategy_category: Some(StrategyCategory::Futures),
            spot_leg_mode: None,
            basis_bps: None,
            annualized_funding_bps: None,
            triangular_path: None,
            onchain_metadata: None,
            predicted_next_funding: None,
            funding_diff_window: None,
            funding_diff_windows: Vec::new(),
            borrow_cost_bps_per_day: None,
            funding_window_alignment_minutes: None,
            funding_cap_distance_bps: None,
            min_hold_hours: None,
            settlement_countdown_seconds: None,
        }
    }
}
