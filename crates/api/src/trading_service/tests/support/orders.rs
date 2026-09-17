use super::*;

pub(in crate::trading_service::tests) fn order_info(
    order_id: &str,
    status: OrderStatus,
    quantity: f64,
) -> OrderInfo {
    OrderInfo {
        execution_style: None,
        venue_time_in_force: None,
        client_order_id: None,
        reduce_only: None,
        order_id: order_id.into(),
        symbol: "BTC".into(),
        exchange: "mock".into(),
        side: OrderSide::Buy,
        order_type: OrderType::Limit,
        status,
        quantity,
        price: 50_000.0,
        filled_quantity: if status == OrderStatus::PartiallyFilled {
            quantity / 2.0
        } else {
            0.0
        },
        filled_price: if status == OrderStatus::PartiallyFilled {
            50_000.0
        } else {
            0.0
        },
        fees: 0.0,
        created_at: chrono::Utc::now(),
    }
}

#[allow(clippy::panic)]
pub(in crate::trading_service::tests) fn must_some<T>(value: Option<T>, context: &str) -> T {
    match value {
        Some(value) => value,
        None => panic!("{context}"),
    }
}
