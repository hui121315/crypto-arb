use super::*;
use crate::adapters::hyperliquid_ws_user_raw::RawBasicOrder;

pub(super) fn parse_open_orders(data: Value) -> ExchangeResult<HyperliquidOpenOrdersSnapshot> {
    let row: RawOpenOrdersEnvelope = parse_value(data, "openOrders")?;
    let venue = venue_for_dex(&row.dex);
    let orders = row
        .orders
        .into_iter()
        .map(|order| open_order(order, &venue))
        .collect::<ExchangeResult<Vec<_>>>()?;
    Ok(HyperliquidOpenOrdersSnapshot { venue, orders })
}

fn open_order(order: RawBasicOrder, venue: &str) -> ExchangeResult<OrderInfo> {
    let status_timestamp = order.timestamp;
    let mut order = order_update(RawOrderUpdate {
        order,
        status: "open".to_owned(),
        status_timestamp,
    })?
    .order;
    order.exchange = venue.to_owned();
    Ok(order)
}

fn venue_for_dex(dex: &str) -> String {
    let dex = dex.trim();
    if dex.is_empty() {
        EXCHANGE.to_owned()
    } else {
        format!("{EXCHANGE}:{dex}")
    }
}
