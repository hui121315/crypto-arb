//! Binance USD-M futures order write payload and ack parsing.
//!
//! Official Binance USD-M docs checked before moving these helpers:
//! - REST POST /fapi/v1/order
//! - REST DELETE /fapi/v1/order
//! - WebSocket `order.place`
//! - WebSocket `order.cancel`

use super::binance_exchange_info::{
    BinanceInstrumentSpec, BinanceOrderConstraints, BinanceQuantityConstraints,
};
use super::binance_format::{
    binance_order_type, binance_side, binance_time_in_force, number_param,
};
use super::binance_private_data::OpenOrderItem;
use crate::error::{ExchangeError, ExchangeResult};
use common::time::now_ms;
use shared_types::{
    LiveOrderState, OrderAck, OrderIntent, OrderSide, OrderType, VenueOrderIdentityUpdate,
};

const NAME: &str = "binance";

pub(super) type RestOrderParams = Vec<(String, String)>;

pub(super) fn rest_place_order_params(
    intent: &OrderIntent,
    symbol: &str,
    position_side: &str,
) -> ExchangeResult<RestOrderParams> {
    validate_client_order_id(&intent.client_order_id)?;
    validate_position_side(intent, position_side)?;
    let mut params = vec![
        ("symbol".to_owned(), symbol.to_owned()),
        ("side".to_owned(), binance_side(intent.side).to_owned()),
        ("positionSide".to_owned(), position_side.to_owned()),
        (
            "type".to_owned(),
            binance_order_type(intent.order_type).to_owned(),
        ),
        ("quantity".to_owned(), number_param(intent.quantity)),
        (
            "newClientOrderId".to_owned(),
            intent.client_order_id.clone(),
        ),
        ("recvWindow".to_owned(), "5000".to_owned()),
        ("newOrderRespType".to_owned(), "RESULT".to_owned()),
    ];

    match intent.order_type {
        OrderType::Limit | OrderType::PostOnly => {
            if let Some(time_in_force) =
                binance_time_in_force(intent.order_type, intent.time_in_force)
            {
                params.push(("timeInForce".to_owned(), time_in_force.to_owned()));
            }
            params.push(("price".to_owned(), required_price(intent)?));
        }
        OrderType::Market => {}
    }

    if intent.reduce_only && position_side == "BOTH" {
        params.push(("reduceOnly".to_owned(), "true".to_owned()));
    }

    Ok(params)
}

pub(super) fn rest_cancel_order_params(
    request_client_order_id: &str,
    symbol: &str,
) -> RestOrderParams {
    vec![
        ("symbol".to_owned(), symbol.to_owned()),
        (
            "origClientOrderId".to_owned(),
            request_client_order_id.to_owned(),
        ),
        ("recvWindow".to_owned(), "5000".to_owned()),
    ]
}

pub(super) fn safe_cancel_probe_params(
    request_client_order_id: &str,
    symbol: &str,
) -> RestOrderParams {
    rest_cancel_order_params(request_client_order_id, symbol)
}

pub(super) fn param_refs(params: &RestOrderParams) -> Vec<(&str, &str)> {
    params
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect()
}

pub(super) fn validate_order_constraints(
    symbol: &str,
    intent: &OrderIntent,
    constraints: &BinanceOrderConstraints,
) -> ExchangeResult<()> {
    validate_symbol_assets(symbol, constraints)?;
    validate_order_capabilities(symbol, intent, constraints)?;
    let (qty_filter, qty_constraints) = quantity_constraints(intent.order_type, constraints);
    validate_quantity_filter_present(symbol, qty_filter, qty_constraints)?;
    let qty_field = format!("{qty_filter} quantity");
    validate_range(
        symbol,
        &qty_field,
        intent.quantity,
        qty_constraints.min_qty,
        qty_constraints.max_qty,
    )?;
    validate_step(
        symbol,
        &qty_field,
        intent.quantity,
        qty_constraints.step_size,
    )?;

    match intent.order_type {
        OrderType::Limit | OrderType::PostOnly => {
            let price = intent.price.unwrap_or(0.0);
            validate_range(
                symbol,
                "price",
                price,
                constraints.price.min_price,
                constraints.price.max_price,
            )?;
            validate_step(symbol, "price", price, constraints.price.tick_size)?;
            if let Some(min_notional) = constraints.min_notional {
                let notional = intent.quantity * price;
                if notional + 1e-9 < min_notional {
                    return Err(validation_error(format!(
                        "{symbol} order notional {notional} below minNotional {min_notional}"
                    )));
                }
            }
        }
        OrderType::Market => {}
    }

    Ok(())
}

pub(super) fn validate_order_spec(
    intent: &OrderIntent,
    spec: &BinanceInstrumentSpec,
) -> ExchangeResult<()> {
    validate_spec_identity(spec)?;
    validate_order_constraints(&spec.native_symbol, intent, &spec.constraints)?;
    validate_required_sizing(intent, spec)
}

fn validate_spec_identity(spec: &BinanceInstrumentSpec) -> ExchangeResult<()> {
    let identity = &spec.constraints.identity;
    let quote = identity.quote_asset.as_str();
    if quote != "USDT" && quote != "USDC" {
        return Err(validation_error(format!(
            "{} has unsupported Binance USD-M quoteAsset {quote}",
            spec.native_symbol
        )));
    }
    if identity.margin_asset != quote {
        return Err(validation_error(format!(
            "{} marginAsset {} does not match quoteAsset {quote}",
            spec.native_symbol, identity.margin_asset
        )));
    }
    if !(spec.contract_size.is_finite() && (spec.contract_size - 1.0).abs() <= f64::EPSILON) {
        return Err(validation_error(format!(
            "{} has invalid USD-M contract size {}",
            spec.native_symbol, spec.contract_size
        )));
    }
    Ok(())
}

fn validate_required_sizing(
    intent: &OrderIntent,
    spec: &BinanceInstrumentSpec,
) -> ExchangeResult<()> {
    let constraints = &spec.constraints;
    let (_, quantity) = quantity_constraints(intent.order_type, constraints);
    require_positive_metadata(&spec.native_symbol, "quantity step", quantity.step_size)?;
    require_positive_metadata(
        &spec.native_symbol,
        "min notional",
        constraints.min_notional,
    )?;
    if matches!(intent.order_type, OrderType::Limit | OrderType::PostOnly) {
        require_positive_metadata(
            &spec.native_symbol,
            "price tick",
            constraints.price.tick_size,
        )?;
    }
    Ok(())
}

fn require_positive_metadata(symbol: &str, field: &str, value: Option<f64>) -> ExchangeResult<()> {
    if value.is_some_and(|value| value.is_finite() && value > 0.0) {
        return Ok(());
    }
    Err(validation_error(format!(
        "{symbol} is missing valid {field} in Binance exchangeInfo"
    )))
}

fn validate_order_capabilities(
    symbol: &str,
    intent: &OrderIntent,
    constraints: &BinanceOrderConstraints,
) -> ExchangeResult<()> {
    let capabilities = &constraints.capabilities;
    if !capabilities.is_trading {
        return Err(validation_error(format!(
            "{symbol} is not TRADING in Binance exchangeInfo"
        )));
    }
    if !capabilities.is_perpetual {
        return Err(validation_error(format!(
            "{symbol} is not PERPETUAL in Binance exchangeInfo"
        )));
    }

    match intent.order_type {
        OrderType::Market => require_capability(symbol, "MARKET", capabilities.supports_market)?,
        OrderType::Limit | OrderType::PostOnly => {
            require_capability(symbol, "LIMIT", capabilities.supports_limit)?;
            if let Some(time_in_force) =
                binance_time_in_force(intent.order_type, intent.time_in_force)
            {
                require_capability(
                    symbol,
                    time_in_force,
                    supports_time_in_force(constraints, time_in_force),
                )?;
            }
        }
    }

    Ok(())
}

fn validate_symbol_assets(
    symbol: &str,
    constraints: &BinanceOrderConstraints,
) -> ExchangeResult<()> {
    let identity = &constraints.identity;
    if identity.base_asset.is_empty()
        || identity.quote_asset.is_empty()
        || identity.margin_asset.is_empty()
    {
        return Err(validation_error(format!(
            "{symbol} is missing baseAsset/quoteAsset/marginAsset in Binance exchangeInfo"
        )));
    }

    let expected = format!("{}{}", identity.base_asset, identity.quote_asset);
    if !symbol.eq_ignore_ascii_case(&expected) {
        return Err(validation_error(format!(
            "{symbol} does not match Binance exchangeInfo baseAsset/quoteAsset {expected}"
        )));
    }

    Ok(())
}

fn supports_time_in_force(constraints: &BinanceOrderConstraints, time_in_force: &str) -> bool {
    let capabilities = &constraints.capabilities;
    match time_in_force {
        "GTC" => capabilities.supports_gtc,
        "IOC" => capabilities.supports_ioc,
        "FOK" => capabilities.supports_fok,
        "GTX" => capabilities.supports_gtx,
        _ => false,
    }
}

fn quantity_constraints(
    order_type: OrderType,
    constraints: &BinanceOrderConstraints,
) -> (&'static str, &BinanceQuantityConstraints) {
    match order_type {
        OrderType::Market => ("MARKET_LOT_SIZE", &constraints.market_qty),
        OrderType::Limit | OrderType::PostOnly => ("LOT_SIZE", &constraints.limit_qty),
    }
}

fn validate_quantity_filter_present(
    symbol: &str,
    filter_name: &str,
    constraints: &BinanceQuantityConstraints,
) -> ExchangeResult<()> {
    if constraints.present {
        return Ok(());
    }
    Err(validation_error(format!(
        "{symbol} is missing {filter_name} in Binance exchangeInfo"
    )))
}

fn require_capability(symbol: &str, capability: &str, supported: bool) -> ExchangeResult<()> {
    if supported {
        return Ok(());
    }
    Err(validation_error(format!(
        "{symbol} does not support {capability} in Binance exchangeInfo"
    )))
}

pub(super) fn ack_from_order_item(
    internal_order_id: String,
    client_order_id: String,
    item: &OpenOrderItem,
) -> OrderAck {
    let exchange_order_id = Some(item.order_id.to_string());
    OrderAck {
        internal_order_id,
        exchange_order_id: exchange_order_id.clone(),
        client_order_id: client_order_id.clone(),
        identity_update: VenueOrderIdentityUpdate::from_ids(
            client_order_id.clone(),
            client_order_id,
            exchange_order_id,
        ),
        state: live_state_from_status(&item.status),
        accepted_at_ms: now_ms(),
        message: None,
        filled_quantity: None,
        filled_price: None,
        filled_fee: None,
    }
}

pub(super) fn validation_error(message: String) -> ExchangeError {
    ExchangeError::Api {
        exchange: NAME.into(),
        code: "validation".into(),
        message,
    }
}

pub(super) fn validate_client_order_id(client_order_id: &str) -> ExchangeResult<()> {
    crate::client_order_id_policy::validate_client_order_id_policy(NAME, client_order_id)
}

pub(super) fn validate_position_side(
    intent: &OrderIntent,
    position_side: &str,
) -> ExchangeResult<()> {
    match position_side {
        "BOTH" | "LONG" | "SHORT" => {}
        _ => {
            return Err(validation_error(format!(
                "binance positionSide must be BOTH, LONG, or SHORT from verified account mode: {position_side}"
            )))
        }
    }
    if intent.reduce_only {
        let closes_verified_side = matches!(
            (position_side, intent.side),
            ("BOTH", _) | ("LONG", OrderSide::Sell) | ("SHORT", OrderSide::Buy)
        );
        if !closes_verified_side {
            return Err(validation_error(format!(
                "binance hedge close side mismatch: order side {} cannot close positionSide {position_side}",
                binance_side(intent.side)
            )));
        }
    }
    Ok(())
}

fn required_price(intent: &OrderIntent) -> ExchangeResult<String> {
    let price = intent
        .price
        .ok_or_else(|| validation_error("binance limit order requires price".to_owned()))?;
    if !(price.is_finite() && price > 0.0) {
        return Err(validation_error(format!(
            "binance invalid positive price: {price}"
        )));
    }
    Ok(number_param(price))
}

fn validate_range(
    symbol: &str,
    field: &str,
    value: f64,
    min: Option<f64>,
    max: Option<f64>,
) -> ExchangeResult<()> {
    if let Some(min) = min {
        if value + 1e-12 < min {
            return Err(validation_error(format!(
                "{symbol} {field} {value} below minimum {min}"
            )));
        }
    }
    if let Some(max) = max {
        if value - 1e-12 > max {
            return Err(validation_error(format!(
                "{symbol} {field} {value} above maximum {max}"
            )));
        }
    }
    Ok(())
}

fn validate_step(symbol: &str, field: &str, value: f64, step: Option<f64>) -> ExchangeResult<()> {
    let Some(step) = step else {
        return Ok(());
    };
    if step <= 0.0 {
        return Ok(());
    }

    let units = value / step;
    if (units - units.round()).abs() > 1e-8 {
        return Err(validation_error(format!(
            "{symbol} {field} {value} does not align to step {step}"
        )));
    }
    Ok(())
}

fn live_state_from_status(status: &str) -> LiveOrderState {
    match status.to_ascii_uppercase().as_str() {
        "NEW" => LiveOrderState::Accepted,
        "PARTIALLY_FILLED" => LiveOrderState::PartiallyFilled,
        "FILLED" => LiveOrderState::Filled,
        "CANCELED" => LiveOrderState::Cancelled,
        "REJECTED" => LiveOrderState::Rejected,
        "EXPIRED" => LiveOrderState::Failed,
        _ => LiveOrderState::Unknown,
    }
}

#[cfg(test)]
#[path = "binance_trade_data_tests.rs"]
mod tests;
