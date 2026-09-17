//! Ticket-bound spot order identity, sizing and precision contract.

use super::spot_instruments;
use crate::error::{ExchangeError, ExchangeResult};
use shared_types::{
    validate_order_sizing_contract, CancelOrderRequest, FeeProduct, OrderIntent,
    OrderSubmissionContext, OrderType,
};

const PRODUCT_SPOT: &str = "spot";

#[derive(Debug, Clone, PartialEq)]
pub(super) struct CompiledSpotOrder {
    pub native_symbol: String,
    pub quantity: String,
    pub quote_notional: String,
    pub price: Option<String>,
}

pub(super) fn compile(
    venue: &str,
    intent: &OrderIntent,
    context: &OrderSubmissionContext,
) -> ExchangeResult<CompiledSpotOrder> {
    let instrument = validated_instrument(venue, &intent.symbol, context)?;
    let sizing = context
        .sizing_plan
        .ok_or_else(|| contract_error(venue, "spot order requires ticket-bound sizing evidence"))?;
    validate_order_sizing_contract(venue, &intent.symbol, instrument, sizing)
        .map_err(|error| contract_error(venue, error.code()))?;
    validate_spot_intent(venue, intent, sizing.rounded_base_qty)?;
    let price = match intent.order_type {
        OrderType::Market => None,
        OrderType::Limit | OrderType::PostOnly => {
            let value = intent
                .price
                .ok_or_else(|| contract_error(venue, "spot limit order requires price"))?;
            let tick = instrument
                .price_tick
                .ok_or_else(|| contract_error(venue, "spot price tick is missing"))?;
            validate_step(venue, "price", value, tick)?;
            Some(decimal_param(venue, "price", value)?)
        }
    };
    Ok(CompiledSpotOrder {
        native_symbol: instrument.native_symbol.clone(),
        quantity: decimal_param(venue, "quantity", sizing.rounded_base_qty)?,
        quote_notional: decimal_param(venue, "quote_notional", sizing.actual_notional_usd)?,
        price,
    })
}

pub(super) fn cancel_symbol(
    venue: &str,
    request: &CancelOrderRequest,
    context: &OrderSubmissionContext,
) -> ExchangeResult<String> {
    Ok(validated_instrument(venue, &request.symbol, context)?
        .native_symbol
        .clone())
}

pub(super) fn query_symbol(
    venue: &str,
    symbol: &str,
    context: &OrderSubmissionContext,
) -> ExchangeResult<String> {
    Ok(validated_instrument(venue, symbol, context)?
        .native_symbol
        .clone())
}

fn validated_instrument<'a>(
    venue: &str,
    symbol: &str,
    context: &'a OrderSubmissionContext,
) -> ExchangeResult<&'a shared_types::InstrumentSpec> {
    if context.product != FeeProduct::Spot {
        return Err(contract_error(venue, "spot route requires product=spot"));
    }
    let instrument = context.instrument_spec.as_ref().ok_or_else(|| {
        contract_error(
            venue,
            "spot route requires ticket-bound instrument evidence",
        )
    })?;
    if instrument.product_type.as_deref() != Some(PRODUCT_SPOT) {
        return Err(contract_error(
            venue,
            "ticket-bound instrument is not a spot instrument",
        ));
    }
    if !spot_instruments::evidence_matches(instrument) || !instrument.is_hedge_constructible() {
        return Err(contract_error(
            venue,
            "spot instrument evidence is stale, incomplete, or unofficial",
        ));
    }
    if !shared_types::venue_names_equal(&instrument.venue, venue) {
        return Err(contract_error(venue, "spot instrument venue mismatch"));
    }
    if !(instrument.native_symbol.eq_ignore_ascii_case(symbol)
        || instrument.canonical_symbol.eq_ignore_ascii_case(symbol))
    {
        return Err(contract_error(venue, "spot instrument symbol mismatch"));
    }
    Ok(instrument)
}

fn validate_spot_intent(
    venue: &str,
    intent: &OrderIntent,
    rounded_base_qty: f64,
) -> ExchangeResult<()> {
    if intent.reduce_only {
        return Err(contract_error(
            venue,
            "spot orders do not support reduce_only",
        ));
    }
    if (intent.leverage - 1.0).abs() > f64::EPSILON {
        return Err(contract_error(venue, "spot cash orders require leverage=1"));
    }
    let tolerance = rounded_base_qty.abs().max(1.0) * 1e-9;
    if (intent.quantity - rounded_base_qty).abs() > tolerance {
        return Err(contract_error(
            venue,
            "spot intent quantity differs from ticket-bound sizing",
        ));
    }
    Ok(())
}

fn validate_step(venue: &str, field: &str, value: f64, step: f64) -> ExchangeResult<()> {
    if !(value.is_finite() && value > 0.0 && step.is_finite() && step > 0.0) {
        return Err(contract_error(venue, &format!("invalid spot {field}/step")));
    }
    let units = value / step;
    if (units - units.round()).abs() > units.abs().max(1.0) * 1e-9 {
        return Err(contract_error(
            venue,
            &format!("spot {field} is not aligned to the official step"),
        ));
    }
    Ok(())
}

pub(super) fn decimal_param(venue: &str, field: &str, value: f64) -> ExchangeResult<String> {
    if !value.is_finite() || value <= 0.0 {
        return Err(contract_error(
            venue,
            &format!("spot {field} must be finite and positive"),
        ));
    }
    let formatted = format!("{value:.16}");
    let trimmed = formatted.trim_end_matches('0').trim_end_matches('.');
    if trimmed.is_empty() {
        Err(contract_error(venue, &format!("spot {field} is empty")))
    } else {
        Ok(trimmed.to_owned())
    }
}

fn contract_error(venue: &str, message: &str) -> ExchangeError {
    ExchangeError::Api {
        exchange: venue.to_owned(),
        code: "SPOT_ORDER_CONTRACT_INVALID".to_owned(),
        message: message.to_owned(),
    }
}

#[cfg(test)]
#[path = "spot_order_contract_tests.rs"]
mod tests;
