use crate::services::account_binding;
use crate::state::{AppState, CachedAccountOpenOrders};
use crate::trading_service::RouteFailure;
use axum::http::StatusCode;
use common::AppError;
use shared_types::{
    problem::codes, AccountFieldQuality, AccountFieldQualityStatus, AccountFieldSubject,
    ApiProblem, ListStatus, OrderInfo, OrderStatus, OrderType, VenueOpenOrdersEnvelope,
    VenueOperationHealth, VenueOperationStatus,
};
use std::sync::Arc;

const OPEN_ORDERS_SOURCE: &str = "account_open_orders_runtime";
const OPEN_ORDERS_ROUTE: &str = "/api/trading/orders";
const OPEN_ORDERS_OPERATION: &str = "open_orders";
const CREDENTIAL_OPEN_ORDERS_PROBE: &str = "credential_probe:open_orders_read";
// Binance charges weight 40 for all-symbol openOrders. Private events own immediate
// finality; this full-account REST snapshot is periodic reconciliation only.
const OPEN_ORDERS_REFRESH_INTERVAL_MS: i64 = 30_000;
const OPEN_ORDERS_REFRESH_RETRY_AFTER_MS: u64 = 2_000;

/// Return the latest account snapshot immediately; one background task owns refresh.
pub(crate) fn cached_envelope_with_operation_health(
    state: &AppState,
    operation_health: Vec<VenueOperationHealth>,
) -> VenueOpenOrdersEnvelope {
    let account_cache_epoch = state.trading_service().account_cache_epoch();
    let cached = cached_entry_for_epoch(state, account_cache_epoch);
    let private_ws_change_ms = state.trading_service().open_order_cache_latest_change_ms();
    let needs_refresh = cached.as_ref().is_none_or(|entry| {
        refresh_needed(entry.1, entry.0.private_ws_change_ms, private_ws_change_ms)
    });

    if needs_refresh {
        let fallback = cached
            .as_ref()
            .map(|(entry, _)| entry.envelope.clone())
            .filter(|envelope| envelope.status == ListStatus::Fresh);
        schedule_refresh(
            state.clone(),
            account_cache_epoch,
            private_ws_change_ms,
            operation_health.clone(),
            fallback,
        );
    }

    cached
        .map(|(entry, _)| entry.envelope)
        .unwrap_or_else(|| warming_envelope(operation_health))
}

fn cached_entry_for_epoch(
    state: &AppState,
    account_cache_epoch: u64,
) -> Option<(CachedAccountOpenOrders, i64)> {
    let entry = state.account_open_orders_snapshot().get_now()?;
    if entry.value.account_cache_epoch != account_cache_epoch {
        return None;
    }
    let freshness_ms = realtime::staleness_ms(Some(&entry))
        .unwrap_or(i64::MAX)
        .max(0);
    Some((entry.value, freshness_ms))
}

fn schedule_refresh(
    state: AppState,
    account_cache_epoch: u64,
    private_ws_change_ms: i64,
    operation_health: Vec<VenueOperationHealth>,
    fallback: Option<VenueOpenOrdersEnvelope>,
) {
    let Ok(refresh_guard) = Arc::clone(state.account_open_orders_refresh_lock()).try_lock_owned()
    else {
        return;
    };
    tokio::spawn(async move {
        let _refresh_guard = refresh_guard;
        let refreshed = envelope_with_operation_health(&state, operation_health).await;
        let envelope = match fallback {
            Some(fallback) if transient_refresh_failure(&refreshed) => {
                tracing::warn!(
                    problems = refreshed.problems.len(),
                    "open-order refresh failed transiently; retaining last fresh snapshot"
                );
                fallback
            }
            _ => refreshed,
        };
        if state.trading_service().account_cache_epoch() == account_cache_epoch {
            state.cache_account_open_orders(CachedAccountOpenOrders {
                account_cache_epoch,
                private_ws_change_ms,
                envelope,
            });
        }
    });
}

fn refresh_needed(
    snapshot_age_ms: i64,
    consumed_private_ws_change_ms: i64,
    latest_private_ws_change_ms: i64,
) -> bool {
    snapshot_age_ms >= OPEN_ORDERS_REFRESH_INTERVAL_MS
        || consumed_private_ws_change_ms < latest_private_ws_change_ms
}

fn transient_refresh_failure(envelope: &VenueOpenOrdersEnvelope) -> bool {
    envelope.problems.iter().any(|problem| {
        problem
            .status
            .is_some_and(|status| status == 408 || status == 429 || status >= 500)
    })
}

fn warming_envelope(operation_health: Vec<VenueOperationHealth>) -> VenueOpenOrdersEnvelope {
    let observed_at_ms = common::time::now_ms();
    let mut problem = missing_open_order_evidence_problem();
    problem.retry_after_ms = Some(OPEN_ORDERS_REFRESH_RETRY_AFTER_MS);
    VenueOpenOrdersEnvelope::new(
        Vec::new(),
        ListStatus::Degraded,
        OPEN_ORDERS_SOURCE,
        observed_at_ms,
        vec![problem],
        operation_health,
    )
}

mod health;
mod projection;

#[cfg(test)]
use health::open_order_last_success_ms;
use health::open_order_row_health;
use projection::open_order_field_quality;

pub(crate) async fn envelope_with_operation_health(
    state: &AppState,
    operation_health: Vec<VenueOperationHealth>,
) -> VenueOpenOrdersEnvelope {
    let mut problems = Vec::new();
    let (rows, runtime_read_succeeded) = read_open_order_rows(state, &mut problems).await;
    let observed_at_ms = common::time::now_ms();
    let route_failures = take_route_failures(state);
    let route_failure_problems = route_failure_problems(&route_failures);
    problems.extend(route_failure_problems.iter().cloned());
    if missing_open_order_evidence_needed(&rows, runtime_read_succeeded, &operation_health) {
        problems.push(missing_open_order_evidence_problem());
    }
    let field_quality = open_order_field_quality(&rows, observed_at_ms);
    let row_health = open_order_row_health(
        &rows,
        &operation_health,
        &route_failures,
        &route_failure_problems,
        observed_at_ms,
    );
    let status = open_order_status(&problems, &operation_health, &field_quality);
    let account_bindings = account_binding::evidence_for_venues_with_summaries(
        rows.iter()
            .map(|row| row.exchange.clone())
            .chain(
                operation_health
                    .iter()
                    .filter(|row| row.configured != Some(false))
                    .map(|row| row.venue.clone()),
            )
            .chain(route_failures.iter().map(|row| row.venue.clone())),
        &state.trading_service().account_summaries(),
        observed_at_ms,
    );
    VenueOpenOrdersEnvelope::new(
        rows,
        status,
        OPEN_ORDERS_SOURCE,
        observed_at_ms,
        problems,
        operation_health,
    )
    .with_field_quality(field_quality)
    .with_row_health(row_health)
    .with_account_bindings(account_bindings)
}

pub(crate) fn operation_health_from_rows(
    rows: &[VenueOperationHealth],
) -> Vec<VenueOperationHealth> {
    rows.iter()
        .filter(|row| is_open_order_evidence_row(row))
        .cloned()
        .collect()
}

async fn read_open_order_rows(
    state: &AppState,
    problems: &mut Vec<ApiProblem>,
) -> (Vec<OrderInfo>, bool) {
    match state.trading_service().list_open_orders().await {
        Ok(rows) => (rows, true),
        Err(error) => {
            let error = AppError::from(error);
            problems.push(open_order_read_problem(&error));
            (Vec::new(), false)
        }
    }
}

fn missing_open_order_evidence_needed(
    rows: &[OrderInfo],
    runtime_read_succeeded: bool,
    operation_health: &[VenueOperationHealth],
) -> bool {
    rows.is_empty() && !runtime_read_succeeded && !has_fresh_open_order_evidence(operation_health)
}

fn is_open_order_evidence_row(row: &VenueOperationHealth) -> bool {
    row.operation == CREDENTIAL_OPEN_ORDERS_PROBE
}

fn take_route_failures(state: &AppState) -> Vec<RouteFailure> {
    state
        .trading_service()
        .take_route_failures(OPEN_ORDERS_OPERATION)
}

fn route_failure_problems(route_failures: &[RouteFailure]) -> Vec<ApiProblem> {
    route_failures.iter().map(route_failure_problem).collect()
}

fn route_failure_problem(failure: &RouteFailure) -> ApiProblem {
    let mut problem = failure
        .error
        .to_api_problem()
        .with_source(OPEN_ORDERS_SOURCE);
    problem.details = Some(serde_json::json!({
        "venue": failure.venue.as_str(),
        "operation": failure.operation,
        "path": OPEN_ORDERS_ROUTE,
        "status": failure.error.status().as_u16(),
        "source": OPEN_ORDERS_SOURCE,
    }));
    problem
}

fn open_order_read_problem(error: &AppError) -> ApiProblem {
    let mut problem = error.to_api_problem().with_source(OPEN_ORDERS_SOURCE);
    problem.details = Some(serde_json::json!({
        "operation": OPEN_ORDERS_OPERATION,
        "path": OPEN_ORDERS_ROUTE,
        "status": error.status().as_u16(),
        "source": OPEN_ORDERS_SOURCE,
    }));
    problem
}

fn missing_open_order_evidence_problem() -> ApiProblem {
    let mut problem = ApiProblem::new(
        codes::OPEN_ORDER_EVIDENCE_MISSING,
        "open order rows are empty and no fresh open-order evidence is available",
    )
    .with_status(StatusCode::OK.as_u16())
    .with_request_id(common::request_id::current())
    .with_source(OPEN_ORDERS_SOURCE);
    problem.details = Some(serde_json::json!({
        "operation": OPEN_ORDERS_OPERATION,
        "path": OPEN_ORDERS_ROUTE,
        "source": OPEN_ORDERS_SOURCE,
    }));
    problem
}

fn open_order_status(
    problems: &[ApiProblem],
    operation_health: &[VenueOperationHealth],
    field_quality: &[AccountFieldQuality],
) -> ListStatus {
    if !problems.is_empty()
        || operation_health.iter().any(open_order_attention_row)
        || field_quality.iter().any(open_order_field_needs_attention)
    {
        ListStatus::Degraded
    } else {
        ListStatus::Fresh
    }
}

fn open_order_attention_row(row: &VenueOperationHealth) -> bool {
    row.configured != Some(false)
        && matches!(
            row.status,
            VenueOperationStatus::Warn
                | VenueOperationStatus::Blocked
                | VenueOperationStatus::Unknown
        )
}

fn has_fresh_open_order_evidence(operation_health: &[VenueOperationHealth]) -> bool {
    operation_health.iter().any(|row| {
        row.status == VenueOperationStatus::Ok && row.operation == CREDENTIAL_OPEN_ORDERS_PROBE
    })
}

fn open_order_field_needs_attention(row: &AccountFieldQuality) -> bool {
    row.status != AccountFieldQualityStatus::Actual
}

#[cfg(test)]
mod tests;
