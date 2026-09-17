use super::BLOCKER_PREFIX;
use shared_types::ArbitrageOpportunityDto;

pub(super) fn apply_transfer_cost(
    opportunity: &mut ArbitrageOpportunityDto,
    transfer_cost_bps: f64,
) -> Option<String> {
    let Some(cost) = opportunity.execution_cost.as_mut() else {
        return Some(format!(
            "{BLOCKER_PREFIX}完整交易成本档案缺失，无法把充提成本并入净收益"
        ));
    };
    let transfer_rate = transfer_cost_bps / 10_000.0;
    opportunity.trading_cost_rate += transfer_rate;
    opportunity.net_single_yield -= transfer_rate;
    opportunity.risk_adjusted_yield -= transfer_rate;
    cost.wear_bps += transfer_cost_bps;
    cost.total_cost_bps += transfer_cost_bps;
    cost.one_cycle.net_bps -= transfer_cost_bps;
    cost.one_cycle.covers_round_trip_cost =
        cost.one_cycle.covers_round_trip_cost && cost.one_cycle.net_bps > 0.0;
    cost.net_bps_at_recommended_hold -= transfer_cost_bps;
    if let Some(round_trip) = cost.round_trip.as_mut() {
        round_trip.borrow_or_financing_bps += transfer_cost_bps;
        round_trip.total_cost_bps += transfer_cost_bps;
        round_trip.one_cycle_net_bps -= transfer_cost_bps;
    }
    (!cost.one_cycle.covers_round_trip_cost).then(|| {
        format!(
            "{BLOCKER_PREFIX}计入充提成本 {:.3}% 后单次净收益不再为正",
            transfer_cost_bps / 100.0
        )
    })
}
