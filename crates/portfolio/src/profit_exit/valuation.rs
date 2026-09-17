use shared_types::{AutoProfitCloseConfig, ExecutionRun, PositionPairEvidence, PositionRow};

#[derive(Debug, Clone, PartialEq)]
pub struct ProfitExitValuation {
    pub matched_notional_usd: f64,
    pub gross_unrealized_pnl_usd: f64,
    pub open_fee_usd: f64,
    pub funding_pnl_usd: f64,
    pub estimated_exit_cost_usd: f64,
    pub safety_buffer_usd: f64,
    pub estimated_net_profit_usd: f64,
    pub estimated_roi_bps: f64,
}

pub(super) fn profit_exit_valuation(
    run: &ExecutionRun,
    evidence: &PositionPairEvidence,
    anchor: &PositionRow,
    partner: &PositionRow,
    config: &AutoProfitCloseConfig,
) -> Option<ProfitExitValuation> {
    run.valuation_problem.is_none().then_some(())?;
    let cost = run.cost_reconciliation.as_ref()?;
    let matched_notional_usd = positive_finite(evidence.matched_notional_usd)?;
    let total_notional_usd = position_notional(anchor)? + position_notional(partner)?;
    let gross_unrealized_pnl_usd =
        finite_sum(anchor.unrealized_pnl_usd, partner.unrealized_pnl_usd)?;

    // Venue unrealized PnL already starts from the actual average entry price,
    // so opening slippage is embedded in it. Only the paid opening fee remains.
    let actual_open_cost_usd = finite(cost.actual_open_cost_usd?)?;
    let actual_open_slippage_usd = finite(cost.actual_slippage_usd?)?;
    let open_fee_usd = non_negative_finite(actual_open_cost_usd - actual_open_slippage_usd)?;
    let funding_pnl_usd = finite(cost.actual_funding_usd.unwrap_or(0.0))?;
    let estimated_exit_cost_usd =
        non_negative_finite(cost.estimated_close_cost_usd + cost.estimated_slippage_usd)?;
    let safety_buffer_usd =
        non_negative_finite(total_notional_usd * config.exit_buffer_bps / 10_000.0)?;
    let estimated_net_profit_usd = gross_unrealized_pnl_usd + funding_pnl_usd
        - open_fee_usd
        - estimated_exit_cost_usd
        - safety_buffer_usd;
    let estimated_roi_bps = estimated_net_profit_usd / matched_notional_usd * 10_000.0;
    finite(estimated_net_profit_usd)?;
    finite(estimated_roi_bps)?;

    Some(ProfitExitValuation {
        matched_notional_usd,
        gross_unrealized_pnl_usd,
        open_fee_usd,
        funding_pnl_usd,
        estimated_exit_cost_usd,
        safety_buffer_usd,
        estimated_net_profit_usd,
        estimated_roi_bps,
    })
}

fn position_notional(row: &PositionRow) -> Option<f64> {
    positive_finite(row.quantity.abs() * row.mark_price)
}

fn positive_finite(value: f64) -> Option<f64> {
    (value.is_finite() && value > 0.0).then_some(value)
}

fn non_negative_finite(value: f64) -> Option<f64> {
    (value.is_finite() && value >= 0.0).then_some(value)
}

fn finite(value: f64) -> Option<f64> {
    value.is_finite().then_some(value)
}

fn finite_sum(left: f64, right: f64) -> Option<f64> {
    finite(left)?;
    finite(right)?;
    finite(left + right)
}
