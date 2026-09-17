use super::*;

pub(super) async fn build_ticket_leg_quotes(
    state: &AppState,
    opportunity: &ArbitrageOpportunityDto,
    notional_caps: [f64; 2],
    now_ms: i64,
) -> (HedgeLegQuote, HedgeLegQuote, Option<f64>) {
    let (prepared_long, prepared_short) = join(
        prepare_leg_quote(
            state,
            opportunity,
            HedgeLegRole::Long,
            now_ms,
            EXECUTION_DEPTH_BPS,
        ),
        prepare_leg_quote(
            state,
            opportunity,
            HedgeLegRole::Short,
            now_ms,
            EXECUTION_DEPTH_BPS,
        ),
    )
    .await;
    let initial_long = prepared_long.build_for_notional(notional_caps[0]);
    let initial_short = prepared_short.build_for_notional(notional_caps[1]);
    let target_base_quantity = ticket_base_quantity(
        state,
        opportunity,
        notional_caps,
        [&initial_long, &initial_short],
    );
    let (long_leg, short_leg) = match target_base_quantity {
        Some(quantity) => (
            prepared_long.build_for_base_quantity(quantity),
            prepared_short.build_for_base_quantity(quantity),
        ),
        None => (initial_long, initial_short),
    };
    (long_leg, short_leg, target_base_quantity)
}

fn ticket_base_quantity(
    state: &AppState,
    opportunity: &ArbitrageOpportunityDto,
    notional_caps: [f64; 2],
    legs: [&HedgeLegQuote; 2],
) -> Option<f64> {
    let long_price = executable_leg_price(legs[0])?;
    let short_price = executable_leg_price(legs[1])?;
    let registry = state.instrument_registry();
    let long_product = fee_product_for(opportunity, HedgeLegRole::Long);
    let short_product = fee_product_for(opportunity, HedgeLegRole::Short);
    let paired = registry
        .resolve_hedge_instrument_for_product(&legs[0].exchange, &legs[0].symbol, long_product)
        .zip(registry.resolve_hedge_instrument_for_product(
            &legs[1].exchange,
            &legs[1].symbol,
            short_product,
        ))
        .and_then(|(long_instrument, short_instrument)| {
            shared_types::plan_paired_leg_sizing(
                notional_caps[0],
                &long_instrument,
                long_price,
                notional_caps[1],
                &short_instrument,
                short_price,
            )
            .ok()
        });
    paired.map(|plan| plan.base_quantity).or_else(|| {
        let quantity = (notional_caps[0] / long_price).min(notional_caps[1] / short_price);
        (quantity.is_finite() && quantity > 0.0).then_some(quantity)
    })
}

fn executable_leg_price(leg: &HedgeLegQuote) -> Option<f64> {
    leg.open_vwap_price
        .or(leg.reference_price)
        .filter(|price| price.is_finite() && *price > 0.0)
}
