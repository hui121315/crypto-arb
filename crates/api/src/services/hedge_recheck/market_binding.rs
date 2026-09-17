use super::*;
use shared_types::{HedgeLegQuote, OrderIntent, OrderPayloadPricePolicy, OrderType};

#[derive(Clone, Copy)]
pub(super) enum SizingRefreshMode {
    PairedWithinCaps,
    ExactQuantity,
}

pub(super) fn refresh_order_sizing_plans(
    preview: &mut HedgePreviewResponse,
    market: &mut crate::services::hedge_ticket::SubmitMarketSnapshot,
    mode: SizingRefreshMode,
) -> Option<String> {
    let Some(long_price) = executable_price(&market.long_leg) else {
        return Some(format!(
            "{} {} 缺少最新可成交价",
            preview.long_leg.exchange, preview.long_leg.symbol
        ));
    };
    let Some(short_price) = executable_price(&market.short_leg) else {
        return Some(format!(
            "{} {} 缺少最新可成交价",
            preview.short_leg.exchange, preview.short_leg.symbol
        ));
    };
    let quantity = market.target_base_quantity;
    let uses_live_mode = preview.long_leg.mode == shared_types::ExecutionMode::Live
        || preview.short_leg.mode == shared_types::ExecutionMode::Live;
    if !uses_live_mode {
        bind_refreshed_order_facts(preview, market, quantity, long_price, short_price);
        return None;
    }
    let fallback_cap = preview.ticket.sizing.target_notional_usd;
    let long_configured_cap = preview.ticket.sizing.long_notional_cap_usd;
    let short_configured_cap = preview.ticket.sizing.short_notional_cap_usd;
    let Some(plans) = preview.ticket_order_plans.as_mut() else {
        return Some("票据缺少双腿下单计划".to_owned());
    };
    let long_plan = &mut plans.long.compile_plan;
    let short_plan = &mut plans.short.compile_plan;
    let Some(long_instrument) = long_plan.instrument_spec.as_ref() else {
        return Some(format!(
            "{} {} 缺少官方下单规格",
            long_plan.exchange, long_plan.symbol
        ));
    };
    let Some(short_instrument) = short_plan.instrument_spec.as_ref() else {
        return Some(format!(
            "{} {} 缺少官方下单规格",
            short_plan.exchange, short_plan.symbol
        ));
    };
    let sizing = match mode {
        SizingRefreshMode::PairedWithinCaps => {
            let long_cap =
                effective_notional_cap(long_configured_cap, fallback_cap, quantity * long_price);
            let short_cap =
                effective_notional_cap(short_configured_cap, fallback_cap, quantity * short_price);
            shared_types::plan_paired_leg_sizing(
                long_cap,
                long_instrument,
                long_price,
                short_cap,
                short_instrument,
                short_price,
            )
            .map(|paired| (paired.base_quantity, paired.long, paired.short))
        }
        SizingRefreshMode::ExactQuantity => {
            shared_types::plan_leg_sizing_for_base_quantity(quantity, long_instrument, long_price)
                .and_then(|long| {
                    shared_types::plan_leg_sizing_for_base_quantity(
                        quantity,
                        short_instrument,
                        short_price,
                    )
                    .map(|short| (quantity, long, short))
                })
        }
    };
    let (quantity, long_sizing, short_sizing) = match sizing {
        Ok(sizing) => sizing,
        Err(block) => return Some(block.code().to_owned()),
    };
    long_plan.sizing_plan = Some(long_sizing);
    short_plan.sizing_plan = Some(short_sizing);
    bind_refreshed_order_facts(preview, market, quantity, long_price, short_price);
    None
}

fn bind_refreshed_order_facts(
    preview: &mut HedgePreviewResponse,
    market: &mut crate::services::hedge_ticket::SubmitMarketSnapshot,
    quantity: f64,
    long_price: f64,
    short_price: f64,
) {
    preview.long_leg.quantity = quantity;
    preview.short_leg.quantity = quantity;
    market.set_target_base_quantity(quantity);
    let long_intent = &mut preview.long_leg;
    let short_intent = &mut preview.short_leg;
    if let Some(plans) = preview.ticket_order_plans.as_mut() {
        bind_order_market_reference(long_intent, &mut plans.long.compile_plan, long_price);
        bind_order_market_reference(short_intent, &mut plans.short.compile_plan, short_price);
    } else {
        let long_order_type = long_intent.order_type;
        let short_order_type = short_intent.order_type;
        bind_intent_market_reference(long_intent, long_order_type, long_price);
        bind_intent_market_reference(short_intent, short_order_type, short_price);
    }
}

fn bind_order_market_reference(
    intent: &mut OrderIntent,
    plan: &mut shared_types::OrderCompilePlan,
    executable_price: f64,
) {
    plan.reference_price = Some(executable_price);
    bind_intent_market_reference(intent, plan.effective_order_type, executable_price);
    if plan.effective_order_type != OrderType::Market {
        return;
    }
    plan.protection_price = intent.price;
    plan.payload_price = match plan.payload_price_policy {
        OrderPayloadPricePolicy::LimitPrice | OrderPayloadPricePolicy::ProtectionPrice => {
            intent.price
        }
        OrderPayloadPricePolicy::ZeroPrice => Some(0.0),
        OrderPayloadPricePolicy::Omit | OrderPayloadPricePolicy::MarketLikeNoPrice => None,
    };
}

fn bind_intent_market_reference(
    intent: &mut OrderIntent,
    effective_order_type: OrderType,
    executable_price: f64,
) {
    if effective_order_type == OrderType::Market {
        intent.price = Some(executable_price);
    }
}

fn executable_price(quote: &HedgeLegQuote) -> Option<f64> {
    quote
        .open_vwap_price
        .filter(|price| price.is_finite() && *price > 0.0)
}

fn effective_notional_cap(configured: f64, fallback: f64, current: f64) -> f64 {
    let authorized = [configured, fallback]
        .into_iter()
        .find(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(current);
    authorized.min(current)
}
