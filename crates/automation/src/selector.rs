use shared_types::{
    is_hedge_preview_ready_at, market_monitor_net_bps_at, ArbitrageOpportunityDto,
    AutomatedArbitrageConfig, StrategyKind, P0_EXECUTABLE_STRATEGY_KINDS,
};
#[cfg(test)]
use shared_types::{DEFERRED_INVENTORY_OR_BORROW_BLOCKER, DEFERRED_SPOT_PERP_TICKET_BLOCKER};
use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap};

#[derive(Debug, Clone)]
pub struct CandidateSelection {
    pub opportunity: ArbitrageOpportunityDto,
    pub reason: String,
}

pub fn select_candidate<'a>(
    rows: impl IntoIterator<Item = &'a ArbitrageOpportunityDto>,
    config: &AutomatedArbitrageConfig,
    active_opportunity_ids: &BTreeSet<String>,
    now_ms: i64,
) -> Option<CandidateSelection> {
    select_candidates(rows, config, active_opportunity_ids, now_ms, 1)
        .into_iter()
        .next()
}

pub fn select_candidates<'a>(
    rows: impl IntoIterator<Item = &'a ArbitrageOpportunityDto>,
    config: &AutomatedArbitrageConfig,
    active_opportunity_ids: &BTreeSet<String>,
    now_ms: i64,
    limit: usize,
) -> Vec<CandidateSelection> {
    let mut candidates = rows
        .into_iter()
        .filter(|row| candidate_ready(row, config, active_opportunity_ids, now_ms))
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| candidate_order(left, right, now_ms));
    candidates.truncate(limit);
    candidates
        .into_iter()
        .cloned()
        .map(|opportunity| CandidateSelection {
            reason: format!(
                "profit-floor-ready strategy={} symbol={} oneCycleNetBps={:.2} worstMarketAgeMs={} previewMinDepthUsd={:.2}",
                opportunity
                    .strategy_kind
                    .map(StrategyKind::as_query_value)
                    .unwrap_or("unknown"),
                opportunity.symbol,
                one_cycle_net_bps(&opportunity),
                candidate_market_age_ms(&opportunity, now_ms),
                config.min_depth_usd
            ),
            opportunity,
        })
        .collect()
}

/// Selects one profitable public-market candidate per P0 strategy for UI/watchlist monitoring and
/// candidate-triggered metadata refresh.
///
/// Monitoring deliberately ignores execution-only requirements such as inventory, account
/// balances and order-book depth. Those remain mandatory in the ticket-bound preview, and this
/// selector never authorizes a deterministic-opportunity Webhook by itself.
pub fn select_monitor_candidates<'a>(
    rows: impl IntoIterator<Item = &'a ArbitrageOpportunityDto>,
    now_ms: i64,
) -> Vec<CandidateSelection> {
    let mut best_by_strategy = HashMap::<StrategyKind, &'a ArbitrageOpportunityDto>::new();
    for row in rows
        .into_iter()
        .filter(|row| market_monitor_net_bps_at(row, now_ms).is_some())
    {
        let Some(strategy) = row.strategy_kind else {
            continue;
        };
        best_by_strategy
            .entry(strategy)
            .and_modify(|current| {
                if candidate_order(row, current, now_ms) == Ordering::Less {
                    *current = row;
                }
            })
            .or_insert(row);
    }

    P0_EXECUTABLE_STRATEGY_KINDS
        .into_iter()
        .filter_map(|strategy| best_by_strategy.remove(&strategy))
        .cloned()
        .map(|opportunity| CandidateSelection {
            reason: format!(
                "market-monitor strategy={} symbol={} oneCycleNetBps={:.2} worstMarketAgeMs={} executionPreviewReady={} blockers={}",
                opportunity
                    .strategy_kind
                    .map(StrategyKind::as_query_value)
                    .unwrap_or("unknown"),
                opportunity.symbol,
                one_cycle_net_bps(&opportunity),
                candidate_market_age_ms(&opportunity, now_ms),
                is_hedge_preview_ready_at(&opportunity, now_ms),
                opportunity.execution_blockers.len(),
            ),
            opportunity,
        })
        .collect()
}

fn candidate_ready(
    row: &ArbitrageOpportunityDto,
    config: &AutomatedArbitrageConfig,
    active_opportunity_ids: &BTreeSet<String>,
    now_ms: i64,
) -> bool {
    is_hedge_preview_ready_at(row, now_ms)
        && row.strategy_kind == Some(config.strategy_kind)
        && canonical_symbol_allowed(&config.canonical_symbols, &row.symbol)
        && !active_opportunity_ids.contains(&row.id)
        && one_cycle_net_bps(row) >= config.min_one_cycle_net_bps
}

fn canonical_symbol_allowed(allowed_symbols: &[String], symbol: &str) -> bool {
    allowed_symbols.is_empty()
        || allowed_symbols
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(symbol.trim()))
}

fn one_cycle_net_bps(row: &ArbitrageOpportunityDto) -> f64 {
    row.execution_cost
        .as_ref()
        .filter(|cost| cost.one_cycle.covers_round_trip_cost)
        .filter(|cost| {
            cost.round_trip.as_ref().is_some_and(|round_trip| {
                round_trip.profitability_evidence.is_cost_verified()
                    && round_trip.profitability_evidence.fee_evidence_ids.len() >= 2
            })
        })
        .map(|cost| cost.one_cycle.net_bps)
        .filter(|value| value.is_finite())
        .unwrap_or(f64::NEG_INFINITY)
}

fn candidate_order(
    left: &ArbitrageOpportunityDto,
    right: &ArbitrageOpportunityDto,
    now_ms: i64,
) -> Ordering {
    one_cycle_net_bps(right)
        .total_cmp(&one_cycle_net_bps(left))
        .then_with(|| {
            candidate_market_age_ms(left, now_ms).cmp(&candidate_market_age_ms(right, now_ms))
        })
        .then_with(|| candidate_capacity(right).total_cmp(&candidate_capacity(left)))
        .then_with(|| left.id.cmp(&right.id))
}

fn candidate_market_age_ms(row: &ArbitrageOpportunityDto, now_ms: i64) -> i64 {
    [
        row.long_leg_market_evidence.as_ref(),
        row.short_leg_market_evidence.as_ref(),
    ]
    .into_iter()
    .chain(
        row.quote_conversions
            .iter()
            .map(|conversion| conversion.market_evidence.as_ref()),
    )
    .flatten()
    .map(|evidence| now_ms.saturating_sub(evidence.health.observed_at_ms).max(0))
    .max()
    .unwrap_or(i64::MAX)
}

fn candidate_capacity(row: &ArbitrageOpportunityDto) -> f64 {
    row.long_volume_24h.min(row.short_volume_24h)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use shared_types::{
        ArbitrageType, ExecutionCostProfile, FeeProduct, FeeScheduleEvidence, HedgeLegRole,
        LegCostBreakdown, MarketDataHealth, MarketDataQuality, MarketDataSourceKind,
        OneCycleCostProfile, OpportunityLegMarketEvidence, OpportunityQuoteConversion,
        ProfitabilityEvidence, Recommendation, RiskLevel, RoundTripCostBreakdown, StrategyKind,
        TradeFeeSnapshot, TradeFeeSource,
    };

    #[test]
    fn picks_highest_verified_net_candidate() {
        let now_ms = Utc::now().timestamp_millis();
        let low_net = opportunity("low-net", 20.0, now_ms);
        let high_net = opportunity("high-net", 30.0, now_ms);

        let selected = select_candidate(
            &[low_net, high_net],
            &AutomatedArbitrageConfig::default(),
            &BTreeSet::new(),
            now_ms,
        );

        assert_eq!(
            selected.map(|row| row.opportunity.id),
            Some("high-net".into())
        );
    }

    #[test]
    fn ranked_candidates_are_descending_and_bounded() {
        let now_ms = Utc::now().timestamp_millis();
        let low = opportunity("low", 20.0, now_ms);
        let high = opportunity("high", 30.0, now_ms);
        let middle = opportunity("middle", 25.0, now_ms);

        let selected = select_candidates(
            &[low, high, middle],
            &AutomatedArbitrageConfig::default(),
            &BTreeSet::new(),
            now_ms,
            2,
        );

        assert_eq!(
            selected
                .into_iter()
                .map(|row| row.opportunity.id)
                .collect::<Vec<_>>(),
            vec!["high", "middle"]
        );
    }

    #[test]
    fn equal_profit_candidates_prefer_fresher_market() {
        let now_ms = Utc::now().timestamp_millis();
        let low_edge = opportunity("low-edge", 20.0, now_ms);
        let high_edge = opportunity("high-edge", 30.0, now_ms);

        let selected = select_candidates(
            &[low_edge, high_edge],
            &AutomatedArbitrageConfig::default(),
            &BTreeSet::new(),
            now_ms,
            2,
        );
        assert_eq!(selected[0].opportunity.id, "high-edge");

        let mut older = opportunity("older", 30.0, now_ms);
        if let Some(evidence) = older.long_leg_market_evidence.as_mut() {
            evidence.health.observed_at_ms = now_ms - 20_000;
        }
        let fresher = opportunity("fresher", 30.0, now_ms);
        let selected = select_candidates(
            &[older, fresher],
            &AutomatedArbitrageConfig::default(),
            &BTreeSet::new(),
            now_ms,
            2,
        );
        assert_eq!(selected[0].opportunity.id, "fresher");
    }

    #[test]
    fn rejects_nominally_fresh_ws_evidence_after_the_artifact_window() {
        let now_ms = Utc::now().timestamp_millis();
        let mut candidate = opportunity("stale-ws", 30.0, now_ms);
        if let Some(evidence) = candidate.short_leg_market_evidence.as_mut() {
            evidence.health.observed_at_ms =
                now_ms - shared_types::HEDGE_PREVIEW_MARKET_MAX_AGE_MS - 1;
        }

        assert!(select_candidate(
            &[candidate],
            &AutomatedArbitrageConfig::default(),
            &BTreeSet::new(),
            now_ms,
        )
        .is_none());
    }

    #[test]
    fn rejects_active_candidates() {
        let now_ms = Utc::now().timestamp_millis();
        let active = opportunity("active", 30.0, now_ms);
        let active_ids = BTreeSet::from(["active".to_owned()]);

        assert!(select_candidate(
            &[active],
            &AutomatedArbitrageConfig::default(),
            &active_ids,
            now_ms,
        )
        .is_none());
    }

    #[test]
    fn automatic_selection_defers_depth_to_the_execution_preview() {
        let now_ms = Utc::now().timestamp_millis();
        let candidate = opportunity("deferred-depth", 30.0, now_ms);

        let selected = select_candidate(
            &[candidate],
            &AutomatedArbitrageConfig::default(),
            &BTreeSet::new(),
            now_ms,
        );

        assert_eq!(
            selected.map(|row| row.opportunity.id),
            Some("deferred-depth".into())
        );
    }

    #[test]
    fn monitoring_keeps_profitable_rows_without_inventory_or_depth() {
        let now_ms = Utc::now().timestamp_millis();
        let candidates = P0_EXECUTABLE_STRATEGY_KINDS
            .into_iter()
            .enumerate()
            .map(|(index, strategy)| {
                let mut candidate = opportunity(&format!("monitor-{index}"), 30.0, now_ms);
                candidate.strategy_kind = Some(strategy);
                candidate.execution_eligible = false;
                candidate.execution_blockers = vec![DEFERRED_INVENTORY_OR_BORROW_BLOCKER.into()];
                candidate
            })
            .collect::<Vec<_>>();

        let selected = select_monitor_candidates(candidates.iter(), now_ms);

        assert_eq!(selected.len(), P0_EXECUTABLE_STRATEGY_KINDS.len());
        assert!(selected
            .iter()
            .all(|row| row.reason.contains("executionPreviewReady=false")));
    }

    #[test]
    fn monitoring_keeps_spot_perp_rows_until_ticket_bound_checks() {
        let now_ms = Utc::now().timestamp_millis();
        let mut candidate = opportunity("spot-perp-ticket", 30.0, now_ms);
        candidate.strategy_kind = Some(StrategyKind::SpotPerp);
        candidate.execution_eligible = false;
        candidate.execution_blockers = vec![DEFERRED_SPOT_PERP_TICKET_BLOCKER.into()];

        let selected = select_monitor_candidates([&candidate], now_ms);

        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].opportunity.id, "spot-perp-ticket");
    }

    #[test]
    fn monitoring_rejects_non_inventory_strategy_blockers() {
        let now_ms = Utc::now().timestamp_millis();
        let mut candidate = opportunity("price-mismatch", 30.0, now_ms);
        candidate.execution_eligible = false;
        candidate.execution_blockers =
            vec!["双边可成交价格差 5.31% 超过 0.30%，可能是行情、合约身份或价格单位异常".into()];

        assert!(select_monitor_candidates([&candidate], now_ms).is_empty());
    }

    #[test]
    fn monitoring_waits_for_cross_quote_ws_evidence() {
        let now_ms = Utc::now().timestamp_millis();
        let mut candidate = opportunity("cross-quote", 30.0, now_ms);
        candidate.quote_conversions = vec![OpportunityQuoteConversion {
            from_quote: "USDC".into(),
            to_quote: "USDT".into(),
            rate: 0.999,
            venue: "okx".into(),
            symbol: "USDC-USDT".into(),
            market_evidence: None,
        }];

        assert!(select_monitor_candidates([&candidate], now_ms).is_empty());
        candidate.quote_conversions[0].market_evidence = Some(market_evidence("okx", now_ms));
        assert_eq!(select_monitor_candidates([&candidate], now_ms).len(), 1);
    }

    #[test]
    fn monitoring_returns_the_best_row_for_each_strategy() {
        let now_ms = Utc::now().timestamp_millis();
        let mut lower_perp = opportunity("perp-low", 20.0, now_ms);
        lower_perp.strategy_kind = Some(StrategyKind::PerpCross);
        let mut higher_perp = opportunity("perp-high", 30.0, now_ms);
        higher_perp.strategy_kind = Some(StrategyKind::PerpCross);
        let mut spot = opportunity("spot", 25.0, now_ms);
        spot.strategy_kind = Some(StrategyKind::SpotCross);

        let selected = select_monitor_candidates([&lower_perp, &higher_perp, &spot], now_ms);

        assert_eq!(
            selected
                .iter()
                .map(|row| row.opportunity.id.as_str())
                .collect::<Vec<_>>(),
            vec!["perp-high", "spot"]
        );
    }

    #[test]
    fn selects_only_the_explicit_strategy_scope() {
        let now_ms = Utc::now().timestamp_millis();
        let mut perp_cross = opportunity("perp-cross", 40.0, now_ms);
        perp_cross.strategy_kind = Some(StrategyKind::PerpCross);
        let mut spread = opportunity("spread", 20.0, now_ms);
        spread.strategy_kind = Some(StrategyKind::PerpPriceSpread);
        let config = AutomatedArbitrageConfig {
            strategy_kind: StrategyKind::PerpPriceSpread,
            ..AutomatedArbitrageConfig::default()
        };

        let selected = select_candidate(&[perp_cross, spread], &config, &BTreeSet::new(), now_ms);

        assert_eq!(
            selected.map(|row| row.opportunity.id),
            Some("spread".into())
        );
    }

    #[test]
    fn selects_only_the_explicit_canonical_symbol_scope() {
        let now_ms = Utc::now().timestamp_millis();
        let mut other = opportunity("other", 40.0, now_ms);
        other.symbol = "COTI".into();
        let mut target = opportunity("target", 20.0, now_ms);
        target.symbol = "BTW".into();
        let config = AutomatedArbitrageConfig {
            canonical_symbols: vec!["BTW".into()],
            ..AutomatedArbitrageConfig::default()
        };

        let selected = select_candidate(&[other, target], &config, &BTreeSet::new(), now_ms);

        assert_eq!(
            selected.map(|row| row.opportunity.id),
            Some("target".into())
        );
    }

    #[test]
    fn rejects_candidates_without_strategy_identity() {
        let now_ms = Utc::now().timestamp_millis();
        let mut candidate = opportunity("unknown", 40.0, now_ms);
        candidate.strategy_kind = None;

        assert!(select_candidate(
            &[candidate],
            &AutomatedArbitrageConfig::default(),
            &BTreeSet::new(),
            now_ms,
        )
        .is_none());
    }

    #[test]
    fn rejects_negative_net_edge_and_missing_bilateral_market_evidence() {
        let now_ms = Utc::now().timestamp_millis();
        let negative = opportunity("negative", -1.0, now_ms);
        let mut missing_market = opportunity("missing-market", 40.0, now_ms);
        missing_market.short_leg_market_evidence = None;

        assert!(select_candidate(
            &[negative],
            &AutomatedArbitrageConfig::default(),
            &BTreeSet::new(),
            now_ms,
        )
        .is_none());
        assert!(select_candidate(
            &[missing_market],
            &AutomatedArbitrageConfig::default(),
            &BTreeSet::new(),
            now_ms,
        )
        .is_none());
    }

    #[allow(clippy::too_many_lines)]
    fn opportunity(id: &str, net_bps: f64, now_ms: i64) -> ArbitrageOpportunityDto {
        let fee = fee_snapshot(now_ms);
        let profitability_evidence = ProfitabilityEvidence::from_fee_snapshots(
            "automation-selector-fixture",
            now_ms,
            &[&fee, &fee],
            None,
        );
        ArbitrageOpportunityDto {
            id: id.to_owned(),
            symbol: "BTC-USDT".to_owned(),
            arb_type: ArbitrageType::CrossExchange,
            type_label: "跨所永续".to_owned(),
            long_exchange: "binance".to_owned(),
            short_exchange: "okx".to_owned(),
            spread_8h: 0.1,
            long_rate_8h: 0.0,
            short_rate_8h: 0.0,
            long_rate: 0.0,
            short_rate: 0.0,
            single_yield: 0.1,
            net_single_yield: 0.1,
            raw_single_yield: 0.1,
            settlement_interval: 8,
            risk_adjusted_yield: 0.1,
            trading_cost_rate: 0.01,
            min_holding_periods: 1,
            risk_level: RiskLevel::Low,
            volatility: 0.0,
            sharpe_ratio: 1.0,
            score: 0.0,
            score_breakdown: None,
            ranking_key: None,
            recommendation: Recommendation::StrongBuy,
            optimal_position: 100.0,
            max_position: 100.0,
            liquidity_score: 90.0,
            volume_24h: 1_000_000.0,
            long_volume_24h: 1_000_000.0,
            short_volume_24h: 1_000_000.0,
            data_source: "fixture".to_owned(),
            confidence: 1.0,
            updated_at: Utc::now(),
            long_funding_interval: 8,
            short_funding_interval: 8,
            settlement_time_diff: false,
            strategy_description: String::new(),
            long_action: "buy".to_owned(),
            short_action: "sell".to_owned(),
            long_next_funding_time: 0,
            short_next_funding_time: 0,
            time_to_settlement_ms: 1_000,
            is_snipe_ready: true,
            long_price: Some(100.0),
            short_price: Some(101.0),
            long_leg_market_evidence: Some(market_evidence("binance", now_ms)),
            short_leg_market_evidence: Some(market_evidence("okx", now_ms)),
            quote_conversions: Vec::new(),
            price_deviation: None,
            basis_spread: None,
            basis_annual_cost: None,
            risk_warnings: Vec::new(),
            execution_eligible: true,
            execution_blockers: Vec::new(),
            execution_cost: Some(ExecutionCostProfile {
                gross_edge_bps: net_bps + 10.0,
                fee_bps: 5.0,
                wear_bps: 5.0,
                total_cost_bps: 10.0,
                one_cycle: OneCycleCostProfile {
                    net_bps,
                    covers_round_trip_cost: true,
                    ..OneCycleCostProfile::default()
                },
                breakeven_periods: 1,
                breakeven_hours: 8.0,
                recommended_hold_periods: 1,
                recommended_hold_hours: 8.0,
                net_bps_at_recommended_hold: net_bps,
                round_trip: Some(RoundTripCostBreakdown {
                    long_leg: leg_cost(fee.clone()),
                    short_leg: leg_cost(fee),
                    open_fee_bps: 2.0,
                    close_fee_bps: 2.0,
                    open_slippage_bps: 3.0,
                    close_slippage_bps: 3.0,
                    borrow_or_financing_bps: 0.0,
                    funding_window_mismatch_buffer_bps: 0.0,
                    min_profit_buffer_bps: 0.0,
                    total_cost_bps: 10.0,
                    one_cycle_net_bps: net_bps,
                    profitability_evidence,
                }),
            }),
            index_composition: None,
            strategy_kind: Some(StrategyKind::PerpCross),
            strategy_category: None,
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
            settlement_countdown_seconds: Some(1),
        }
    }

    fn market_evidence(venue: &str, now_ms: i64) -> OpportunityLegMarketEvidence {
        OpportunityLegMarketEvidence {
            venue: venue.to_owned(),
            symbol: "BTC-USDT".to_owned(),
            price: Some(100.0),
            health: MarketDataHealth {
                quality: MarketDataQuality::Fresh,
                source: MarketDataSourceKind::WsPush,
                freshness_ms: Some(1),
                retry_after_ms: None,
                last_error: None,
                observed_at_ms: now_ms,
                coverage: None,
                problem: None,
            },
        }
    }

    fn fee_snapshot(now_ms: i64) -> TradeFeeSnapshot {
        TradeFeeSnapshot {
            venue: "binance".to_owned(),
            symbol: "BTC-USDT".to_owned(),
            product: FeeProduct::Perp,
            account_id: None,
            maker_fee_bps: 1.0,
            taker_fee_bps: 2.0,
            open_fee_bps: 2.0,
            close_fee_bps: 2.0,
            source: TradeFeeSource::OfficialSchedule,
            fetched_at_ms: now_ms,
            valid_until_ms: now_ms + 60_000,
            freshness_ms: Some(0),
            evidence: Some(FeeScheduleEvidence {
                evidence_id: "fee-fixture".to_owned(),
                source_name: "official schedule".to_owned(),
                source_url: "https://www.binance.com/en/fee/futureFee".to_owned(),
                checked_at_ms: now_ms,
                effective_at_ms: Some(now_ms),
                schedule_version: Some("v1".to_owned()),
                tier: Some("standard".to_owned()),
                scope: Some("perp".to_owned()),
                problem: None,
            }),
            verification_problem: None,
            note: None,
        }
    }

    fn leg_cost(fee_snapshot: TradeFeeSnapshot) -> LegCostBreakdown {
        LegCostBreakdown {
            role: HedgeLegRole::Long,
            venue: fee_snapshot.venue.clone(),
            symbol: fee_snapshot.symbol.clone(),
            product: FeeProduct::Perp,
            open_fee_bps: 2.0,
            close_fee_bps: 2.0,
            open_slippage_bps: 3.0,
            close_slippage_bps: 3.0,
            fee_snapshot: Some(fee_snapshot),
        }
    }
}
