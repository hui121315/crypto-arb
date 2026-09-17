use super::{
    parse_order_side, parse_order_status, parse_order_type, parse_reduce_only,
    parse_required_decimal, parse_required_text, parse_required_timestamp, parse_time_in_force,
    BitgetTimeInForce, NAME,
};
use crate::adapter::{client_order_id_from_str, strip_common_suffixes};
use crate::error::ExchangeResult;
use serde::Deserialize;
use shared_types::{OrderInfo, OrderType};

/// V3 `GET /api/v3/trade/unfilled-orders` and `order-info` row.
#[derive(Debug, Deserialize)]
pub(in crate::adapters) struct UtaOrderRow {
    pub(super) symbol: String,
    #[serde(rename = "orderId")]
    pub(super) order_id: String,
    #[serde(rename = "orderStatus")]
    pub(super) order_status: String,
    #[serde(rename = "orderType")]
    pub(super) order_type: String,
    #[serde(rename = "timeInForce")]
    pub(super) time_in_force: String,
    pub(super) side: String,
    pub(super) price: String,
    pub(super) qty: String,
    #[serde(rename = "cumExecQty")]
    pub(super) cum_exec_qty: String,
    #[serde(rename = "avgPrice")]
    pub(super) avg_price: String,
    #[serde(rename = "createdTime")]
    pub(super) created_time: String,
    #[serde(rename = "clientOid")]
    pub(super) client_oid: String,
    #[serde(rename = "reduceOnly")]
    pub(super) reduce_only: String,
    #[serde(default, rename = "feeDetail")]
    pub(super) fee_detail: Vec<UtaFeeDetail>,
    #[serde(default, rename = "execType")]
    pub(super) exec_type: String,
    #[serde(default, rename = "cancelReason")]
    pub(super) cancel_reason: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct UtaFeeDetail {
    #[serde(default, rename = "feeCoin")]
    fee_coin: Option<String>,
    #[serde(default)]
    fee: Option<String>,
}

pub(in crate::adapters) fn parse_open_order(order: UtaOrderRow) -> ExchangeResult<OrderInfo> {
    parse_required_text("order", "orderId", &order.order_id)?;
    let symbol = parse_required_text("order", "symbol", &order.symbol)?;
    let symbol = strip_common_suffixes(symbol);
    let side = parse_order_side(&order.side)?;
    let order_type = if parse_time_in_force(&order.time_in_force)? == BitgetTimeInForce::PostOnly {
        OrderType::PostOnly
    } else {
        parse_order_type(&order.order_type)?
    };
    let status = parse_order_status(&order.order_status)?;
    let created_at = parse_required_timestamp("order", "createdTime", &order.created_time)?;
    let reduce_only = parse_reduce_only(&order.reduce_only)?;
    let filled_quantity = parse_required_decimal("order", "cumExecQty", &order.cum_exec_qty)?;
    Ok(OrderInfo {
        execution_style: order_context(&order.exec_type, &order.cancel_reason),
        venue_time_in_force: Some(order.time_in_force.clone()),
        client_order_id: client_order_id_from_str(&order.client_oid),
        reduce_only,
        order_id: order.order_id,
        symbol,
        exchange: NAME.into(),
        side,
        order_type,
        status,
        quantity: parse_required_decimal("order", "qty", &order.qty)?,
        price: parse_required_decimal("order", "price", &order.price)?,
        filled_quantity,
        filled_price: parse_required_decimal("order", "avgPrice", &order.avg_price)?,
        fees: parse_fees(&order.fee_detail)?,
        created_at,
    })
}

fn parse_fees(rows: &[UtaFeeDetail]) -> ExchangeResult<f64> {
    rows.iter().try_fold(0.0, |total, row| {
        let coin = non_empty_optional_text(row.fee_coin.as_deref());
        let fee = non_empty_optional_text(row.fee.as_deref());
        match (coin, fee) {
            (None, None) => Ok(total),
            (Some(_), Some(fee)) => {
                parse_required_decimal("order", "feeDetail.fee", fee).map(|fee| total + fee)
            }
            (None, Some(_)) => parse_required_text("order", "feeDetail.feeCoin", "").map(|_| total),
            (Some(_), None) => {
                parse_required_decimal("order", "feeDetail.fee", "").map(|fee| total + fee)
            }
        }
    })
}

fn non_empty_optional_text(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn order_context(exec_type: &str, cancel_reason: &str) -> Option<String> {
    let exec_type = exec_type.trim();
    let cancel_reason = cancel_reason.trim();
    match (exec_type.is_empty(), cancel_reason.is_empty()) {
        (true, true) => None,
        (false, true) => Some(exec_type.to_owned()),
        (true, false) => Some(format!("cancel_reason={cancel_reason}")),
        (false, false) => Some(format!("{exec_type}; cancel_reason={cancel_reason}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_fee_placeholders_are_not_parse_failures() -> ExchangeResult<()> {
        assert_eq!(
            parse_fees(&[
                UtaFeeDetail {
                    fee_coin: None,
                    fee: None,
                },
                UtaFeeDetail {
                    fee_coin: Some(String::new()),
                    fee: Some(String::new()),
                },
            ])?,
            0.0
        );
        Ok(())
    }

    #[test]
    fn one_sided_fee_placeholder_remains_invalid() {
        let result = parse_fees(&[UtaFeeDetail {
            fee_coin: Some("USDT".to_owned()),
            fee: None,
        }]);

        assert!(result.is_err());
    }
}
