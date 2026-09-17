use super::*;

/// Mark-to-market directional exposure from the unmatched base quantity.
/// Entry-price spread is arbitrage PnL, not directional exposure.
pub(crate) fn net_base_exposure_usd(run: &ExecutionRun) -> f64 {
    let long_quantity = active_filled_quantity(&run.long_leg);
    let short_quantity = active_filled_quantity(&run.short_leg);
    let residual_quantity = long_quantity - short_quantity;
    if residual_quantity.abs() <= f64::EPSILON {
        return 0.0;
    }
    residual_quantity * exposure_mark_price(run).unwrap_or_default()
}

fn active_filled_quantity(leg: &ExecutionRunLeg) -> f64 {
    if !matches!(
        leg.state,
        LiveOrderState::PartiallyFilled | LiveOrderState::Filled
    ) {
        return 0.0;
    }
    finite_positive(leg.filled_quantity).unwrap_or_default()
}

fn exposure_mark_price(run: &ExecutionRun) -> Option<f64> {
    let mut total = 0.0;
    let mut count = 0_u8;
    for price in [&run.long_leg, &run.short_leg]
        .into_iter()
        .filter_map(leg_fill_price)
    {
        total += price;
        count += 1;
    }
    (count > 0).then(|| total / f64::from(count))
}

fn leg_fill_price(leg: &ExecutionRunLeg) -> Option<f64> {
    let notional = finite_positive(leg.filled_notional_usd)?;
    let quantity = finite_positive(leg.filled_quantity)?;
    Some(notional / quantity)
}
