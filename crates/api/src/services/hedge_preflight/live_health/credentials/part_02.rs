fn live_operation_health_source(row: &VenueOperationHealth) -> String {
    format!("venue_operation_health:{}:{}", row.operation, row.source)
}

fn live_operation_last_error(row: &VenueOperationHealth) -> Option<ApiProblem> {
    let mut problem = row
        .problem
        .clone()
        .or_else(|| (row.status != VenueOperationStatus::Ok).then(|| live_operation_problem(row)))?;
    if problem.request_id.is_none() {
        problem.request_id = operation_request_id(row);
    }
    if problem.retry_after_ms.is_none() {
        problem.retry_after_ms = live_operation_retry_after(row);
    }
    if problem.details.is_none() {
        problem.details = Some(live_operation_problem_details(row));
    }
    Some(problem)
}

fn live_operation_problem(row: &VenueOperationHealth) -> ApiProblem {
    ApiProblem::new(codes::HEDGE_PRE_TRADE_REJECTED, row.message.clone())
        .with_source(format!("venue_operation_health:{}", row.operation))
        .with_retry_after_ms(live_operation_retry_after(row))
        .with_request_id(operation_request_id(row))
}

fn live_operation_problem_details(row: &VenueOperationHealth) -> serde_json::Value {
    let evidence = row.evidence.as_ref();
    serde_json::json!({
        "venue": row.venue,
        "operation": row.operation,
        "method": evidence.map(|item| item.method.as_str()),
        "path": evidence.map(|item| item.path.as_str()),
        "symbol": live_operation_symbol(row),
        "status": row.status,
        "source": row.source,
        "configured": row.configured,
        "supported": row.supported,
        "latencyMs": row.latency_ms,
        "latencyP95Ms": row.latency_p95_ms,
        "requestContext": evidence.map(|item| &item.request_context),
    })
}

fn live_operation_symbol(row: &VenueOperationHealth) -> Option<&str> {
    row.evidence
        .as_ref()?
        .request_context
        .iter()
        .find_map(|item| {
            let (key, value) = item.split_once('=')?;
            (matches!(key, "symbol" | "instId" | "contract_code")
                && !value.trim().is_empty()
                && value != "not_recorded")
                .then_some(value)
        })
}

fn push_unique_live_operation_health(
    mut values: Vec<AccountDataHealth>,
    value: AccountDataHealth,
) -> Vec<AccountDataHealth> {
    if !values
        .iter()
        .any(|existing| existing.subject == value.subject && existing.source == value.source)
    {
        values.push(value);
    }
    values
}

pub(super) fn max_live_operation_freshness(
    plans: &[&OrderCompilePlan],
    rows: &[VenueOperationHealth],
) -> Option<u64> {
    live_operation_rows(plans, rows)
        .filter_map(|row| row.freshness_ms)
        .filter_map(|value| u64::try_from(value).ok())
        .max()
}

pub(super) fn max_live_operation_retry_after(
    plans: &[&OrderCompilePlan],
    rows: &[VenueOperationHealth],
) -> Option<u64> {
    live_operation_rows(plans, rows)
        .filter_map(live_operation_retry_after)
        .max()
}

fn live_operation_retry_after(row: &VenueOperationHealth) -> Option<u64> {
    row.retry_after_ms.or_else(|| {
        row.problem
            .as_ref()
            .and_then(|problem| problem.retry_after_ms)
    })
}

pub(super) fn live_operation_request_id(
    plans: &[&OrderCompilePlan],
    rows: &[VenueOperationHealth],
) -> Option<String> {
    live_operation_rows(plans, rows).find_map(operation_request_id)
}

pub(super) fn operation_request_id(row: &VenueOperationHealth) -> Option<String> {
    row.problem
        .as_ref()
        .and_then(|problem| problem.request_id.clone())
        .or_else(|| {
            row.evidence
                .as_ref()
                .and_then(|evidence| evidence.request_id.clone())
        })
}

pub(super) fn live_operation_problems(
    plans: &[&OrderCompilePlan],
    rows: &[VenueOperationHealth],
) -> Vec<ApiProblem> {
    live_operation_rows(plans, rows)
        .filter_map(live_operation_last_error)
        .fold(Vec::new(), push_unique_problem)
}

pub(super) fn live_operation_rows<'a>(
    plans: &'a [&'a OrderCompilePlan],
    rows: &'a [VenueOperationHealth],
) -> impl Iterator<Item = &'a VenueOperationHealth> {
    plans.iter().flat_map(move |plan| {
        required_live_operations()
            .into_iter()
            .filter_map(move |required| {
                live_operation_row(rows, &plan.exchange, required.operation)
            })
    })
}
