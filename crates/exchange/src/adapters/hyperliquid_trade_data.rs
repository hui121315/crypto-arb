//! Hyperliquid write-action payload construction.
//!
//! Official Hyperliquid exchange endpoint docs checked before moving this code:
//! - order action fields `a` / `b` / `p` / `s` / `r` / `t.limit.tif` / `c`
//! - cancel action by exchange oid
//! - cancelByCloid action
//! - cloid 128-bit hex format

use crate::adapters::hyperliquid_instruments::HyperliquidInstrumentSpec;
use crate::error::{ExchangeError, ExchangeResult};
use serde_json::{json, Value};
use shared_types::{CancelOrderRequest, OrderIntent, OrderSide, OrderType, TimeInForce};

const NAME: &str = "hyperliquid";
pub(super) const EXCHANGE_PATH: &str = "/exchange";

pub(super) fn hyperliquid_order_action(
    spec: &HyperliquidInstrumentSpec,
    intent: &OrderIntent,
) -> ExchangeResult<Value> {
    validate_instrument_spec(spec)?;
    let quantity = validated_quantity(intent.quantity, spec)?;
    let shape = hyperliquid_order_shape(intent, spec)?;
    Ok(json!({
        "type": "order",
        "orders": [{
            "a": spec.asset_id,
            "b": matches!(intent.side, OrderSide::Buy),
            "p": shape.price,
            "s": quantity,
            "r": intent.reduce_only,
            "t": {"limit": {"tif": shape.tif}},
            "c": hyperliquid_venue_cloid(&intent.client_order_id)?,
        }],
        "grouping": "na",
    }))
}

pub(super) fn hyperliquid_cancel_action(
    asset_id: u32,
    request: &CancelOrderRequest,
) -> ExchangeResult<Value> {
    if let Some(order_id) = request
        .exchange_order_id
        .as_deref()
        .and_then(|value| value.parse::<i64>().ok())
    {
        return Ok(json!({
            "type": "cancel",
            "cancels": [{"a": asset_id, "o": order_id}],
        }));
    }
    Ok(json!({
        "type": "cancelByCloid",
        "cancels": [{
            "asset": asset_id,
            "cloid": hyperliquid_venue_cloid(&request.client_order_id)?,
        }],
    }))
}

pub(super) fn required_cloid(client_order_id: &str) -> ExchangeResult<String> {
    hyperliquid_venue_cloid(client_order_id)
}

pub(super) fn hyperliquid_venue_cloid(client_order_id: &str) -> ExchangeResult<String> {
    crate::client_order_id_policy::required_venue_client_order_id(NAME, client_order_id)
}

fn hyperliquid_order_shape(
    intent: &OrderIntent,
    spec: &HyperliquidInstrumentSpec,
) -> ExchangeResult<HyperliquidOrderShape> {
    // Hyperliquid exchange endpoint exposes orders as limit actions with
    // `tif` = Alo/Ioc/Gtc. Market intent is represented as a protected-price
    // IOC limit because the exchange action schema is limit-or-trigger shaped.
    match intent.order_type {
        OrderType::Limit => Ok(HyperliquidOrderShape {
            price: required_price(intent, spec)?,
            tif: hyperliquid_limit_tif(intent.time_in_force)?,
        }),
        OrderType::PostOnly => Ok(HyperliquidOrderShape {
            price: required_price(intent, spec)?,
            tif: "Alo",
        }),
        OrderType::Market => {
            if intent.time_in_force != TimeInForce::Ioc {
                return Err(validation_error(
                    "hyperliquid protected market order requires ioc time_in_force".to_owned(),
                ));
            }
            Ok(HyperliquidOrderShape {
                price: required_price(intent, spec)?,
                tif: "Ioc",
            })
        }
    }
}

fn hyperliquid_limit_tif(time_in_force: TimeInForce) -> ExchangeResult<&'static str> {
    // Hyperliquid limit `tif` only supports Gtc/Ioc/Alo. It has no FOK, and
    // post-only (Alo) is modeled through `OrderType::PostOnly`, so a limit
    // intent carrying Fok/Gtx is fail-closed instead of silently downgraded.
    match time_in_force {
        TimeInForce::Gtc => Ok("Gtc"),
        TimeInForce::Ioc => Ok("Ioc"),
        TimeInForce::Fok => Err(validation_error(
            "hyperliquid has no fok tif; supported limit tif: gtc/ioc or post-only order_type"
                .to_owned(),
        )),
        TimeInForce::Gtx => Err(validation_error(
            "hyperliquid has no gtx tif; use post-only order_type for official alo".to_owned(),
        )),
    }
}

struct HyperliquidOrderShape {
    price: String,
    tif: &'static str,
}

fn validate_instrument_spec(spec: &HyperliquidInstrumentSpec) -> ExchangeResult<()> {
    if !spec.is_trading {
        return Err(validation_error(format!(
            "hyperliquid instrument {} is not trading",
            spec.canonical_symbol
        )));
    }
    if !(positive(spec.price_tick) && spec.price_decimals <= 8) {
        return Err(validation_error(format!(
            "hyperliquid instrument {} is missing valid price metadata",
            spec.canonical_symbol
        )));
    }
    if !(positive(spec.qty_step)
        && positive(spec.min_qty)
        && spec.size_decimals <= 6
        && spec.min_qty + f64::EPSILON >= spec.qty_step)
    {
        return Err(validation_error(format!(
            "hyperliquid instrument {} is missing valid lot metadata",
            spec.canonical_symbol
        )));
    }
    Ok(())
}

fn required_price(
    intent: &OrderIntent,
    spec: &HyperliquidInstrumentSpec,
) -> ExchangeResult<String> {
    let price = intent
        .price
        .ok_or_else(|| validation_error("hyperliquid limit order requires price".to_owned()))?;
    let price = positive_number(price, "price")?;
    if decimal_places(&price) > usize::from(spec.price_decimals) {
        return Err(validation_error(format!(
            "hyperliquid price {price} exceeds {} decimal places for {}",
            spec.price_decimals, spec.canonical_symbol
        )));
    }
    if price.contains('.') && significant_figures(&price) > 5 {
        return Err(validation_error(format!(
            "hyperliquid price {price} exceeds the official five significant figure limit"
        )));
    }
    Ok(price)
}

fn validated_quantity(value: f64, spec: &HyperliquidInstrumentSpec) -> ExchangeResult<String> {
    let quantity = positive_number(value, "quantity")?;
    if decimal_places(&quantity) > usize::from(spec.size_decimals) {
        return Err(validation_error(format!(
            "hyperliquid quantity {quantity} exceeds {} decimal places for {}",
            spec.size_decimals, spec.canonical_symbol
        )));
    }
    if value + (spec.min_qty * 1e-9) < spec.min_qty {
        return Err(validation_error(format!(
            "hyperliquid quantity {quantity} is below the official lot minimum {}",
            spec.min_qty
        )));
    }
    Ok(quantity)
}

fn decimal_places(value: &str) -> usize {
    value
        .split_once('.')
        .map_or(0, |(_, decimals)| decimals.len())
}

fn significant_figures(value: &str) -> usize {
    value
        .bytes()
        .filter(u8::is_ascii_digit)
        .skip_while(|digit| *digit == b'0')
        .count()
}

fn positive_number(value: f64, field: &str) -> ExchangeResult<String> {
    if !value.is_finite() || value <= 0.0 {
        return Err(validation_error(format!(
            "hyperliquid invalid positive {field}: {value}"
        )));
    }
    let text = format!("{value:.12}");
    Ok(text.trim_end_matches('0').trim_end_matches('.').to_owned())
}

fn positive(value: f64) -> bool {
    value.is_finite() && value > 0.0
}

fn validation_error(message: String) -> ExchangeError {
    ExchangeError::Api {
        exchange: NAME.into(),
        code: "validation".into(),
        message,
    }
}

#[cfg(test)]
#[path = "hyperliquid_trade_data_tests.rs"]
mod tests;
