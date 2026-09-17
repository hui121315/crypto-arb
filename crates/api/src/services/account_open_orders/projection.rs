use super::*;

pub(super) fn open_order_field_quality(
    rows: &[OrderInfo],
    observed_at_ms: i64,
) -> Vec<AccountFieldQuality> {
    rows.iter()
        .flat_map(|row| {
            [
                non_empty_order_field(row, "orderId", &row.order_id, observed_at_ms),
                non_empty_order_field(row, "symbol", &row.symbol, observed_at_ms),
                positive_order_number(row, "quantity", row.quantity, observed_at_ms),
                order_price_quality(row, observed_at_ms),
                filled_quantity_quality(row, observed_at_ms),
                filled_price_quality(row, observed_at_ms),
                non_negative_order_number(row, "fees", row.fees, observed_at_ms),
                open_order_status_quality(row, observed_at_ms),
                available_open_order_field(
                    row,
                    "clientOrderId",
                    row.client_order_id
                        .as_deref()
                        .is_some_and(|value| !value.trim().is_empty()),
                    observed_at_ms,
                ),
                available_open_order_field(
                    row,
                    "venueTimeInForce",
                    row.venue_time_in_force
                        .as_deref()
                        .is_some_and(|value| !value.trim().is_empty()),
                    observed_at_ms,
                ),
                available_open_order_field(
                    row,
                    "reduceOnly",
                    row.reduce_only.is_some(),
                    observed_at_ms,
                ),
                available_open_order_field(
                    row,
                    "executionStyle",
                    row.execution_style
                        .as_deref()
                        .is_some_and(|value| !value.trim().is_empty()),
                    observed_at_ms,
                ),
            ]
        })
        .flatten()
        .collect()
}

fn available_open_order_field(
    row: &OrderInfo,
    field: &'static str,
    present: bool,
    observed_at_ms: i64,
) -> Option<AccountFieldQuality> {
    present.then(|| {
        AccountFieldQuality::new(
            open_order_subject(row),
            field,
            AccountFieldQualityStatus::Actual,
            OPEN_ORDERS_SOURCE,
            Some(observed_at_ms),
        )
    })
}

fn non_empty_order_field(
    row: &OrderInfo,
    field: &'static str,
    value: &str,
    observed_at_ms: i64,
) -> Option<AccountFieldQuality> {
    value.trim().is_empty().then(|| {
        unavailable_open_order_field(
            row,
            field,
            AccountFieldQualityStatus::Missing,
            observed_at_ms,
        )
    })
}

fn positive_order_number(
    row: &OrderInfo,
    field: &'static str,
    value: f64,
    observed_at_ms: i64,
) -> Option<AccountFieldQuality> {
    (!positive_finite(value)).then(|| {
        unavailable_open_order_field(
            row,
            field,
            AccountFieldQualityStatus::Invalid,
            observed_at_ms,
        )
    })
}

fn non_negative_order_number(
    row: &OrderInfo,
    field: &'static str,
    value: f64,
    observed_at_ms: i64,
) -> Option<AccountFieldQuality> {
    (!non_negative_finite(value)).then(|| {
        unavailable_open_order_field(
            row,
            field,
            AccountFieldQualityStatus::Invalid,
            observed_at_ms,
        )
    })
}

fn order_price_quality(row: &OrderInfo, observed_at_ms: i64) -> Option<AccountFieldQuality> {
    match row.order_type {
        OrderType::Limit | OrderType::PostOnly if !positive_finite(row.price) => {
            Some(unavailable_open_order_field(
                row,
                "price",
                AccountFieldQualityStatus::Missing,
                observed_at_ms,
            ))
        }
        OrderType::Market if !non_negative_finite(row.price) => Some(unavailable_open_order_field(
            row,
            "price",
            AccountFieldQualityStatus::Invalid,
            observed_at_ms,
        )),
        _ => None,
    }
}

fn filled_quantity_quality(row: &OrderInfo, observed_at_ms: i64) -> Option<AccountFieldQuality> {
    (!non_negative_finite(row.filled_quantity)
        || row.filled_quantity > row.quantity.max(0.0) + f64::EPSILON)
        .then(|| {
            unavailable_open_order_field(
                row,
                "filledQuantity",
                AccountFieldQualityStatus::Invalid,
                observed_at_ms,
            )
        })
}

fn filled_price_quality(row: &OrderInfo, observed_at_ms: i64) -> Option<AccountFieldQuality> {
    (row.filled_quantity > 0.0 && !positive_finite(row.filled_price)).then(|| {
        unavailable_open_order_field(
            row,
            "filledPrice",
            AccountFieldQualityStatus::Missing,
            observed_at_ms,
        )
    })
}

fn open_order_status_quality(row: &OrderInfo, observed_at_ms: i64) -> Option<AccountFieldQuality> {
    matches!(
        row.status,
        OrderStatus::Filled | OrderStatus::Canceled | OrderStatus::Rejected | OrderStatus::Expired
    )
    .then(|| {
        unavailable_open_order_field(
            row,
            "status",
            AccountFieldQualityStatus::Invalid,
            observed_at_ms,
        )
    })
}

fn positive_finite(value: f64) -> bool {
    value.is_finite() && value > 0.0
}
fn non_negative_finite(value: f64) -> bool {
    value.is_finite() && value >= 0.0
}

fn unavailable_open_order_field(
    row: &OrderInfo,
    field: &'static str,
    status: AccountFieldQualityStatus,
    observed_at_ms: i64,
) -> AccountFieldQuality {
    AccountFieldQuality::new(
        open_order_subject(row),
        field,
        status,
        OPEN_ORDERS_SOURCE,
        Some(observed_at_ms),
    )
    .with_problem(open_order_field_problem(row, field, status, observed_at_ms))
}

fn open_order_subject(row: &OrderInfo) -> AccountFieldSubject {
    AccountFieldSubject::open_order(
        &row.exchange,
        &row.order_id,
        &row.symbol,
        order_side_label(row.side),
    )
}

fn open_order_field_problem(
    row: &OrderInfo,
    field: &'static str,
    status: AccountFieldQualityStatus,
    observed_at_ms: i64,
) -> ApiProblem {
    let mut problem = ApiProblem::new(
        codes::OPEN_ORDER_FIELD_UNAVAILABLE,
        format!("open order field {field} is {status:?}"),
    )
    .with_status(StatusCode::OK.as_u16())
    .with_request_id(common::request_id::current())
    .with_source(OPEN_ORDERS_SOURCE);
    problem.details = Some(serde_json::json!({
        "venue": row.exchange.as_str(), "symbol": row.symbol.as_str(), "side": order_side_label(row.side),
        "orderId": row.order_id.as_str(), "field": field, "status": status,
        "operation": OPEN_ORDERS_OPERATION, "path": OPEN_ORDERS_ROUTE, "source": OPEN_ORDERS_SOURCE,
        "observedAtMs": observed_at_ms,
    }));
    problem
}

fn order_side_label(side: shared_types::OrderSide) -> &'static str {
    match side {
        shared_types::OrderSide::Buy => "buy",
        shared_types::OrderSide::Sell => "sell",
    }
}
