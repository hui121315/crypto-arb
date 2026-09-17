use super::*;

pub(super) fn order_protection_rejection(
    preview: &HedgePreviewResponse,
    market: &crate::services::hedge_ticket::SubmitMarketSnapshot,
) -> Option<String> {
    let order_types = preview.ticket_order_plans.as_ref().map_or(
        [preview.long_leg.order_type, preview.short_leg.order_type],
        |plans| {
            [
                plans.long.compile_plan.effective_order_type,
                plans.short.compile_plan.effective_order_type,
            ]
        },
    );
    [
        (&preview.long_leg, &market.long_leg, order_types[0]),
        (&preview.short_leg, &market.short_leg, order_types[1]),
    ]
    .into_iter()
    .find_map(|(intent, quote, order_type)| leg_order_protection_blocker(intent, quote, order_type))
}

pub(super) fn leg_order_protection_blocker(
    intent: &OrderIntent,
    quote: &HedgeLegQuote,
    order_type: OrderType,
) -> Option<String> {
    if order_type == OrderType::Market {
        return None;
    }
    if order_type == OrderType::PostOnly {
        return Some(format!(
            "{} {} PostOnly 不能证明当前双腿立即成交",
            quote.exchange, quote.symbol
        ));
    }
    let protection = intent
        .price
        .filter(|price| price.is_finite() && *price > 0.0)?;
    let executable = quote
        .open_vwap_price
        .filter(|price| price.is_finite() && *price > 0.0)?;
    let protected = match quote.side {
        OrderSide::Buy => executable <= protection + f64::EPSILON,
        OrderSide::Sell => executable + f64::EPSILON >= protection,
    };
    (!protected).then(|| {
        format!(
            "{} {} 最新可成交价 {:.8} 超出原票据保护价 {:.8}",
            quote.exchange, quote.symbol, executable, protection
        )
    })
}
