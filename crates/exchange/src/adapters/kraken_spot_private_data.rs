//! Kraken Spot WebSocket v2 private account and execution frames.
//! Field semantics: https://docs.kraken.com/exchange/api-reference/spot-websocket-v2/executions

use super::kraken_symbols::{canonical_asset, canonical_symbol};
use crate::error::{ExchangeError, ExchangeResult};
use chrono::{DateTime, TimeZone, Utc};
use serde_json::Value;
use shared_types::{OrderInfo, OrderSide, OrderStatus, OrderType, VenueBalanceInfo};

const VENUE: &str = "kraken:spot";

#[derive(Debug, Clone, PartialEq)]
pub struct KrakenSpotFill {
    pub order_id: String,
    pub client_order_id: Option<String>,
    pub symbol: Option<String>,
    pub side: Option<OrderSide>,
    pub execution_id: String,
    pub quantity: f64,
    pub price: f64,
    pub fee_amount: Option<f64>,
    pub fee_currency: Option<String>,
    pub occurred_at_ms: i64,
}

#[derive(Debug, Clone)]
pub struct KrakenSpotExecution {
    pub order: Option<OrderInfo>,
    pub fill: Option<KrakenSpotFill>,
    pub received_at_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FrameKind {
    Snapshot,
    Update,
}

#[derive(Debug, Clone)]
pub(super) struct SpotOrderPatch {
    pub order_id: String,
    pub symbol: Option<String>,
    pub side: Option<OrderSide>,
    pub order_type: Option<OrderType>,
    pub status: Option<OrderStatus>,
    pub quantity: Option<f64>,
    pub price: Option<f64>,
    pub filled_quantity: Option<f64>,
    pub filled_price: Option<f64>,
    pub fees: Option<f64>,
    pub created_at: Option<DateTime<Utc>>,
    pub client_order_id: Option<String>,
    pub time_in_force: Option<String>,
    pub reduce_only: Option<bool>,
    pub fill: Option<KrakenSpotFill>,
}

impl SpotOrderPatch {
    pub(super) fn merge(self, current: Option<OrderInfo>) -> Option<OrderInfo> {
        if current.as_ref().is_some_and(|row| {
            self.filled_quantity
                .is_some_and(|qty| qty < row.filled_quantity)
        }) {
            return current;
        }
        let keep_terminal = current.as_ref().is_some_and(|row| {
            matches!(
                row.status,
                OrderStatus::Filled
                    | OrderStatus::Canceled
                    | OrderStatus::Expired
                    | OrderStatus::Rejected
            )
        });
        if current.is_none()
            && (self.symbol.is_none()
                || self.side.is_none()
                || self.order_type.is_none()
                || self.quantity.is_none())
        {
            return None;
        }
        let mut row = current.unwrap_or_else(|| OrderInfo {
            order_id: self.order_id.clone(),
            symbol: self.symbol.clone().unwrap_or_default(),
            exchange: VENUE.to_owned(),
            side: self.side.unwrap_or(OrderSide::Buy),
            order_type: self.order_type.unwrap_or(OrderType::Limit),
            status: self.status.unwrap_or(OrderStatus::Pending),
            quantity: self.quantity.unwrap_or_default(),
            price: self.price.unwrap_or_default(),
            filled_quantity: self.filled_quantity.unwrap_or_default(),
            filled_price: self.filled_price.unwrap_or_default(),
            fees: self.fees.unwrap_or_default(),
            created_at: self.created_at.unwrap_or_else(Utc::now),
            execution_style: None,
            venue_time_in_force: self.time_in_force.clone(),
            client_order_id: self.client_order_id.clone(),
            reduce_only: self.reduce_only,
        });
        if let Some(value) = self.symbol {
            row.symbol = value;
        }
        if let Some(value) = self.side {
            row.side = value;
        }
        if let Some(value) = self.order_type {
            row.order_type = value;
        }
        if let Some(value) = self.status {
            if !keep_terminal {
                row.status = value;
            }
        }
        if let Some(value) = self.quantity {
            row.quantity = value;
        }
        if let Some(value) = self.price {
            row.price = value;
        }
        if let Some(value) = self.filled_quantity {
            row.filled_quantity = value;
        }
        if let Some(value) = self.filled_price {
            row.filled_price = value;
        }
        if let Some(value) = self.fees {
            row.fees = value;
        }
        if let Some(value) = self.created_at {
            row.created_at = value;
        }
        if self.client_order_id.is_some() {
            row.client_order_id = self.client_order_id;
        }
        if self.time_in_force.is_some() {
            row.venue_time_in_force = self.time_in_force;
        }
        if self.reduce_only.is_some() {
            row.reduce_only = self.reduce_only;
        }
        Some(row)
    }
}

#[derive(Debug, Clone)]
pub(super) struct SpotBalancePatch {
    pub currency: String,
    pub total: f64,
}

#[derive(Debug, Clone)]
pub(super) enum SpotPrivateFrame {
    Executions {
        kind: FrameKind,
        sequence: i64,
        rows: Vec<SpotOrderPatch>,
    },
    Balances {
        kind: FrameKind,
        sequence: i64,
        rows: Vec<SpotBalancePatch>,
    },
}

pub(super) fn parse_private_frame(text: &str) -> ExchangeResult<Option<SpotPrivateFrame>> {
    let value: Value = serde_json::from_str(text)
        .map_err(|error| ExchangeError::Parse(format!("kraken spot private json: {error}")))?;
    let Some(channel) = value.get("channel").and_then(Value::as_str) else {
        return Ok(None);
    };
    let kind = match value.get("type").and_then(Value::as_str) {
        Some("snapshot") => FrameKind::Snapshot,
        Some("update") => FrameKind::Update,
        other => {
            return Err(ExchangeError::Parse(format!(
                "kraken spot private {channel} invalid frame type {other:?}"
            )))
        }
    };
    let sequence = value
        .get("sequence")
        .and_then(Value::as_i64)
        .ok_or_else(|| ExchangeError::Parse(format!("kraken spot {channel} sequence missing")))?;
    let data = value
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| ExchangeError::Parse(format!("kraken spot {channel} data missing")))?;
    match channel {
        "executions" => Ok(Some(SpotPrivateFrame::Executions {
            kind,
            sequence,
            rows: data
                .iter()
                .map(parse_order_patch)
                .collect::<ExchangeResult<_>>()?,
        })),
        "balances" => Ok(Some(SpotPrivateFrame::Balances {
            kind,
            sequence,
            rows: data
                .iter()
                .map(parse_balance_patch)
                .collect::<ExchangeResult<_>>()?,
        })),
        _ => Ok(None),
    }
}

fn parse_order_patch(value: &Value) -> ExchangeResult<SpotOrderPatch> {
    let order_id = string(value, "order_id")
        .ok_or_else(|| ExchangeError::Parse("kraken spot execution order_id missing".to_owned()))?;
    Ok(SpotOrderPatch {
        order_id,
        symbol: string(value, "symbol").map(|value| canonical_symbol(&value)),
        side: string(value, "side").and_then(|value| match value.as_str() {
            "buy" => Some(OrderSide::Buy),
            "sell" => Some(OrderSide::Sell),
            _ => None,
        }),
        order_type: string(value, "order_type").and_then(|value| match value.as_str() {
            "market" => Some(OrderType::Market),
            "limit" => Some(OrderType::Limit),
            _ => None,
        }),
        status: string(value, "order_status").and_then(|value| order_status(&value)),
        quantity: number(value, "order_qty"),
        price: number(value, "limit_price").or_else(|| number(value, "price")),
        filled_quantity: number(value, "cum_qty"),
        filled_price: number(value, "avg_price").or_else(|| {
            let quantity = number(value, "cum_qty").filter(|quantity| *quantity > 0.0)?;
            number(value, "cum_cost")
                .map(|cost| cost / quantity)
                .filter(|price| price.is_finite())
        }),
        // Per-trade fees and USD equivalents are not a cumulative order fee.
        fees: None,
        created_at: string(value, "timestamp").and_then(|value| parse_timestamp(&value)),
        client_order_id: string(value, "cl_ord_id"),
        time_in_force: string(value, "time_in_force"),
        reduce_only: value.get("reduce_only").and_then(Value::as_bool),
        fill: parse_fill(value)?,
    })
}

fn parse_fill(value: &Value) -> ExchangeResult<Option<KrakenSpotFill>> {
    if string(value, "exec_type").as_deref() != Some("trade") {
        return Ok(None);
    }
    let required = |field: &str| {
        string(value, field)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| ExchangeError::Parse(format!("kraken spot trade {field} missing")))
    };
    let positive = |field: &str| {
        number(value, field)
            .filter(|number| *number > 0.0)
            .ok_or_else(|| ExchangeError::Parse(format!("kraken spot trade {field} invalid")))
    };
    let (fee_amount, fee_currency) = trade_fee(value);
    let occurred_at_ms = parse_timestamp(&required("timestamp")?)
        .ok_or_else(|| ExchangeError::Parse("kraken spot trade timestamp invalid".into()))?
        .timestamp_millis();
    Ok(Some(KrakenSpotFill {
        order_id: required("order_id")?,
        client_order_id: string(value, "cl_ord_id"),
        symbol: string(value, "symbol"),
        side: string(value, "side").and_then(|side| match side.as_str() {
            "buy" => Some(OrderSide::Buy),
            "sell" => Some(OrderSide::Sell),
            _ => None,
        }),
        execution_id: required("exec_id")?,
        quantity: positive("last_qty")?,
        price: positive("last_price")?,
        fee_amount,
        fee_currency,
        occurred_at_ms,
    }))
}

fn trade_fee(value: &Value) -> (Option<f64>, Option<String>) {
    let Some(rows) = value
        .get("fees")
        .and_then(Value::as_array)
        .filter(|rows| !rows.is_empty())
    else {
        return (None, None);
    };
    let mut currency: Option<String> = None;
    let mut total = 0.0;
    for row in rows {
        let (Some(asset), Some(amount)) = (string(row, "asset"), number(row, "qty")) else {
            return (None, None);
        };
        let asset = canonical_asset(&asset);
        if asset.is_empty() {
            return (None, None);
        }
        if currency.as_ref().is_some_and(|known| known != &asset) {
            return (None, None);
        }
        currency = Some(asset);
        total += amount;
    }
    if total.is_finite() {
        (Some(total), currency)
    } else {
        (None, None)
    }
}

fn parse_balance_patch(value: &Value) -> ExchangeResult<SpotBalancePatch> {
    let currency = string(value, "asset")
        .map(|value| canonical_asset(&value))
        .ok_or_else(|| ExchangeError::Parse("kraken spot balance asset missing".to_owned()))?;
    let total = number(value, "balance")
        .ok_or_else(|| ExchangeError::Parse(format!("kraken spot {currency} balance missing")))?;
    Ok(SpotBalancePatch { currency, total })
}

pub(super) fn parse_balance_ex(text: &str) -> ExchangeResult<Vec<VenueBalanceInfo>> {
    let value = parse_rest_result(text, "BalanceEx")?;
    let rows = value.as_object().ok_or_else(|| {
        ExchangeError::Parse("kraken BalanceEx result is not an object".to_owned())
    })?;
    let mut balances = Vec::with_capacity(rows.len());
    for (asset, row) in rows {
        let total = number(row, "balance").unwrap_or_default();
        let credit = number(row, "credit").unwrap_or_default();
        let credit_used = number(row, "credit_used").unwrap_or_default();
        let frozen = number(row, "hold_trade").unwrap_or_default();
        balances.push(VenueBalanceInfo {
            venue: VENUE.to_owned(),
            currency: canonical_asset(asset),
            total,
            available: total + credit - credit_used - frozen,
            frozen,
            unrealized_pnl: 0.0,
        });
    }
    balances.sort_by(|left, right| left.currency.cmp(&right.currency));
    Ok(balances)
}

pub(super) fn parse_rest_orders(text: &str) -> ExchangeResult<Vec<OrderInfo>> {
    let result = parse_rest_result(text, "orders")?;
    let rows = result.get("open").unwrap_or(&result);
    let rows = rows
        .as_object()
        .ok_or_else(|| ExchangeError::Parse("kraken order result is not an object".to_owned()))?;
    Ok(rows
        .iter()
        .map(|(id, row)| parse_rest_order(id, row))
        .collect())
}

fn parse_rest_order(order_id: &str, value: &Value) -> OrderInfo {
    let description = value.get("descr").unwrap_or(&Value::Null);
    let quantity = number(value, "vol").unwrap_or_default();
    let filled_quantity = number(value, "vol_exec").unwrap_or_default();
    let cost = number(value, "cost").unwrap_or_default();
    let created_at = number(value, "opentm")
        .and_then(|seconds| {
            Utc.timestamp_millis_opt((seconds * 1_000.0) as i64)
                .single()
        })
        .unwrap_or_else(Utc::now);
    OrderInfo {
        order_id: order_id.to_owned(),
        symbol: string(description, "pair")
            .map_or_else(String::new, |value| canonical_symbol(&value)),
        exchange: VENUE.to_owned(),
        side: match string(description, "type").as_deref() {
            Some("sell") => OrderSide::Sell,
            _ => OrderSide::Buy,
        },
        order_type: match string(description, "ordertype").as_deref() {
            Some("market") => OrderType::Market,
            _ => OrderType::Limit,
        },
        status: string(value, "status")
            .and_then(|value| order_status(&value))
            .unwrap_or(OrderStatus::Pending),
        quantity,
        price: number(description, "price").unwrap_or_default(),
        filled_quantity,
        filled_price: if filled_quantity > 0.0 {
            cost / filled_quantity
        } else {
            0.0
        },
        fees: number(value, "fee").unwrap_or_default(),
        created_at,
        execution_style: None,
        venue_time_in_force: string(description, "timeinforce"),
        client_order_id: string(value, "cl_ord_id"),
        reduce_only: None,
    }
}

fn parse_rest_result(text: &str, operation: &str) -> ExchangeResult<Value> {
    let value: Value = serde_json::from_str(text)
        .map_err(|error| ExchangeError::Parse(format!("kraken {operation} json: {error}")))?;
    if let Some(error) = value
        .get("error")
        .and_then(Value::as_array)
        .and_then(|rows| rows.first())
        .and_then(Value::as_str)
    {
        return Err(ExchangeError::Api {
            exchange: "kraken".to_owned(),
            code: error.split(':').next().unwrap_or("unknown").to_owned(),
            message: error.to_owned(),
        });
    }
    value
        .get("result")
        .cloned()
        .ok_or_else(|| ExchangeError::Parse(format!("kraken {operation} result missing")))
}

fn string(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(|value| match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    })
}

fn number(value: &Value, key: &str) -> Option<f64> {
    value.get(key).and_then(|value| match value {
        Value::Number(value) => value.as_f64(),
        Value::String(value) => value.parse().ok(),
        _ => None,
    })
}

fn order_status(value: &str) -> Option<OrderStatus> {
    match value {
        "pending_new" | "pending" => Some(OrderStatus::Pending),
        "new" | "open" => Some(OrderStatus::Open),
        "partially_filled" => Some(OrderStatus::PartiallyFilled),
        "filled" | "closed" => Some(OrderStatus::Filled),
        "canceled" | "cancelled" => Some(OrderStatus::Canceled),
        "expired" => Some(OrderStatus::Expired),
        "rejected" => Some(OrderStatus::Rejected),
        _ => None,
    }
}

fn parse_timestamp(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|value| value.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trade() -> Value {
        let frame: Value = serde_json::from_str(include_str!(
            "../../fixtures/kraken/spot_v2_execution_update.json"
        ))
        .unwrap();
        frame["data"][0].clone()
    }

    #[test]
    fn spot_trade_keeps_native_fee_separate_from_usd_estimate_and_order_totals() {
        let mut trade = trade();
        trade["fee_usd_equiv"] = serde_json::json!(999.0);
        trade["fees"] = serde_json::json!([{"asset":"EUR","qty":0.25}]);
        let patch = parse_order_patch(&trade).unwrap();
        assert_eq!(patch.fees, None);
        let fill = patch.fill.unwrap();
        assert_eq!(fill.fee_amount, Some(0.25));
        assert_eq!(fill.fee_currency.as_deref(), Some("EUR"));
        assert_eq!(fill.quantity, 0.005);
        assert_eq!(fill.execution_id, "TGBB7L-HT5LX-J3BZ4A");
    }

    #[test]
    fn spot_trade_distinguishes_zero_missing_and_mixed_fee_assets() {
        let mut trade = trade();
        for fees in [
            serde_json::Value::Null,
            serde_json::json!([]),
            serde_json::json!([{"asset":"USD"}]),
            serde_json::json!([{"asset":"USD","qty":0.1},{"asset":"EUR","qty":0.1}]),
        ] {
            trade["fees"] = fees;
            assert_eq!(parse_fill(&trade).unwrap().unwrap().fee_amount, None);
        }
        trade["fees"] = serde_json::json!([{"asset":"USD","qty":0.0}]);
        assert_eq!(parse_fill(&trade).unwrap().unwrap().fee_amount, Some(0.0));
        trade["fees"] = serde_json::json!([{"asset":"USD","qty":-0.01}]);
        assert_eq!(parse_fill(&trade).unwrap().unwrap().fee_amount, Some(-0.01));
    }

    #[test]
    fn spot_average_price_uses_cumulative_cost_never_the_last_trade_price() {
        let mut trade = trade();
        trade.as_object_mut().unwrap().remove("avg_price");
        trade["last_price"] = serde_json::json!(25000.0);
        assert_eq!(
            parse_order_patch(&trade).unwrap().filled_price,
            Some(26599.9)
        );
        trade.as_object_mut().unwrap().remove("cum_cost");
        assert_eq!(parse_order_patch(&trade).unwrap().filled_price, None);
    }

    #[test]
    fn order_state_notifications_are_not_additional_trade_fills() {
        let mut trade = trade();
        trade["exec_type"] = serde_json::json!("filled");
        assert!(parse_fill(&trade).unwrap().is_none());
        trade["exec_type"] = serde_json::json!("trade");
        trade["exec_id"] = serde_json::json!("");
        assert!(parse_fill(&trade).is_err());
    }

    #[test]
    fn parses_official_execution_and_balance_frames() {
        let execution = parse_private_frame(include_str!(
            "../../fixtures/kraken/spot_v2_execution_update.json"
        ))
        .unwrap();
        let Some(SpotPrivateFrame::Executions { sequence, rows, .. }) = execution else {
            panic!("execution frame expected");
        };
        assert_eq!(sequence, 10);
        assert_eq!(rows[0].filled_quantity, Some(0.005));
        assert_eq!(rows[0].status, Some(OrderStatus::PartiallyFilled));

        let balances = parse_private_frame(include_str!(
            "../../fixtures/kraken/spot_v2_balances_snapshot.json"
        ))
        .unwrap();
        let Some(SpotPrivateFrame::Balances { rows, .. }) = balances else {
            panic!("balance frame expected");
        };
        assert_eq!(rows[0].currency, "BTC");
        assert_eq!(rows[0].total, 1.2);
    }

    #[test]
    fn balance_ex_uses_official_available_formula() {
        let rows =
            parse_balance_ex(include_str!("../../fixtures/kraken/spot_balance_ex.json")).unwrap();
        let usd = rows.iter().find(|row| row.currency == "USD").unwrap();
        assert!((usd.available - 17_185.45).abs() < 1e-9);
        assert_eq!(usd.frozen, 8_249.76);
    }

    #[test]
    fn rest_order_fixtures_keep_open_and_terminal_state() {
        let open =
            parse_rest_orders(include_str!("../../fixtures/kraken/spot_open_orders.json")).unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].status, OrderStatus::Open);
        assert_eq!(open[0].filled_quantity, 0.375);

        let closed =
            parse_rest_orders(include_str!("../../fixtures/kraken/spot_query_orders.json"))
                .unwrap();
        assert_eq!(closed.len(), 1);
        assert_eq!(closed[0].status, OrderStatus::Filled);
        assert_eq!(closed[0].filled_quantity, 1.25);
    }
}
