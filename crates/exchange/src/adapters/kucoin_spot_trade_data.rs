//! KuCoin Classic Spot request compiler for the Pro WebSocket writer.
//!
//! Official schemas:
//! - <https://www.kucoin.com/docs-new/3470133w0>
//! - <https://www.kucoin.com/docs-new/3470134w0>

use super::kucoin_private_rest::{self, SignedRequest};
use super::kucoin_response::KucoinResponse;
use super::kucoin_trade_data::checked_client_oid;
use super::spot_order_contract::CompiledSpotOrder;
use crate::adapter::strip_common_suffixes;
use crate::error::{ExchangeError, ExchangeResult};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use shared_types::{
    CancelOrderRequest, OrderInfo, OrderIntent, OrderSide, OrderStatus, OrderType, TimeInForce,
};

pub(super) fn query_by_client_path(
    client_order_id: &str,
    native_symbol: &str,
) -> ExchangeResult<String> {
    Ok(format!(
        "/api/v1/hf/orders/client-order/{}?symbol={}",
        encoded(&checked_client_oid(client_order_id)?),
        native_symbol
    ))
}

pub(super) fn query_by_order_path(order_id: &str, native_symbol: &str) -> ExchangeResult<String> {
    let order_id = order_id.trim();
    if order_id.is_empty() || order_id.chars().any(char::is_control) {
        return Err(super::kucoin_trade_data::validation_error(
            "kucoin spot orderId must be non-empty and contain no control characters".to_owned(),
        ));
    }
    Ok(format!(
        "/api/v1/hf/orders/{}?symbol={}",
        encoded(order_id),
        native_symbol
    ))
}

pub(super) async fn query_order(request: &SignedRequest<'_>) -> ExchangeResult<OrderInfo> {
    let response: KucoinResponse<SpotOrderRow> = kucoin_private_rest::signed_get(request).await?;
    order_info(response.into_data("spot get order")?)
}

pub(super) fn place_args(
    intent: &OrderIntent,
    compiled: &CompiledSpotOrder,
) -> ExchangeResult<Value> {
    let mut args = Map::from_iter([
        (
            "clientOid".to_owned(),
            Value::String(checked_client_oid(&intent.client_order_id)?),
        ),
        (
            "symbol".to_owned(),
            Value::String(compiled.native_symbol.clone()),
        ),
        (
            "side".to_owned(),
            Value::String(side(intent.side).to_owned()),
        ),
        ("size".to_owned(), Value::String(compiled.quantity.clone())),
    ]);
    match intent.order_type {
        OrderType::Market => {
            args.insert("type".to_owned(), Value::String("market".to_owned()));
        }
        OrderType::Limit | OrderType::PostOnly => {
            args.insert("type".to_owned(), Value::String("limit".to_owned()));
            args.insert(
                "price".to_owned(),
                Value::String(compiled.price.clone().unwrap_or_default()),
            );
            args.insert(
                "timeInForce".to_owned(),
                Value::String(time_in_force(intent.time_in_force).to_owned()),
            );
            if intent.order_type == OrderType::PostOnly
                || intent.post_only
                || intent.time_in_force == TimeInForce::Gtx
            {
                args.insert("postOnly".to_owned(), Value::Bool(true));
            }
        }
    }
    Ok(Value::Object(args))
}

pub(super) fn cancel_args(
    request: &CancelOrderRequest,
    native_symbol: &str,
) -> ExchangeResult<Value> {
    let mut args = json!({"symbol": native_symbol});
    if let Some(order_id) = request
        .exchange_order_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        args["orderId"] = Value::String(order_id.to_owned());
    } else {
        args["clientOid"] = Value::String(checked_client_oid(&request.client_order_id)?);
    }
    Ok(args)
}

fn side(value: OrderSide) -> &'static str {
    match value {
        OrderSide::Buy => "buy",
        OrderSide::Sell => "sell",
    }
}

fn time_in_force(value: TimeInForce) -> &'static str {
    match value {
        TimeInForce::Gtc | TimeInForce::Gtx => "GTC",
        TimeInForce::Ioc => "IOC",
        TimeInForce::Fok => "FOK",
    }
}

fn order_info(row: SpotOrderRow) -> ExchangeResult<OrderInfo> {
    let size = number("size", &row.size)?;
    let deal_size = number("dealSize", &row.deal_size)?;
    let remain_size = number("remainSize", &row.remain_size)?;
    let deal_funds = number("dealFunds", &row.deal_funds)?;
    let fee = number("fee", &row.fee)?;
    let price = number("price", &row.price)?;
    let quantity = if size > 0.0 {
        size
    } else {
        deal_size + remain_size
    };
    let filled_price = if deal_size > 0.0 {
        deal_funds / deal_size
    } else {
        0.0
    };
    let created_at = DateTime::from_timestamp_millis(row.created_at).unwrap_or_else(Utc::now);
    let status = spot_status(&row, deal_size, remain_size);
    Ok(OrderInfo {
        order_id: row.id,
        symbol: strip_common_suffixes(&row.symbol),
        exchange: "kucoin".to_owned(),
        side: match row.side.as_str() {
            "buy" => OrderSide::Buy,
            "sell" => OrderSide::Sell,
            value => return Err(ExchangeError::Parse(format!("kucoin spot side: {value}"))),
        },
        order_type: if row.order_type == "market" {
            OrderType::Market
        } else if row.post_only {
            OrderType::PostOnly
        } else {
            OrderType::Limit
        },
        status,
        quantity,
        price,
        filled_quantity: deal_size,
        filled_price,
        fees: fee,
        created_at,
        execution_style: None,
        venue_time_in_force: non_empty(row.time_in_force),
        client_order_id: non_empty(row.client_oid),
        reduce_only: Some(false),
    })
}

fn spot_status(row: &SpotOrderRow, deal_size: f64, remain_size: f64) -> OrderStatus {
    if row.active {
        if deal_size > 0.0 {
            OrderStatus::PartiallyFilled
        } else {
            OrderStatus::Open
        }
    } else if row.cancel_exist || number_or_zero(&row.cancelled_size) > 0.0 {
        OrderStatus::Canceled
    } else if deal_size > 0.0 && remain_size <= 0.0 {
        OrderStatus::Filled
    } else {
        OrderStatus::Pending
    }
}

fn number(field: &str, value: &str) -> ExchangeResult<f64> {
    value
        .parse::<f64>()
        .map_err(|error| ExchangeError::Parse(format!("kucoin spot {field}: {error}")))
        .and_then(|parsed| {
            if parsed.is_finite() && parsed >= 0.0 {
                Ok(parsed)
            } else {
                Err(ExchangeError::Parse(format!(
                    "kucoin spot {field} is invalid: {value}"
                )))
            }
        })
}

fn number_or_zero(value: &str) -> f64 {
    value.parse::<f64>().unwrap_or(0.0)
}

fn non_empty(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
}

fn encoded(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SpotOrderRow {
    id: String,
    symbol: String,
    #[serde(rename = "type")]
    order_type: String,
    side: String,
    #[serde(default)]
    price: String,
    #[serde(default)]
    size: String,
    #[serde(default)]
    deal_size: String,
    #[serde(default)]
    deal_funds: String,
    #[serde(default)]
    fee: String,
    #[serde(default)]
    time_in_force: String,
    #[serde(default)]
    post_only: bool,
    #[serde(default)]
    client_oid: String,
    #[serde(default)]
    cancel_exist: bool,
    #[serde(default)]
    cancelled_size: String,
    #[serde(default)]
    remain_size: String,
    #[serde(default)]
    active: bool,
    #[serde(default)]
    created_at: i64,
}

#[cfg(test)]
#[path = "kucoin_spot_trade_data_tests.rs"]
mod tests;
