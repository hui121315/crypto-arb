use super::*;

pub(crate) fn live_operation_health_guard(
    mode: ExecutionMode,
    plans: &[&OrderCompilePlan],
    rows: &[VenueOperationHealth],
) -> Option<ExecutionGuard> {
    if mode != ExecutionMode::Live || plans.is_empty() {
        return None;
    }
    let blockers = live_operation_health_blockers(plans, rows);
    let passed = blockers.is_empty();
    let detail = if passed {
        "通过".to_owned()
    } else {
        format!("实盘运行态阻断: {}", blockers.join("; "))
    };
    Some(ExecutionGuard {
        key: "live_operation_health".to_owned(),
        label: "实盘运行态证据".to_owned(),
        passed,
        detail: detail.clone(),
        preflight_outcome: Some(MarginPreflightOutcome {
            status: if passed {
                HedgePreflightStatus::Passed
            } else {
                HedgePreflightStatus::Blocked
            },
            checked_at_ms: common::time::now_ms(),
            scope: live_operation_health_scope(plans),
            observed_venues: observed_live_operation_health_venues(plans, rows),
            balance_rows: Vec::new(),
            source: Some(
                "venue_operation_health.private_rest+account_cache(balance+positions)+order_write_runtime+private_ws_runtime(session+subscribe)"
                    .to_owned(),
            ),
            freshness_ms: max_live_operation_freshness(plans, rows),
            retry_after_ms: max_live_operation_retry_after(plans, rows),
            request_id: live_operation_request_id(plans, rows),
            problems: live_operation_problems(plans, rows),
            field_quality: Vec::new(),
            row_health: live_operation_row_health(plans, rows),
            error: (!passed).then_some(detail),
        }),
    })
}

pub(super) fn account_mode_detail(blockers: &[String]) -> String {
    if blockers.is_empty() {
        "通过".to_owned()
    } else {
        format!("账户模式阻断: {}", blockers.join("; "))
    }
}

pub(super) fn order_write_blockers(checks: &[OrderWriteCheck<'_>]) -> Vec<String> {
    checks
        .iter()
        .filter_map(|check| {
            check.preflight.as_ref().err().map(|error| {
                format!(
                    "{} {} 下单准入不可用: {error}",
                    check.plan.exchange, check.plan.symbol
                )
            })
        })
        .fold(Vec::new(), push_unique)
}

pub(super) fn order_write_scope(checks: &[OrderWriteCheck<'_>]) -> HedgePreflightScope {
    HedgePreflightScope {
        venues: order_write_venues(checks),
        symbols: order_write_symbols(checks),
        account_modes: Vec::new(),
        operations: vec![HedgePreflightOperation::OrderWrite],
    }
}

pub(super) fn order_write_venues(checks: &[OrderWriteCheck<'_>]) -> Vec<String> {
    checks
        .iter()
        .map(|check| normalized_venue_name(&check.plan.exchange))
        .filter(|venue| !venue.is_empty())
        .fold(Vec::new(), push_unique)
}

pub(super) fn observed_order_write_venues(checks: &[OrderWriteCheck<'_>]) -> Vec<String> {
    checks
        .iter()
        .filter(|check| check.preflight.is_ok())
        .map(|check| normalized_venue_name(&check.plan.exchange))
        .filter(|venue| !venue.is_empty())
        .fold(Vec::new(), push_unique)
}

pub(super) fn order_write_symbols(checks: &[OrderWriteCheck<'_>]) -> Vec<String> {
    checks
        .iter()
        .map(|check| check.plan.symbol.clone())
        .fold(Vec::new(), push_unique)
}

pub(super) fn live_operation_health_blockers(
    plans: &[&OrderCompilePlan],
    rows: &[VenueOperationHealth],
) -> Vec<String> {
    plans
        .iter()
        .flat_map(|plan| {
            required_live_operations()
                .into_iter()
                .filter_map(move |required| live_operation_blocker(plan, rows, &required))
        })
        .fold(Vec::new(), push_unique)
}

pub(super) fn live_operation_blocker(
    plan: &OrderCompilePlan,
    rows: &[VenueOperationHealth],
    required: &RequiredLiveOperation,
) -> Option<String> {
    let Some(row) = live_operation_row(rows, &plan.exchange, required.operation) else {
        return Some(format!(
            "{} {} 缺少{}运行态证据",
            plan.exchange, plan.symbol, required.label
        ));
    };
    (row.status != VenueOperationStatus::Ok).then(|| {
        format!(
            "{} {} {} 未通过: {}",
            plan.exchange, plan.symbol, required.label, row.message
        )
    })
}

pub(super) fn live_operation_row<'a>(
    rows: &'a [VenueOperationHealth],
    venue: &str,
    operation: &str,
) -> Option<&'a VenueOperationHealth> {
    let exact = normalized_venue_name(venue);
    let exact_row = rows
        .iter()
        .find(|row| normalized_venue_name(&row.venue) == exact && row.operation == operation);
    if exact_row.is_some() || !allows_family_live_operation_fallback(operation) {
        return exact_row;
    }
    let family = normalized_venue_name(venue_family(venue));
    (family != exact).then(|| {
        rows.iter()
            .find(|row| normalized_venue_name(&row.venue) == family && row.operation == operation)
    })?
}

pub(super) fn allows_family_live_operation_fallback(operation: &str) -> bool {
    operation == OP_PRIVATE_READ
}

pub(super) fn required_live_operations() -> [RequiredLiveOperation; 6] {
    [
        RequiredLiveOperation {
            operation: OP_PRIVATE_READ,
            label: "私有 REST",
        },
        RequiredLiveOperation {
            operation: OP_BALANCE,
            label: "余额读取",
        },
        RequiredLiveOperation {
            operation: OP_POSITIONS,
            label: "持仓读取",
        },
        RequiredLiveOperation {
            operation: OP_ORDER_WRITE,
            label: "写单",
        },
        RequiredLiveOperation {
            operation: OP_PRIVATE_WS_SESSION,
            label: "私有 WS 会话",
        },
        RequiredLiveOperation {
            operation: OP_PRIVATE_WS_SUBSCRIBE,
            label: "私有 WS 订阅",
        },
    ]
}

#[derive(Debug, Clone, Copy)]
pub(super) struct RequiredLiveOperation {
    pub(super) operation: &'static str,
    pub(super) label: &'static str,
}

pub(super) fn live_operation_health_scope(plans: &[&OrderCompilePlan]) -> HedgePreflightScope {
    HedgePreflightScope {
        venues: plans
            .iter()
            .map(|plan| normalized_venue_name(&plan.exchange))
            .filter(|venue| !venue.is_empty())
            .fold(Vec::new(), push_unique),
        symbols: plans
            .iter()
            .map(|plan| plan.symbol.clone())
            .fold(Vec::new(), push_unique),
        account_modes: Vec::new(),
        operations: vec![
            HedgePreflightOperation::PrivateRead,
            HedgePreflightOperation::MarginBalance,
            HedgePreflightOperation::OrderWrite,
            HedgePreflightOperation::Positions,
            HedgePreflightOperation::PrivateWs,
        ],
    }
}

pub(super) fn observed_live_operation_health_venues(
    plans: &[&OrderCompilePlan],
    rows: &[VenueOperationHealth],
) -> Vec<String> {
    plans
        .iter()
        .filter(|plan| live_operation_venue_passed(plan, rows))
        .map(|plan| normalized_venue_name(&plan.exchange))
        .filter(|venue| !venue.is_empty())
        .fold(Vec::new(), push_unique)
}

pub(super) fn live_operation_venue_passed(
    plan: &OrderCompilePlan,
    rows: &[VenueOperationHealth],
) -> bool {
    required_live_operations().into_iter().all(|required| {
        live_operation_row(rows, &plan.exchange, required.operation)
            .is_some_and(|row| row.status == VenueOperationStatus::Ok)
    })
}

pub(super) fn live_operation_row_health(
    plans: &[&OrderCompilePlan],
    rows: &[VenueOperationHealth],
) -> Vec<AccountDataHealth> {
    live_operation_rows(plans, rows)
        .map(live_operation_data_health)
        .fold(Vec::new(), push_unique_live_operation_health)
}

fn live_operation_data_health(row: &VenueOperationHealth) -> AccountDataHealth {
    let mut health = AccountDataHealth::new(
        AccountFieldSubject::account(row.venue.clone()),
        live_operation_health_source(row),
        row.observed_at_ms,
    );
    health.freshness_ms = row.freshness_ms;
    health.last_success_ms = (row.status == VenueOperationStatus::Ok).then_some(row.observed_at_ms);
    health.last_error = live_operation_last_error(row);
    health.retry_after_ms = live_operation_retry_after(row);
    health.request_id = operation_request_id(row);
    health
}
