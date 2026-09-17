use super::*;

pub(in crate::lifecycle::private_ws::tests) fn private_order_cancel_event(
    record: &OrderRecord,
    occurred_at_ms: i64,
) -> anyhow::Result<PrivateWsEvent> {
    private_order_status_event(record, OrderStatus::Canceled, occurred_at_ms)
}

pub(in crate::lifecycle::private_ws::tests) fn private_order_filled_event(
    record: &OrderRecord,
    occurred_at_ms: i64,
) -> anyhow::Result<PrivateWsEvent> {
    private_order_status_event(record, OrderStatus::Filled, occurred_at_ms)
}

fn private_order_status_event(
    record: &OrderRecord,
    status: OrderStatus,
    occurred_at_ms: i64,
) -> anyhow::Result<PrivateWsEvent> {
    let order_id = record
        .exchange_order_id
        .clone()
        .ok_or_else(|| anyhow::anyhow!("mock submit did not return an exchange order id"))?;
    let filled_quantity = if status == OrderStatus::Filled {
        record.intent.quantity
    } else {
        0.0
    };
    Ok(PrivateWsEvent::Order(PrivateOrderDelta {
        client_order_id: record.intent.client_order_id.clone(),
        order: OrderInfo {
            execution_style: None,
            venue_time_in_force: None,
            client_order_id: Some(record.intent.client_order_id.clone()),
            reduce_only: Some(record.intent.reduce_only),
            order_id,
            symbol: record.intent.symbol.clone(),
            exchange: record.intent.exchange.clone(),
            side: record.intent.side,
            order_type: record.intent.order_type,
            status,
            quantity: record.intent.quantity,
            price: record.intent.price.unwrap_or_default(),
            filled_quantity,
            filled_price: record.intent.price.unwrap_or_default(),
            fees: 0.0,
            created_at: chrono::Utc::now(),
        },
        received_at_ms: occurred_at_ms,
    }))
}
