use super::*;

pub(super) fn order_elapsed_ms(orders: &[OrderRecord]) -> Option<u32> {
    let mut total = 0_u64;
    let mut count = 0_u64;
    for order in orders
        .iter()
        .filter(|order| order_has_terminal_elapsed_sample(order))
    {
        let elapsed = (order.updated_at_ms - order.intent.created_at_ms).max(0) as u64;
        total += elapsed;
        count += 1;
    }
    (count > 0).then(|| total.saturating_div(count).min(u32::MAX as u64) as u32)
}

pub(super) fn order_has_terminal_elapsed_sample(order: &OrderRecord) -> bool {
    matches!(
        order.state,
        LiveOrderState::Filled
            | LiveOrderState::Cancelled
            | LiveOrderState::Rejected
            | LiveOrderState::Failed
    )
}

pub(super) fn api_health_or_missing(
    snapshot: &VenueOperationHealthSnapshot,
    problems: &mut Vec<RuntimeProblem>,
    now_ms: i64,
    live_operations_required: bool,
) -> ApiHealthSlot {
    if let Some(health) = api_health_from_operations(snapshot, live_operations_required) {
        return health;
    }
    problems.push(missing_runtime_evidence_problem(
        "trading_api",
        "venue_operation_health",
        "TRADING_API_HEALTH_MISSING",
        "trading API health has no venue operation evidence",
        now_ms,
    ));
    ApiHealthSlot {
        healthy: 0,
        total: 0,
        failed_venues: Vec::new(),
    }
}

pub(super) fn api_health_from_operations(
    snapshot: &VenueOperationHealthSnapshot,
    live_operations_required: bool,
) -> Option<ApiHealthSlot> {
    let has_rows = snapshot.rows.iter().any(is_api_operation_row);
    let mut total = 0_u32;
    let mut healthy = 0_u32;
    let mut failed = BTreeSet::new();
    for row in snapshot.rows.iter().filter(|row| is_api_operation_row(row)) {
        if !live_operations_required && row.configured == Some(false) {
            continue;
        }
        total = total.saturating_add(1);
        if row.is_currently_usable() {
            healthy = healthy.saturating_add(1);
        } else {
            failed.insert(row.venue.clone());
        }
    }
    if !has_rows {
        return None;
    }
    Some(ApiHealthSlot {
        healthy,
        total,
        failed_venues: failed.into_iter().collect(),
    })
}

pub(super) fn ws_health_from_operations(
    snapshot: &VenueOperationHealthSnapshot,
    live_operations_required: bool,
) -> Option<WsHealthSlot> {
    let has_rows = snapshot.rows.iter().any(is_private_ws_row);
    let mut channels = 0_u32;
    let mut disconnected = Vec::new();
    for row in snapshot.rows.iter().filter(|row| is_private_ws_row(row)) {
        if !live_operations_required && row.configured == Some(false) {
            continue;
        }
        channels = channels.saturating_add(1);
        if !row.is_currently_usable() {
            disconnected.push(format!("{}:{}", row.venue, row.operation));
        }
    }
    has_rows.then_some(WsHealthSlot {
        channels,
        disconnected,
    })
}

pub(super) fn ws_health_or_missing(
    snapshot: &VenueOperationHealthSnapshot,
    problems: &mut Vec<RuntimeProblem>,
    now_ms: i64,
    live_operations_required: bool,
) -> WsHealthSlot {
    if let Some(health) = ws_health_from_operations(snapshot, live_operations_required) {
        return health;
    }
    problems.push(missing_runtime_evidence_problem(
        "private_ws",
        "venue_operation_health",
        "PRIVATE_WS_HEALTH_MISSING",
        "private WS health has no venue operation evidence",
        now_ms,
    ));
    WsHealthSlot {
        channels: 0,
        disconnected: Vec::new(),
    }
}

pub(super) fn is_api_operation_row(row: &VenueOperationHealth) -> bool {
    row.supported != Some(false)
        && VenueOperationKind::parse(&row.operation).is_trading_api_status_row()
}

pub(super) fn is_api_transport_row(row: &VenueOperationHealth) -> bool {
    row.supported != Some(false)
        && VenueOperationKind::parse(&row.operation).is_api_transport_status_row()
}

pub(super) fn is_private_ws_row(row: &VenueOperationHealth) -> bool {
    row.supported != Some(false)
        && VenueOperationKind::parse(&row.operation).is_private_ws_status_row()
}

pub(super) fn next_funding_slot(
    rows: &[shared_types::PositionRow],
    now_ms: i64,
) -> Option<NextFundingSlot> {
    rows.iter()
        .filter(|row| row.funding_rate_verified)
        .filter_map(|row| {
            row.next_funding_ms
                .filter(|ts| *ts >= now_ms)
                .map(|ts| (row, ts))
        })
        .min_by_key(|(_, ts)| *ts)
        .map(|(row, ts)| NextFundingSlot {
            symbol: row.symbol.clone(),
            venue: row.venue.clone(),
            minutes_to_settle: ((ts - now_ms).max(0) / 60_000) as u32,
            estimated_outflow_usd: row.quantity.abs() * row.mark_price * row.funding_rate_8h.abs(),
        })
}

pub(super) fn risk_status(
    summary: &shared_types::PortfolioSummary,
    risk: &shared_types::RiskSnapshot,
) -> RiskStatusSlot {
    if risk.hard_limits.kill_switch_active || risk.var_pct_of_nav > 5.0 {
        RiskStatusSlot::Block
    } else if summary.naked_exposure_usd > 1_000.0 || summary.net_delta_pct_of_nav.abs() > 5.0 {
        RiskStatusSlot::Warn
    } else {
        RiskStatusSlot::Ok
    }
}
