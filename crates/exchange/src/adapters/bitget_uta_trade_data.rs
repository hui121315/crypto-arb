//! Bitget V3 / UTA order write payload and ack parsing.
//!
//! V3 differences vs V2 (`bitget_trade_data.rs`):
//! - REST path: `POST /api/v3/trade/place-order` / `POST /api/v3/trade/cancel-order`.
//! - Body uses `category` (V2 `productType`), `qty` (V2 `size`) and
//!   `timeInForce` (not V2 `force`). We re-use `BitgetMarginMode` verbatim
//!   from the V2 config module since both worlds use `crossed | isolated`.
//! - Ack response shape is identical (`{orderId, clientOid}` inside
//!   `BitgetObjectResponse<OrderAckRow>`).
//!
//! Official references:
//! - Place order: <https://www.bitget.com/api-doc/uta/trade/Place-Order>
//! - Cancel order: <https://www.bitget.com/api-doc/uta/trade/Cancel-Order>

use crate::adapters::bitget_config::BitgetMarginMode;
use crate::adapters::bitget_order_compiler::CompiledBitgetOrder;
use crate::adapters::bitget_uta_config::BitgetUtaCategory;
use crate::error::{ExchangeError, ExchangeResult};
use common::time::now_ms;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use shared_types::{
    CancelOrderRequest, LiveOrderState, OrderAck, OrderInfo, OrderIntent, OrderSide, OrderStatus,
    OrderType, TimeInForce, VenueOrderIdentityUpdate,
};

const NAME: &str = "bitget";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UtaPlaceOrderBody {
    /// `USDT-FUTURES / USDC-FUTURES / COIN-FUTURES / SPOT` (UTA REST uses
    /// upper-case).
    category: &'static str,
    symbol: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    margin_mode: Option<&'static str>,
    /// V3 renames V2 `size` to `qty`.
    qty: String,
    side: &'static str,
    order_type: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    price: Option<String>,
    #[serde(rename = "timeInForce", skip_serializing_if = "Option::is_none")]
    time_in_force: Option<&'static str>,
    client_oid: String,
    #[serde(rename = "posSide", skip_serializing_if = "Option::is_none")]
    pos_side: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reduce_only: Option<&'static str>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UtaCancelOrderBody {
    category: &'static str,
    symbol: String,
    #[serde(rename = "orderId", skip_serializing_if = "Option::is_none")]
    order_id: Option<String>,
    client_oid: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct UtaOrderAckRow {
    #[serde(default, rename = "orderId")]
    pub(super) order_id: String,
    #[serde(default, rename = "clientOid")]
    pub(super) client_oid: String,
}

struct BitgetUtaOrderShape {
    order_type: &'static str,
    price: Option<String>,
    time_in_force: Option<&'static str>,
}

pub(super) fn place_order_body_json<C>(
    intent: &OrderIntent,
    context: C,
    margin_mode: BitgetMarginMode,
) -> ExchangeResult<String>
where
    C: IntoBitgetOrderContext,
{
    let compiled = context.into_context(intent);
    let body = build_place_order_body(intent, &compiled, margin_mode)?;
    serde_json::to_string(&body)
        .map_err(|error| ExchangeError::Parse(format!("bitget place order body: {error}")))
}

pub(super) fn cancel_order_body_json_for(
    request: &CancelOrderRequest,
    category: BitgetUtaCategory,
    symbol: String,
) -> ExchangeResult<String> {
    let body = UtaCancelOrderBody {
        category: category.as_query(),
        symbol,
        order_id: request.exchange_order_id.clone(),
        client_oid: checked_client_oid(&request.client_order_id)?,
    };
    serde_json::to_string(&body)
        .map_err(|error| ExchangeError::Parse(format!("bitget cancel order body: {error}")))
}

#[cfg(test)]
pub(super) fn cancel_order_body_json(
    request: &CancelOrderRequest,
    symbol: String,
) -> ExchangeResult<String> {
    cancel_order_body_json_for(request, BitgetUtaCategory::UsdtFutures, symbol)
}

/// Place-order params shaped for the V3 WS trade channel.
///
/// V3 WS lifts `category` to the top-level request envelope (see
/// `bitget_uta_ws_trade::WsTradeRequest`), so the per-order args dropped that
/// field; everything else mirrors the REST body verbatim.
pub(super) fn place_order_params<C>(
    intent: &OrderIntent,
    context: C,
    margin_mode: BitgetMarginMode,
) -> ExchangeResult<Value>
where
    C: IntoBitgetOrderContext,
{
    let compiled = context.into_context(intent);
    let body = build_place_order_body(intent, &compiled, margin_mode)?;
    let mut value = serde_json::to_value(body)
        .map_err(|error| ExchangeError::Parse(format!("bitget place order params: {error}")))?;
    if let Value::Object(fields) = &mut value {
        fields.remove("category");
        if let Some(Value::String(reduce_only)) = fields.get_mut("reduceOnly") {
            if reduce_only.eq_ignore_ascii_case("yes") {
                *reduce_only = "YES".to_owned();
            }
        }
    }
    Ok(value)
}

pub(super) trait IntoBitgetOrderContext {
    fn into_context(self, intent: &OrderIntent) -> CompiledBitgetOrder;
}

impl IntoBitgetOrderContext for &CompiledBitgetOrder {
    fn into_context(self, _intent: &OrderIntent) -> CompiledBitgetOrder {
        self.clone()
    }
}

#[cfg(test)]
impl IntoBitgetOrderContext for String {
    fn into_context(self, intent: &OrderIntent) -> CompiledBitgetOrder {
        CompiledBitgetOrder {
            category: BitgetUtaCategory::UsdtFutures,
            native_symbol: self,
            quantity: intent.quantity,
            pos_side: None,
            reduce_only: intent.reduce_only,
        }
    }
}

/// Cancel params shaped for the V3 WS trade channel (no `category`, no
/// `symbol` either — both are carried by the WS request envelope).
pub(super) fn cancel_order_params(
    request: &CancelOrderRequest,
) -> ExchangeResult<Map<String, Value>> {
    let mut params = Map::new();
    if let Some(order_id) = &request.exchange_order_id {
        params.insert("orderId".to_owned(), order_id.clone().into());
    }
    params.insert(
        "clientOid".to_owned(),
        checked_client_oid(&request.client_order_id)?.into(),
    );
    Ok(params)
}

pub(super) fn ack_from_row(
    internal_order_id: String,
    client_order_id: String,
    row: UtaOrderAckRow,
    state: LiveOrderState,
    message: Option<String>,
) -> OrderAck {
    let exchange_order_id = non_empty(row.order_id);
    let venue_client_order_id =
        non_empty(row.client_oid).unwrap_or_else(|| client_order_id.clone());
    OrderAck {
        internal_order_id,
        exchange_order_id: exchange_order_id.clone(),
        client_order_id: client_order_id.clone(),
        identity_update: VenueOrderIdentityUpdate::from_ids(
            client_order_id,
            venue_client_order_id,
            exchange_order_id,
        ),
        state,
        accepted_at_ms: now_ms(),
        message,
        filled_quantity: None,
        filled_price: None,
        filled_fee: None,
    }
}

pub(super) fn ack_from_order_query(
    internal_order_id: String,
    client_order_id: String,
    order: OrderInfo,
) -> OrderAck {
    let exchange_order_id = non_empty(order.order_id);
    let venue_client_order_id = order
        .client_order_id
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| client_order_id.clone());
    OrderAck {
        internal_order_id,
        exchange_order_id: exchange_order_id.clone(),
        client_order_id: client_order_id.clone(),
        identity_update: VenueOrderIdentityUpdate::from_ids(
            client_order_id,
            venue_client_order_id,
            exchange_order_id,
        ),
        state: live_state_from_order_status(order.status),
        accepted_at_ms: now_ms(),
        message: Some(
            "bitget place result recovered by GET /api/v3/trade/order-info using clientOid"
                .to_owned(),
        ),
        filled_quantity: (order.filled_quantity > 0.0).then_some(order.filled_quantity),
        filled_price: (order.filled_price > 0.0).then_some(order.filled_price),
        filled_fee: (order.fees != 0.0).then_some(order.fees),
    }
}

fn live_state_from_order_status(status: OrderStatus) -> LiveOrderState {
    match status {
        OrderStatus::Pending | OrderStatus::Open => LiveOrderState::Accepted,
        OrderStatus::PartiallyFilled => LiveOrderState::PartiallyFilled,
        OrderStatus::Filled => LiveOrderState::Filled,
        OrderStatus::Canceled => LiveOrderState::Cancelled,
        OrderStatus::Rejected => LiveOrderState::Rejected,
        OrderStatus::Expired => LiveOrderState::Failed,
    }
}

fn build_place_order_body(
    intent: &OrderIntent,
    compiled: &CompiledBitgetOrder,
    margin_mode: BitgetMarginMode,
) -> ExchangeResult<UtaPlaceOrderBody> {
    let shape = bitget_order_shape(intent)?;
    Ok(UtaPlaceOrderBody {
        category: compiled.category.as_query(),
        symbol: compiled.native_symbol.clone(),
        margin_mode: (compiled.category != BitgetUtaCategory::Spot).then_some(margin_mode.as_str()),
        qty: number_param(compiled.quantity)?,
        side: bitget_side(intent.side),
        order_type: shape.order_type,
        price: shape.price,
        time_in_force: shape.time_in_force,
        client_oid: checked_client_oid(&intent.client_order_id)?,
        pos_side: (compiled.category != BitgetUtaCategory::Spot)
            .then_some(compiled.pos_side)
            .flatten(),
        reduce_only: (compiled.category != BitgetUtaCategory::Spot && compiled.reduce_only)
            .then_some("yes"),
    })
}

fn bitget_order_shape(intent: &OrderIntent) -> ExchangeResult<BitgetUtaOrderShape> {
    match intent.order_type {
        OrderType::Market => Ok(BitgetUtaOrderShape {
            order_type: "market",
            price: None,
            time_in_force: None,
        }),
        OrderType::Limit => Ok(BitgetUtaOrderShape {
            order_type: "limit",
            price: Some(required_price(intent)?),
            time_in_force: Some(bitget_time_in_force(intent.time_in_force)),
        }),
        OrderType::PostOnly => Ok(BitgetUtaOrderShape {
            order_type: "limit",
            price: Some(required_price(intent)?),
            time_in_force: Some("post_only"),
        }),
    }
}

fn bitget_time_in_force(time_in_force: TimeInForce) -> &'static str {
    match time_in_force {
        TimeInForce::Gtc => "gtc",
        TimeInForce::Ioc => "ioc",
        TimeInForce::Fok => "fok",
        TimeInForce::Gtx => "post_only",
    }
}

fn required_price(intent: &OrderIntent) -> ExchangeResult<String> {
    let price = intent
        .price
        .ok_or_else(|| validation_error("bitget limit order requires price".to_owned()))?;
    number_param(price)
}

pub(super) fn checked_client_oid(value: &str) -> ExchangeResult<String> {
    crate::client_order_id_policy::required_venue_client_order_id(NAME, value)
}

fn bitget_side(side: OrderSide) -> &'static str {
    match side {
        OrderSide::Buy => "buy",
        OrderSide::Sell => "sell",
    }
}

fn number_param(value: f64) -> ExchangeResult<String> {
    if !value.is_finite() || value <= 0.0 {
        return Err(validation_error(format!(
            "bitget invalid positive number: {value}"
        )));
    }
    let text = format!("{value:.12}");
    let trimmed = text.trim_end_matches('0').trim_end_matches('.');
    Ok(trimmed.to_owned())
}

fn non_empty(value: String) -> Option<String> {
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn validation_error(message: String) -> ExchangeError {
    ExchangeError::Api {
        exchange: NAME.into(),
        code: "validation".into(),
        message,
    }
}

#[cfg(test)]
#[path = "bitget_uta_trade_data_tests.rs"]
mod tests;
