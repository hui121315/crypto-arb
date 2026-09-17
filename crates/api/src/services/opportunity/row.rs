use super::*;

pub(crate) fn sort_refs(
    rows: &mut [&ArbitrageOpportunityDto],
    sort_key: OpportunityListSortKey,
    now_ms: i64,
) {
    rows.sort_by(|left, right| list_rank_order(left, right, sort_key, now_ms));
}
#[cfg(test)]
pub(crate) fn list_row_from_dto(row: &ArbitrageOpportunityDto) -> OpportunityListRow {
    list_row_from_dto_at(row, Utc::now().timestamp_millis())
}

pub(crate) fn list_row_from_dto_at(
    row: &ArbitrageOpportunityDto,
    observed_at_ms: i64,
) -> OpportunityListRow {
    let cost = list_cost(row.execution_cost.as_ref(), observed_at_ms);
    OpportunityListRow {
        id: row.id.clone(),
        symbol: row.symbol.clone(),
        strategy_kind: row.strategy_kind,
        strategy_category: row.strategy_category,
        type_label: row.type_label.clone(),
        spot_leg_mode: row.spot_leg_mode,
        long_leg: OpportunityListLeg {
            venue: row.long_exchange.clone(),
            action: row.long_action.clone(),
            price: row.long_price,
            market_evidence: row.long_leg_market_evidence.clone(),
            funding: list_leg_funding(row, HedgeLegRole::Long),
        },
        short_leg: OpportunityListLeg {
            venue: row.short_exchange.clone(),
            action: row.short_action.clone(),
            price: row.short_price,
            market_evidence: row.short_leg_market_evidence.clone(),
            funding: list_leg_funding(row, HedgeLegRole::Short),
        },
        metrics: OpportunityListMetrics {
            score: 0.0,
            risk_level: row.risk_level,
            net_single_yield: row.net_single_yield,
            annualized_funding_bps: row.annualized_funding_bps,
            one_cycle_net_bps: cost.one_cycle_net_bps,
            time_to_settlement_ms: row.time_to_settlement_ms,
            settlement_countdown_seconds: row.settlement_countdown_seconds,
            liquidity_score: row.liquidity_score,
        },
        cost,
        execution: OpportunityListExecution {
            // List eligibility owns ticket construction only; Live submission rechecks the ticket.
            eligible: is_ticket_build_ready_at(row, observed_at_ms),
            blockers: row.execution_blockers.clone(),
            optimal_position: row.optimal_position,
            max_position: row.max_position,
        },
        data_source: row.data_source.clone(),
        updated_at: row.updated_at,
    }
}

fn list_leg_funding(
    row: &ArbitrageOpportunityDto,
    role: HedgeLegRole,
) -> Option<OpportunityListLegFunding> {
    (p0_hedge_leg_product(row.strategy_kind, row.spot_leg_mode, role) == Some(FeeProduct::Perp))
        .then(|| {
            let (rate, interval_hours, next_funding_time_ms) = match role {
                HedgeLegRole::Long => (
                    row.long_rate,
                    row.long_funding_interval,
                    row.long_next_funding_time,
                ),
                HedgeLegRole::Short => (
                    row.short_rate,
                    row.short_funding_interval,
                    row.short_next_funding_time,
                ),
            };
            OpportunityListLegFunding {
                rate,
                interval_hours: (interval_hours > 0).then_some(interval_hours),
                next_funding_time_ms: (next_funding_time_ms > 0).then_some(next_funding_time_ms),
            }
        })
}

fn list_cost(cost: Option<&ExecutionCostProfile>, now_ms: i64) -> OpportunityListCost {
    let Some(cost) = cost else {
        return OpportunityListCost::default();
    };
    let fee_evidence_count = verified_fee_snapshot_count(cost, now_ms);
    let profitability = cost
        .round_trip
        .as_ref()
        .map(|round_trip| &round_trip.profitability_evidence);
    let fee_evidence_complete = profitability.is_some_and(|evidence| evidence.is_cost_verified())
        && has_verified_round_trip_cost(cost, now_ms);
    let fee_evidence_ids = profitability
        .filter(|_| fee_evidence_complete)
        .map(|evidence| evidence.fee_evidence_ids.clone())
        .unwrap_or_default();
    OpportunityListCost {
        verified: fee_evidence_complete,
        gross_edge_bps: cost.gross_edge_bps,
        total_cost_bps: cost.total_cost_bps,
        wear_bps: cost.wear_bps,
        one_cycle_net_bps: fee_evidence_complete.then_some(cost.one_cycle.net_bps),
        one_cycle_covers_cost: cost.one_cycle.covers_round_trip_cost,
        breakeven_periods: cost.breakeven_periods,
        breakeven_hours: cost.breakeven_hours,
        recommended_hold_hours: cost.recommended_hold_hours,
        net_bps_at_recommended_hold: cost.net_bps_at_recommended_hold,
        fee_evidence_count,
        fee_evidence_complete,
        fee_evidence_ids,
        one_cycle_penalty: 0.0,
    }
}

fn verified_fee_snapshot_count(cost: &ExecutionCostProfile, now_ms: i64) -> usize {
    let Some(round_trip) = cost.round_trip.as_ref() else {
        return 0;
    };
    [
        round_trip.long_leg.fee_snapshot.as_ref(),
        round_trip.short_leg.fee_snapshot.as_ref(),
    ]
    .into_iter()
    .flatten()
    .filter(|snapshot| snapshot.is_fresh_verified(now_ms))
    .count()
}

fn list_rank_order(
    left: &ArbitrageOpportunityDto,
    right: &ArbitrageOpportunityDto,
    sort_key: OpportunityListSortKey,
    now_ms: i64,
) -> std::cmp::Ordering {
    match sort_key {
        OpportunityListSortKey::Score | OpportunityListSortKey::NetSingleYield => {
            profit_floor_order(left, right, now_ms)
        }
        OpportunityListSortKey::Settlement => left
            .settlement_countdown_seconds
            .unwrap_or(i64::MAX)
            .cmp(&right.settlement_countdown_seconds.unwrap_or(i64::MAX))
            .then_with(|| profit_floor_order(left, right, now_ms))
            .then_with(|| left.id.cmp(&right.id)),
    }
}

fn profit_floor_order(
    left: &ArbitrageOpportunityDto,
    right: &ArbitrageOpportunityDto,
    now_ms: i64,
) -> std::cmp::Ordering {
    right
        .execution_eligible
        .cmp(&left.execution_eligible)
        .then_with(|| {
            verified_one_cycle_net_bps(right, now_ms)
                .total_cmp(&verified_one_cycle_net_bps(left, now_ms))
        })
        .then_with(|| right.net_single_yield.total_cmp(&left.net_single_yield))
        .then_with(|| left.id.cmp(&right.id))
}

fn verified_one_cycle_net_bps(row: &ArbitrageOpportunityDto, now_ms: i64) -> f64 {
    row.execution_cost
        .as_ref()
        .filter(|cost| has_verified_round_trip_cost(cost, now_ms))
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
