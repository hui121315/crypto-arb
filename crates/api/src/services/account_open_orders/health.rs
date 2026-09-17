use super::*;
use shared_types::{normalized_venue_name, AccountDataHealth, AccountFieldSubject, OrderSide};

pub(super) fn open_order_row_health(
    rows: &[OrderInfo],
    operation_health: &[VenueOperationHealth],
    route_failures: &[RouteFailure],
    route_failure_problems: &[ApiProblem],
    observed_at_ms: i64,
) -> Vec<AccountDataHealth> {
    let mut row_health = rows
        .iter()
        .map(|row| {
            open_order_data_health(
                row,
                open_order_health_for_venue(operation_health, &row.exchange),
                observed_at_ms,
            )
        })
        .collect::<Vec<_>>();
    row_health.extend(route_failure_account_health(
        route_failures,
        route_failure_problems,
        observed_at_ms,
    ));
    row_health
}

fn route_failure_account_health(
    route_failures: &[RouteFailure],
    route_failure_problems: &[ApiProblem],
    observed_at_ms: i64,
) -> Vec<AccountDataHealth> {
    route_failures
        .iter()
        .zip(route_failure_problems)
        .map(|(failure, problem)| {
            let mut data_health = AccountDataHealth::new(
                AccountFieldSubject::account(&failure.venue),
                problem.source.as_deref().unwrap_or(OPEN_ORDERS_SOURCE),
                observed_at_ms,
            );
            data_health.last_error = Some(problem.clone());
            data_health.retry_after_ms = problem.retry_after_ms;
            data_health.request_id = problem.request_id.clone();
            data_health
        })
        .collect()
}

fn open_order_health_for_venue<'a>(
    rows: &'a [VenueOperationHealth],
    venue: &str,
) -> Option<&'a VenueOperationHealth> {
    let normalized = normalized_venue_name(venue);
    rows.iter()
        .filter(|row| normalized_venue_name(&row.venue) == normalized)
        .find(|row| row.operation == CREDENTIAL_OPEN_ORDERS_PROBE)
}

fn open_order_data_health(
    row: &OrderInfo,
    health: Option<&VenueOperationHealth>,
    observed_at_ms: i64,
) -> AccountDataHealth {
    let mut data_health = AccountDataHealth::new(
        AccountFieldSubject::open_order(
            &row.exchange,
            &row.order_id,
            &row.symbol,
            order_side_label(row.side),
        ),
        health
            .map(|health| health.source.as_str())
            .unwrap_or(OPEN_ORDERS_SOURCE),
        observed_at_ms,
    );
    let Some(health) = health else {
        return data_health;
    };
    data_health.observed_at_ms = health.observed_at_ms;
    data_health.freshness_ms = health.freshness_ms;
    data_health.last_success_ms = open_order_last_success_ms(health);
    data_health.last_error = open_order_last_error(health);
    data_health.retry_after_ms = health.retry_after_ms.or_else(|| {
        data_health
            .last_error
            .as_ref()
            .and_then(|problem| problem.retry_after_ms)
    });
    data_health.request_id = data_health
        .last_error
        .as_ref()
        .and_then(|problem| problem.request_id.clone())
        .or_else(|| {
            health
                .evidence
                .as_ref()
                .and_then(|evidence| evidence.request_id.clone())
        });
    data_health
}

pub(super) fn open_order_last_success_ms(row: &VenueOperationHealth) -> Option<i64> {
    let freshness_ms = row.freshness_ms.filter(|freshness| *freshness >= 0)?;
    (row.status == VenueOperationStatus::Ok)
        .then_some(row.observed_at_ms.saturating_sub(freshness_ms))
}

fn open_order_last_error(row: &VenueOperationHealth) -> Option<ApiProblem> {
    row.problem.clone().or_else(|| {
        row.error.as_ref().map(|message| {
            let mut problem = ApiProblem::new(codes::OPEN_ORDER_READ_DEGRADED, message.clone())
                .with_status(StatusCode::OK.as_u16())
                .with_source(row.source.clone());
            problem.details = Some(serde_json::json!({
                "venue": row.venue.as_str(),
                "operation": row.operation.as_str(),
                "source": row.source.as_str(),
                "observedAtMs": row.observed_at_ms,
            }));
            problem
        })
    })
}

fn order_side_label(side: OrderSide) -> &'static str {
    match side {
        OrderSide::Buy => "buy",
        OrderSide::Sell => "sell",
    }
}
