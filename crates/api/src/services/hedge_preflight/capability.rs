use super::*;

pub(crate) struct OrderCapabilityCheck<'a> {
    pub(crate) plan: &'a OrderCompilePlan,
    pub(crate) capabilities: Result<ExchangeCapabilities, String>,
}

impl<'a> OrderCapabilityCheck<'a> {
    pub(crate) fn new(
        plan: &'a OrderCompilePlan,
        capabilities: Result<ExchangeCapabilities, String>,
    ) -> Self {
        Self { plan, capabilities }
    }
}

pub(crate) struct AccountModeCheck<'a> {
    pub(crate) plan: &'a OrderCompilePlan,
    pub(crate) account_mode: Result<Option<VenueAccountModeInfo>, String>,
}

impl<'a> AccountModeCheck<'a> {
    pub(crate) fn new(
        plan: &'a OrderCompilePlan,
        account_mode: Result<Option<VenueAccountModeInfo>, String>,
    ) -> Self {
        Self { plan, account_mode }
    }
}

pub(crate) struct OrderWriteCheck<'a> {
    pub(crate) plan: &'a OrderCompilePlan,
    pub(crate) preflight: Result<(), String>,
}

impl<'a> OrderWriteCheck<'a> {
    pub(crate) fn new(plan: &'a OrderCompilePlan, preflight: Result<(), String>) -> Self {
        Self { plan, preflight }
    }
}

pub(crate) fn account_mode_guard(checks: &[AccountModeCheck<'_>]) -> Option<ExecutionGuard> {
    if checks.is_empty() {
        return None;
    }
    let blockers = account_mode_blockers(checks);
    let passed = blockers.is_empty();
    let detail = account_mode_detail(&blockers);
    Some(ExecutionGuard {
        key: "account_mode".to_owned(),
        label: "账户模式证据".to_owned(),
        passed,
        detail: detail.clone(),
        preflight_outcome: Some(MarginPreflightOutcome {
            status: if passed {
                HedgePreflightStatus::Passed
            } else {
                HedgePreflightStatus::Blocked
            },
            checked_at_ms: common::time::now_ms(),
            scope: account_mode_scope(checks),
            observed_venues: observed_account_mode_venues(checks),
            balance_rows: Vec::new(),
            source: Some(
                "live_trading_adapter.get_exchange_account_mode+hedge_ticket.order_compile_plan"
                    .to_owned(),
            ),
            freshness_ms: max_account_mode_freshness(checks),
            retry_after_ms: None,
            request_id: None,
            problems: Vec::new(),
            field_quality: Vec::new(),
            row_health: Vec::new(),
            error: (!passed).then_some(detail),
        }),
    })
}

pub(crate) fn order_capability_guard(checks: &[OrderCapabilityCheck<'_>]) -> ExecutionGuard {
    let blockers = capability_blockers(checks);
    let passed = blockers.is_empty();
    let detail = capability_detail(&blockers);
    ExecutionGuard {
        key: "order_capability".to_owned(),
        label: "交易所下单能力".to_owned(),
        passed,
        detail: detail.clone(),
        preflight_outcome: Some(MarginPreflightOutcome {
            status: if passed {
                HedgePreflightStatus::Passed
            } else {
                HedgePreflightStatus::Blocked
            },
            checked_at_ms: common::time::now_ms(),
            scope: capability_scope(checks),
            observed_venues: observed_capability_venues(checks),
            balance_rows: Vec::new(),
            source: Some(
                "live_trading_adapter.exchange_capabilities+hedge_ticket.order_compile_plan"
                    .to_owned(),
            ),
            freshness_ms: None,
            retry_after_ms: None,
            request_id: None,
            problems: Vec::new(),
            field_quality: Vec::new(),
            row_health: Vec::new(),
            error: (!passed).then_some(detail),
        }),
    }
}

pub(crate) fn order_write_guard(checks: &[OrderWriteCheck<'_>]) -> Option<ExecutionGuard> {
    if checks.is_empty() {
        return None;
    }
    let blockers = order_write_blockers(checks);
    let passed = blockers.is_empty();
    let detail = if passed {
        "通过".to_owned()
    } else {
        format!("下单准入阻断: {}", blockers.join("; "))
    };
    Some(ExecutionGuard {
        key: "order_write".to_owned(),
        label: "实盘下单准入".to_owned(),
        passed,
        detail: detail.clone(),
        preflight_outcome: Some(MarginPreflightOutcome {
            status: if passed {
                HedgePreflightStatus::Passed
            } else {
                HedgePreflightStatus::Blocked
            },
            checked_at_ms: common::time::now_ms(),
            scope: order_write_scope(checks),
            observed_venues: observed_order_write_venues(checks),
            balance_rows: Vec::new(),
            source: Some(
                "live_trading_adapter.preflight_order+official_order_status_endpoint".to_owned(),
            ),
            freshness_ms: None,
            retry_after_ms: None,
            request_id: None,
            problems: Vec::new(),
            field_quality: Vec::new(),
            row_health: Vec::new(),
            error: (!passed).then_some(detail),
        }),
    })
}
