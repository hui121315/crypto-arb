//! OKX trade request and acknowledgement mapping.
//!
//! Official OKX V5 docs checked before moving this code:
//! - `POST /api/v5/trade/order`
//! - `POST /api/v5/trade/cancel-order`
//! - private WebSocket `order` / `cancel-order`

use super::okx_instruments::OkxOrderSizing;
use super::okx_live_config::OkxTdMode;
use crate::error::{ExchangeError, ExchangeResult};
use serde::{Deserialize, Serialize};
use shared_types::{
    CancelOrderRequest, LiveOrderState, OrderAck, OrderIntent, OrderSide, OrderType, TimeInForce,
    VenueOrderIdentityUpdate,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OkxPositionMode {
    Net,
    LongShort,
}

impl OkxPositionMode {
    pub(super) fn from_pos_mode(value: &str) -> ExchangeResult<Self> {
        match value {
            "net_mode" => Ok(Self::Net),
            "long_short_mode" => Ok(Self::LongShort),
            _ => Err(validation_error(format!(
                "okx unsupported posMode: {value}"
            ))),
        }
    }

    pub(super) fn as_account_mode(self) -> &'static str {
        match self {
            Self::Net => "net_mode",
            Self::LongShort => "long_short_mode",
        }
    }

    pub(super) fn pos_side_for_intent(self, intent: &OrderIntent) -> &'static str {
        match (self, intent.reduce_only, intent.side) {
            (Self::Net, _, _) => "net",
            (Self::LongShort, false, OrderSide::Buy) => "long",
            (Self::LongShort, false, OrderSide::Sell) => "short",
            (Self::LongShort, true, OrderSide::Buy) => "short",
            (Self::LongShort, true, OrderSide::Sell) => "long",
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PlaceOrderBody {
    #[serde(rename = "instId")]
    inst_id: String,
    #[serde(rename = "tdMode")]
    td_mode: &'static str,
    side: &'static str,
    #[serde(rename = "posSide", skip_serializing_if = "Option::is_none")]
    pos_side: Option<&'static str>,
    #[serde(rename = "ordType")]
    ord_type: &'static str,
    sz: String,
    #[serde(rename = "clOrdId")]
    cl_ord_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    px: Option<String>,
    #[serde(rename = "reduceOnly", skip_serializing_if = "Option::is_none")]
    reduce_only: Option<bool>,
    #[serde(rename = "tgtCcy", skip_serializing_if = "Option::is_none")]
    target_currency: Option<&'static str>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CancelOrderBody {
    #[serde(rename = "instId")]
    inst_id: String,
    #[serde(rename = "clOrdId")]
    cl_ord_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WsPlaceOrderBody {
    #[serde(rename = "instIdCode", skip_serializing_if = "Option::is_none")]
    inst_id_code: Option<u64>,
    #[serde(rename = "instId", skip_serializing_if = "Option::is_none")]
    inst_id: Option<String>,
    #[serde(rename = "tdMode")]
    td_mode: &'static str,
    side: &'static str,
    #[serde(rename = "posSide", skip_serializing_if = "Option::is_none")]
    pos_side: Option<&'static str>,
    #[serde(rename = "ordType")]
    ord_type: &'static str,
    sz: String,
    #[serde(rename = "clOrdId")]
    cl_ord_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    px: Option<String>,
    #[serde(rename = "reduceOnly", skip_serializing_if = "Option::is_none")]
    reduce_only: Option<bool>,
    #[serde(rename = "tgtCcy", skip_serializing_if = "Option::is_none")]
    target_currency: Option<&'static str>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WsCancelOrderBody {
    #[serde(rename = "instIdCode", skip_serializing_if = "Option::is_none")]
    inst_id_code: Option<u64>,
    #[serde(rename = "instId", skip_serializing_if = "Option::is_none")]
    inst_id: Option<String>,
    #[serde(rename = "clOrdId")]
    cl_ord_id: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct AccountConfigRow {
    #[serde(default, rename = "posMode")]
    pos_mode: String,
}

pub(super) fn parse_position_mode(row: &AccountConfigRow) -> ExchangeResult<OkxPositionMode> {
    OkxPositionMode::from_pos_mode(&row.pos_mode)
        .map_err(|error| ExchangeError::Parse(format!("okx account config: {error}")))
}

#[derive(Debug, Deserialize)]
pub(super) struct OrderAckItem {
    #[serde(default, rename = "ordId")]
    ord_id: String,
    #[serde(default, rename = "clOrdId")]
    cl_ord_id: String,
    #[serde(default, rename = "sCode")]
    s_code: String,
    #[serde(default, rename = "sMsg")]
    s_msg: String,
}

pub(super) fn ack_from_item(
    internal_order_id: String,
    client_order_id: String,
    item: OrderAckItem,
) -> OrderAck {
    let exchange_order_id = (!item.ord_id.is_empty()).then_some(item.ord_id);
    let venue_client_order_id = if item.cl_ord_id.is_empty() {
        client_order_id.clone()
    } else {
        item.cl_ord_id
    };
    let identity_update = VenueOrderIdentityUpdate::from_ids(
        client_order_id.clone(),
        venue_client_order_id,
        exchange_order_id.clone(),
    );
    // Fail closed: an order counts as Accepted only when OKX confirms a
    // per-item sCode of "0" AND returns an exchange order id. A missing sCode or
    // a sCode=0 without an ordId is an unconfirmed ack and must never surface as
    // a real acceptance/finality.
    if item.s_code == "0" && exchange_order_id.is_some() {
        return OrderAck {
            internal_order_id,
            exchange_order_id,
            client_order_id,
            identity_update,
            state: LiveOrderState::Accepted,
            accepted_at_ms: common::time::now_ms(),
            message: None,
            filled_quantity: None,
            filled_price: None,
            filled_fee: None,
        };
    }
    let message = if item.s_code.is_empty() {
        "okx order ack missing sCode; treating as unconfirmed".to_owned()
    } else if item.s_code != "0" {
        format!("{} {}", item.s_code, item.s_msg).trim().to_owned()
    } else {
        "okx order ack reported sCode=0 without ordId; treating as unconfirmed".to_owned()
    };
    OrderAck {
        internal_order_id,
        exchange_order_id: None,
        client_order_id,
        identity_update,
        state: LiveOrderState::Rejected,
        accepted_at_ms: common::time::now_ms(),
        message: Some(message),
        filled_quantity: None,
        filled_price: None,
        filled_fee: None,
    }
}

pub(super) fn place_order_body_json(
    intent: &OrderIntent,
    inst_id: String,
    td_mode: OkxTdMode,
    position_mode: OkxPositionMode,
    sizing: OkxOrderSizing,
) -> ExchangeResult<String> {
    serde_json::to_string(&build_place_order_body(
        intent,
        inst_id,
        td_mode,
        position_mode,
        sizing,
    )?)
    .map_err(|error| ExchangeError::Parse(format!("okx place order body: {error}")))
}

pub(super) fn pre_check_order_body_json(
    intent: &OrderIntent,
    inst_id: String,
    td_mode: OkxTdMode,
    position_mode: OkxPositionMode,
    sizing: OkxOrderSizing,
) -> ExchangeResult<String> {
    serde_json::to_string(&build_place_order_body(
        intent,
        inst_id,
        td_mode,
        position_mode,
        sizing,
    )?)
    .map_err(|error| ExchangeError::Parse(format!("okx order pre-check body: {error}")))
}

pub(super) fn cancel_order_body_json(
    request: &CancelOrderRequest,
    inst_id: String,
) -> ExchangeResult<String> {
    serde_json::to_string(&build_cancel_order_body(request, inst_id)?)
        .map_err(|error| ExchangeError::Parse(format!("okx cancel order body: {error}")))
}

pub(super) fn place_spot_order_body_json(
    intent: &OrderIntent,
    inst_id: String,
    quantity: String,
    price: Option<String>,
) -> ExchangeResult<String> {
    validate_client_order_id(&intent.client_order_id)?;
    serde_json::to_string(&PlaceOrderBody {
        inst_id,
        td_mode: "cash",
        side: okx_side(intent.side),
        pos_side: None,
        ord_type: okx_order_type(intent),
        sz: quantity,
        cl_ord_id: intent.client_order_id.clone(),
        px: price,
        reduce_only: None,
        target_currency: matches!(intent.order_type, OrderType::Market).then_some("base_ccy"),
    })
    .map_err(|error| ExchangeError::Parse(format!("okx spot place order body: {error}")))
}

#[cfg(test)]
pub(super) fn place_order_arg(
    intent: &OrderIntent,
    inst_id: String,
    td_mode: OkxTdMode,
    position_mode: OkxPositionMode,
    sizing: OkxOrderSizing,
) -> ExchangeResult<serde_json::Value> {
    serde_json::to_value(build_place_order_body(
        intent,
        inst_id,
        td_mode,
        position_mode,
        sizing,
    )?)
    .map_err(|error| ExchangeError::Parse(format!("okx place order arg: {error}")))
}

#[cfg(test)]
pub(super) fn cancel_order_arg(
    request: &CancelOrderRequest,
    inst_id: String,
) -> ExchangeResult<serde_json::Value> {
    serde_json::to_value(build_cancel_order_body(request, inst_id)?)
        .map_err(|error| ExchangeError::Parse(format!("okx cancel order arg: {error}")))
}

pub(super) fn place_order_ws_arg(
    intent: &OrderIntent,
    inst_id_code: u64,
    td_mode: OkxTdMode,
    position_mode: OkxPositionMode,
    sizing: OkxOrderSizing,
) -> ExchangeResult<serde_json::Value> {
    serde_json::to_value(build_ws_place_order_body(
        intent,
        inst_id_code,
        td_mode,
        position_mode,
        sizing,
    )?)
    .map_err(|error| ExchangeError::Parse(format!("okx ws place order arg: {error}")))
}

pub(super) fn cancel_order_ws_arg(
    request: &CancelOrderRequest,
    inst_id_code: u64,
) -> ExchangeResult<serde_json::Value> {
    serde_json::to_value(build_ws_cancel_order_body(request, inst_id_code)?)
        .map_err(|error| ExchangeError::Parse(format!("okx ws cancel order arg: {error}")))
}

pub(super) fn place_spot_order_ws_arg(
    intent: &OrderIntent,
    inst_id: String,
    quantity: String,
    price: Option<String>,
) -> ExchangeResult<serde_json::Value> {
    validate_client_order_id(&intent.client_order_id)?;
    serde_json::to_value(WsPlaceOrderBody {
        inst_id_code: None,
        inst_id: Some(inst_id),
        td_mode: "cash",
        side: okx_side(intent.side),
        pos_side: None,
        ord_type: okx_order_type(intent),
        sz: quantity,
        cl_ord_id: intent.client_order_id.clone(),
        px: price,
        reduce_only: None,
        target_currency: matches!(intent.order_type, OrderType::Market).then_some("base_ccy"),
    })
    .map_err(|error| ExchangeError::Parse(format!("okx spot ws place order arg: {error}")))
}

pub(super) fn cancel_spot_order_ws_arg(
    request: &CancelOrderRequest,
    inst_id: String,
) -> ExchangeResult<serde_json::Value> {
    validate_client_order_id(&request.client_order_id)?;
    serde_json::to_value(WsCancelOrderBody {
        inst_id_code: None,
        inst_id: Some(inst_id),
        cl_ord_id: request.client_order_id.clone(),
    })
    .map_err(|error| ExchangeError::Parse(format!("okx spot ws cancel order arg: {error}")))
}

fn build_place_order_body(
    intent: &OrderIntent,
    inst_id: String,
    td_mode: OkxTdMode,
    position_mode: OkxPositionMode,
    sizing: OkxOrderSizing,
) -> ExchangeResult<PlaceOrderBody> {
    validate_client_order_id(&intent.client_order_id)?;
    let pos_side = position_mode.pos_side_for_intent(intent);
    Ok(PlaceOrderBody {
        inst_id,
        td_mode: td_mode.as_str(),
        side: okx_side(intent.side),
        pos_side: Some(pos_side),
        ord_type: okx_order_type(intent),
        sz: sizing.sz,
        cl_ord_id: intent.client_order_id.clone(),
        px: sizing.px,
        reduce_only: (intent.reduce_only && position_mode == OkxPositionMode::Net).then_some(true),
        target_currency: None,
    })
}

fn build_cancel_order_body(
    request: &CancelOrderRequest,
    inst_id: String,
) -> ExchangeResult<CancelOrderBody> {
    validate_client_order_id(&request.client_order_id)?;
    Ok(CancelOrderBody {
        inst_id,
        cl_ord_id: request.client_order_id.clone(),
    })
}

fn build_ws_place_order_body(
    intent: &OrderIntent,
    inst_id_code: u64,
    td_mode: OkxTdMode,
    position_mode: OkxPositionMode,
    sizing: OkxOrderSizing,
) -> ExchangeResult<WsPlaceOrderBody> {
    validate_client_order_id(&intent.client_order_id)?;
    let pos_side = position_mode.pos_side_for_intent(intent);
    Ok(WsPlaceOrderBody {
        inst_id_code: Some(inst_id_code),
        inst_id: None,
        td_mode: td_mode.as_str(),
        side: okx_side(intent.side),
        pos_side: Some(pos_side),
        ord_type: okx_order_type(intent),
        sz: sizing.sz,
        cl_ord_id: intent.client_order_id.clone(),
        px: sizing.px,
        reduce_only: (intent.reduce_only && position_mode == OkxPositionMode::Net).then_some(true),
        target_currency: None,
    })
}

fn build_ws_cancel_order_body(
    request: &CancelOrderRequest,
    inst_id_code: u64,
) -> ExchangeResult<WsCancelOrderBody> {
    validate_client_order_id(&request.client_order_id)?;
    Ok(WsCancelOrderBody {
        inst_id_code: Some(inst_id_code),
        inst_id: None,
        cl_ord_id: request.client_order_id.clone(),
    })
}

fn okx_side(side: OrderSide) -> &'static str {
    match side {
        OrderSide::Buy => "buy",
        OrderSide::Sell => "sell",
    }
}

fn okx_order_type(intent: &OrderIntent) -> &'static str {
    match intent.order_type {
        OrderType::Limit => okx_limit_order_type(intent.time_in_force),
        OrderType::PostOnly => "post_only",
        OrderType::Market => "market",
    }
}

fn okx_limit_order_type(time_in_force: TimeInForce) -> &'static str {
    match time_in_force {
        TimeInForce::Gtc => "limit",
        TimeInForce::Ioc => "ioc",
        TimeInForce::Fok => "fok",
        TimeInForce::Gtx => "post_only",
    }
}

fn validate_client_order_id(client_order_id: &str) -> ExchangeResult<()> {
    crate::client_order_id_policy::validate_client_order_id_policy("okx", client_order_id)
}

fn validation_error(message: String) -> ExchangeError {
    ExchangeError::Api {
        exchange: "okx".into(),
        code: "validation".into(),
        message,
    }
}

#[cfg(test)]
#[path = "okx_trade_data_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "okx_trade_data_ack_tests.rs"]
mod ack_tests;
