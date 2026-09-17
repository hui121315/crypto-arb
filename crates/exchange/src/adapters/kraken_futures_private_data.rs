//! Kraken Derivatives private feed parsing.

use super::kraken_symbols::{canonical_asset, canonical_symbol};
use crate::error::{ExchangeError, ExchangeResult};
use chrono::{TimeZone, Utc};
use serde_json::Value;
use shared_types::{OrderInfo, OrderSide, OrderStatus, OrderType, PositionInfo, VenueBalanceInfo};

const VENUE: &str = "kraken:futures";

#[derive(Debug, Clone)]
pub(super) struct FuturesFillPatch {
    pub order_id: String,
    pub client_order_id: Option<String>,
    pub sequence: i64,
    pub quantity: f64,
    pub remaining_quantity: f64,
    pub price: f64,
    pub fee: f64,
}

#[derive(Debug, Clone)]
pub(super) enum FuturesPrivateFrame {
    OrdersSnapshot(Vec<OrderInfo>),
    OrderDelta {
        order: Option<OrderInfo>,
        order_id: String,
        terminal_status: Option<OrderStatus>,
    },
    Fills(Vec<FuturesFillPatch>),
    Balances {
        snapshot: bool,
        sequence: i64,
        rows: Vec<VenueBalanceInfo>,
    },
    Positions(Vec<PositionInfo>),
}

pub(super) fn parse_private_frame(text: &str) -> ExchangeResult<Option<FuturesPrivateFrame>> {
    let value: Value = serde_json::from_str(text)
        .map_err(|error| ExchangeError::Parse(format!("kraken futures private json: {error}")))?;
    let Some(feed) = value.get("feed").and_then(Value::as_str) else {
        return Ok(None);
    };
    match feed {
        "open_orders_snapshot" => Ok(Some(FuturesPrivateFrame::OrdersSnapshot(
            value
                .get("orders")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    ExchangeError::Parse("kraken open_orders snapshot missing".to_owned())
                })?
                .iter()
                .map(parse_order)
                .collect::<ExchangeResult<_>>()?,
        ))),
        "open_orders" => parse_order_delta(&value).map(Some),
        "fills_snapshot" | "fills" => Ok(Some(FuturesPrivateFrame::Fills(
            value
                .get("fills")
                .and_then(Value::as_array)
                .ok_or_else(|| ExchangeError::Parse("kraken fills missing".to_owned()))?
                .iter()
                .map(parse_fill)
                .collect::<ExchangeResult<_>>()?,
        ))),
        "balances_snapshot" | "balances" => Ok(Some(FuturesPrivateFrame::Balances {
            snapshot: feed == "balances_snapshot",
            sequence: integer(&value, "seq")
                .ok_or_else(|| ExchangeError::Parse("kraken balances seq missing".to_owned()))?,
            rows: parse_balances(&value),
        })),
        "open_positions" => Ok(Some(FuturesPrivateFrame::Positions(
            value
                .get("positions")
                .and_then(Value::as_array)
                .ok_or_else(|| ExchangeError::Parse("kraken positions missing".to_owned()))?
                .iter()
                .map(parse_position)
                .collect::<ExchangeResult<_>>()?,
        ))),
        _ => Ok(None),
    }
}

pub(super) fn parse_order(value: &Value) -> ExchangeResult<OrderInfo> {
    let order_id = string_any(value, &["order_id", "orderId"])
        .ok_or_else(|| ExchangeError::Parse("kraken futures order id missing".to_owned()))?;
    let filled_quantity = number_any(value, &["filled", "filledSize"]).unwrap_or_default();
    let quantity = number_any(value, &["qty", "quantity"]).unwrap_or_else(|| {
        number_any(value, &["unfilledSize"]).unwrap_or_default() + filled_quantity
    });
    let timestamp = integer_any(value, &["time"])
        .and_then(|value| Utc.timestamp_millis_opt(value).single())
        .or_else(|| {
            string_any(value, &["timestamp", "receivedTime"])
                .and_then(|value| chrono::DateTime::parse_from_rfc3339(&value).ok())
                .map(|value| value.with_timezone(&Utc))
        })
        .unwrap_or_else(Utc::now);
    Ok(OrderInfo {
        order_id,
        symbol: string_any(value, &["instrument", "symbol"])
            .map_or_else(String::new, |value| canonical_symbol(&value)),
        exchange: VENUE.to_owned(),
        side: match string_any(value, &["side"]).as_deref() {
            Some("sell") => OrderSide::Sell,
            Some("buy") => OrderSide::Buy,
            _ if integer(value, "direction") == Some(1) => OrderSide::Sell,
            _ => OrderSide::Buy,
        },
        order_type: match string_any(value, &["type", "orderType"]).as_deref() {
            Some("mkt" | "market") => OrderType::Market,
            Some("post") => OrderType::PostOnly,
            _ => OrderType::Limit,
        },
        status: string_any(value, &["status"])
            .and_then(|value| order_status(&value))
            .unwrap_or({
                if quantity > 0.0 && filled_quantity >= quantity {
                    OrderStatus::Filled
                } else if filled_quantity > 0.0 {
                    OrderStatus::PartiallyFilled
                } else {
                    OrderStatus::Open
                }
            }),
        quantity,
        price: number_any(value, &["limit_price", "limitPrice"]).unwrap_or_default(),
        filled_quantity,
        filled_price: number_any(value, &["filledPrice", "avgPrice"]).unwrap_or_default(),
        fees: number_any(value, &["fee"]).unwrap_or_default(),
        created_at: timestamp,
        execution_style: None,
        venue_time_in_force: string_any(value, &["type", "orderType"]),
        client_order_id: string_any(value, &["cli_ord_id", "cliOrdId"]),
        reduce_only: bool_any(value, &["reduce_only", "reduceOnly"]),
    })
}

pub(super) fn parse_position(value: &Value) -> ExchangeResult<PositionInfo> {
    let symbol = string_any(value, &["instrument", "symbol"])
        .ok_or_else(|| ExchangeError::Parse("kraken futures position symbol missing".to_owned()))?;
    let signed_quantity = number_any(value, &["balance", "size"])
        .or_else(|| number_any(value, &["quantity"]))
        .unwrap_or_default();
    let side = string_any(value, &["side"]).unwrap_or_else(|| {
        if signed_quantity < 0.0 {
            "short".to_owned()
        } else {
            "long".to_owned()
        }
    });
    let quantity = signed_quantity.abs();
    let margin = number_any(value, &["initial_margin", "initialMargin"]).unwrap_or_default();
    let maintenance =
        number_any(value, &["maintenance_margin", "maintenanceMargin"]).unwrap_or_default();
    Ok(PositionInfo {
        symbol: canonical_symbol(&symbol),
        exchange: VENUE.to_owned(),
        side: side.to_ascii_lowercase(),
        quantity,
        entry_price: number_any(value, &["entry_price", "price"]).unwrap_or_default(),
        mark_price: number_any(value, &["mark_price", "markPrice"]).unwrap_or_default(),
        unrealized_pnl: number_any(value, &["pnl", "unrealizedPnl"]).unwrap_or_default(),
        leverage: number_any(value, &["effective_leverage", "effectiveLeverage"]).unwrap_or(1.0),
        liquidation_price: number_any(value, &["liquidation_threshold", "liquidationThreshold"])
            .filter(|value| *value > 0.0),
        liquidation_distance_pct: None,
        next_funding_ms: None,
        paired_with: None,
        margin,
        maintenance_margin_ratio: if margin > 0.0 {
            maintenance / margin
        } else {
            0.0
        },
        position_mode: Some("one_way".to_owned()),
        margin_mode: None,
        risk_rate: None,
        available_position: Some(quantity),
        frozen_position: None,
    })
}

fn parse_order_delta(value: &Value) -> ExchangeResult<FuturesPrivateFrame> {
    let is_cancel = value
        .get("is_cancel")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !is_cancel {
        let order = value.get("order").ok_or_else(|| {
            ExchangeError::Parse("kraken open_orders delta order missing".to_owned())
        })?;
        let order = parse_order(order)?;
        return Ok(FuturesPrivateFrame::OrderDelta {
            order_id: order.order_id.clone(),
            order: Some(order),
            terminal_status: None,
        });
    }
    let order_id = string_any(value, &["order_id"])
        .ok_or_else(|| ExchangeError::Parse("kraken cancelled order id missing".to_owned()))?;
    let reason = string_any(value, &["reason"]).unwrap_or_default();
    let terminal_status = if reason == "full_fill" {
        OrderStatus::Filled
    } else if matches!(
        reason.as_str(),
        "not_enough_margin"
            | "post_order_failed_because_it_would_filled"
            | "would_execute_self"
            | "would_not_reduce_position"
            | "ioc_order_failed_because_it_would_not_be_executed"
    ) {
        OrderStatus::Rejected
    } else {
        OrderStatus::Canceled
    };
    Ok(FuturesPrivateFrame::OrderDelta {
        order: None,
        order_id,
        terminal_status: Some(terminal_status),
    })
}

fn parse_fill(value: &Value) -> ExchangeResult<FuturesFillPatch> {
    Ok(FuturesFillPatch {
        order_id: string_any(value, &["order_id"]).ok_or_else(|| {
            ExchangeError::Parse("kraken futures fill order_id missing".to_owned())
        })?,
        client_order_id: string_any(value, &["cli_ord_id"]),
        sequence: integer(value, "seq")
            .ok_or_else(|| ExchangeError::Parse("kraken futures fill seq missing".to_owned()))?,
        quantity: number_any(value, &["qty"]).unwrap_or_default(),
        remaining_quantity: number_any(value, &["remaining_order_qty"]).unwrap_or_default(),
        price: number_any(value, &["price"]).unwrap_or_default(),
        fee: number_any(value, &["fee_paid"]).unwrap_or_default(),
    })
}

fn parse_balances(value: &Value) -> Vec<VenueBalanceInfo> {
    let mut rows = std::collections::BTreeMap::new();
    if let Some(holding) = value.get("holding").and_then(Value::as_object) {
        for (currency, total) in holding {
            if let Some(total) = json_number(total) {
                let currency = canonical_asset(currency);
                rows.insert(currency.clone(), balance(currency, total, total, 0.0));
            }
        }
    }
    if let Some(currencies) = value
        .pointer("/flex_futures/currencies")
        .and_then(Value::as_object)
    {
        for (currency, row) in currencies {
            let total = number_any(row, &["quantity"]).unwrap_or_default();
            let available = number_any(row, &["available"]).unwrap_or_default();
            let currency = canonical_asset(currency);
            rows.insert(currency.clone(), balance(currency, total, available, 0.0));
        }
    }
    if let Some(futures) = value.get("futures").and_then(Value::as_object) {
        for row in futures.values() {
            let Some(currency) = string_any(row, &["unit"]) else {
                continue;
            };
            let total = number_any(row, &["balance", "portfolio_value"]).unwrap_or_default();
            let available = number_any(row, &["available"]).unwrap_or_default();
            let pnl = number_any(row, &["pnl"]).unwrap_or_default();
            let currency = canonical_asset(&currency);
            rows.insert(currency.clone(), balance(currency, total, available, pnl));
        }
    }
    rows.into_values().collect()
}

fn balance(currency: String, total: f64, available: f64, pnl: f64) -> VenueBalanceInfo {
    VenueBalanceInfo {
        venue: VENUE.to_owned(),
        currency,
        total,
        available,
        frozen: (total - available).max(0.0),
        unrealized_pnl: pnl,
    }
}

fn string_any(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        value.get(*key).and_then(|value| match value {
            Value::String(value) => Some(value.clone()),
            Value::Number(value) => Some(value.to_string()),
            _ => None,
        })
    })
}

fn number_any(value: &Value, keys: &[&str]) -> Option<f64> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(json_number))
}

fn integer_any(value: &Value, keys: &[&str]) -> Option<i64> {
    keys.iter().find_map(|key| integer(value, key))
}

fn integer(value: &Value, key: &str) -> Option<i64> {
    value.get(key).and_then(|value| {
        value
            .as_i64()
            .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok()))
    })
}

fn bool_any(value: &Value, keys: &[&str]) -> Option<bool> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_bool))
}

fn json_number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(value) => value.as_f64(),
        Value::String(value) => value.parse().ok(),
        _ => None,
    }
}

fn order_status(value: &str) -> Option<OrderStatus> {
    match value {
        "pending" => Some(OrderStatus::Pending),
        "untouched" | "placed" | "open" => Some(OrderStatus::Open),
        "partiallyFilled" | "partially_filled" => Some(OrderStatus::PartiallyFilled),
        "filled" => Some(OrderStatus::Filled),
        "cancelled" | "canceled" => Some(OrderStatus::Canceled),
        "expired" => Some(OrderStatus::Expired),
        "rejected" => Some(OrderStatus::Rejected),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_official_orders_fills_positions_and_balances() {
        let orders = parse_private_frame(include_str!(
            "../../fixtures/kraken/futures_open_orders_snapshot.json"
        ))
        .unwrap();
        let Some(FuturesPrivateFrame::OrdersSnapshot(orders)) = orders else {
            panic!("orders expected");
        };
        assert_eq!(orders[0].symbol, "BTC");
        assert_eq!(orders[0].side, OrderSide::Sell);

        let fills = parse_private_frame(include_str!(
            "../../fixtures/kraken/futures_fills_snapshot.json"
        ))
        .unwrap();
        let Some(FuturesPrivateFrame::Fills(fills)) = fills else {
            panic!("fills expected");
        };
        assert_eq!(fills[0].remaining_quantity, 0.0);

        let positions = parse_private_frame(include_str!(
            "../../fixtures/kraken/futures_open_positions.json"
        ))
        .unwrap();
        let Some(FuturesPrivateFrame::Positions(positions)) = positions else {
            panic!("positions expected");
        };
        assert_eq!(positions[0].side, "long");
        assert_eq!(positions[0].liquidation_price, Some(9572.804662403718));

        let balances = parse_private_frame(include_str!(
            "../../fixtures/kraken/futures_balances_snapshot.json"
        ))
        .unwrap();
        let Some(FuturesPrivateFrame::Balances { rows, .. }) = balances else {
            panic!("balances expected");
        };
        assert_eq!(rows[0].currency, "USD");
        assert_eq!(rows[0].available, 5000.0);
    }

    #[test]
    fn cancellation_reason_preserves_terminal_meaning() {
        let frame = parse_private_frame(include_str!(
            "../../fixtures/kraken/futures_order_cancelled.json"
        ))
        .unwrap();
        let Some(FuturesPrivateFrame::OrderDelta {
            terminal_status, ..
        }) = frame
        else {
            panic!("delta expected");
        };
        assert_eq!(terminal_status, Some(OrderStatus::Canceled));
    }
}
