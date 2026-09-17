use super::*;

pub(super) fn trading_error_text(error: &TradingError) -> String {
    match error {
        TradingError::InsufficientMargin {
            exchange,
            required,
            available,
        } => {
            format!("保证金不足: {exchange} required {required:.4}, available {available:.4}")
        }
        TradingError::RiskBlocked(reasons) => format!("保证金风控阻断: {reasons:?}"),
        TradingError::OrderNotFound(id) => format!("订单不存在: {id}"),
        TradingError::SubmissionInFlight(client_order_id) => {
            format!("同 client_order_id 提交进行中: {client_order_id}")
        }
        TradingError::Exchange(error) => format!("交易所错误: {error}"),
        TradingError::AuditLogUnavailable { reason } => format!("审计链路不可用: {reason}"),
    }
}

pub(super) fn margin_outcome(
    status: HedgePreflightStatus,
    intents: &[&OrderIntent],
    balances: &[VenueBalanceInfo],
    evidence: MarginBalanceEvidence,
    error: Option<String>,
) -> MarginPreflightOutcome {
    let checked_at_ms = common::time::now_ms();
    let problems = margin_problems(
        status,
        intents,
        balances,
        &evidence,
        error.as_deref(),
        checked_at_ms,
    );
    let field_quality =
        margin_field_quality(intents, balances, &evidence, &problems, checked_at_ms);
    let row_health = margin_row_health(intents, balances, &evidence, &problems, checked_at_ms);
    MarginPreflightOutcome {
        status,
        checked_at_ms,
        scope: margin_scope(intents),
        observed_venues: observed_balance_venues(balances),
        balance_rows: balances.to_vec(),
        source: Some(MARGIN_BALANCE_SOURCE.into()),
        freshness_ms: evidence.freshness_ms,
        retry_after_ms: evidence.retry_after_ms,
        request_id: evidence.request_id,
        field_quality,
        row_health,
        problems,
        error,
    }
}

pub(super) fn margin_problems(
    status: HedgePreflightStatus,
    intents: &[&OrderIntent],
    balances: &[VenueBalanceInfo],
    evidence: &MarginBalanceEvidence,
    error: Option<&str>,
    observed_at_ms: i64,
) -> Vec<ApiProblem> {
    if let (HedgePreflightStatus::Failed, Some(message)) = (status, error) {
        return vec![margin_problem(
            codes::MARGIN_BALANCE_READ_FAILED,
            message.to_owned(),
            &required_margin_venues(intents),
            observed_at_ms,
        )];
    }
    let missing = missing_margin_venues(balances, intents);
    if !missing.is_empty() {
        return vec![margin_problem(
            codes::MARGIN_BALANCE_MISSING,
            format!("保证金余额缺少目标交易所数据: {}", missing.join(", ")),
            &missing,
            observed_at_ms,
        )];
    }
    if let Some(problem) = margin_balance_evidence_problem(intents, evidence, observed_at_ms) {
        return vec![problem];
    }
    if let (HedgePreflightStatus::Blocked, Some(message)) = (status, error) {
        vec![margin_problem(
            codes::INSUFFICIENT_MARGIN,
            message.to_owned(),
            &required_margin_venues(intents),
            observed_at_ms,
        )]
    } else {
        Vec::new()
    }
}

pub(super) fn margin_balance_evidence_problem(
    intents: &[&OrderIntent],
    evidence: &MarginBalanceEvidence,
    observed_at_ms: i64,
) -> Option<ApiProblem> {
    let venues = required_margin_venues(intents);
    let missing = venues
        .iter()
        .filter(|venue| {
            margin_balance_cache_health_for_venue(&evidence.operation_health, venue).is_none()
        })
        .cloned()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Some(margin_problem(
            codes::BALANCE_EVIDENCE_MISSING,
            format!("保证金余额缺少当前目标交易所证据: {}", missing.join(", ")),
            &missing,
            observed_at_ms,
        ));
    }

    let degraded = venues
        .iter()
        .filter_map(|venue| {
            margin_balance_cache_health_for_venue(&evidence.operation_health, venue)
                .filter(|row| !row.is_currently_usable())
                .map(|row| (venue, row))
        })
        .collect::<Vec<_>>();
    if degraded.is_empty() {
        return None;
    }

    let affected_venues = degraded
        .iter()
        .map(|(venue, _)| (*venue).clone())
        .collect::<Vec<_>>();
    let mut problem = margin_problem(
        codes::BALANCE_READ_DEGRADED,
        format!("保证金余额证据已降级: {}", affected_venues.join(", ")),
        &affected_venues,
        observed_at_ms,
    );
    if let Some((_, row)) = degraded.first() {
        problem.source = Some(row.source.clone());
        problem.request_id = row_request_id(row).map(str::to_owned);
        problem.retry_after_ms = row_retry_after_ms(row);
    }
    problem.details = Some(json!({
        "venues": affected_venues,
        "operation": MARGIN_BALANCE_OPERATION,
        "observedAtMs": observed_at_ms,
        "source": MARGIN_BALANCE_SOURCE,
        "evidence": degraded.into_iter().map(|(venue, row)| json!({
            "venue": venue,
            "source": row.source,
            "status": row.status,
            "freshnessMs": row.freshness_ms,
            "observedAtMs": row.observed_at_ms,
        })).collect::<Vec<_>>(),
    }));
    Some(problem)
}

pub(super) fn margin_problem(
    code: &'static str,
    message: String,
    venues: &[String],
    observed_at_ms: i64,
) -> ApiProblem {
    let mut problem = ApiProblem::new(code, message).with_source(MARGIN_BALANCE_SOURCE);
    problem.request_id = common::request_id::current();
    problem.details = Some(json!({
        "venues": venues,
        "operation": MARGIN_BALANCE_OPERATION,
        "observedAtMs": observed_at_ms,
        "source": MARGIN_BALANCE_SOURCE,
    }));
    problem
}

pub(super) fn margin_field_quality(
    intents: &[&OrderIntent],
    balances: &[VenueBalanceInfo],
    evidence: &MarginBalanceEvidence,
    problems: &[ApiProblem],
    observed_at_ms: i64,
) -> Vec<AccountFieldQuality> {
    let mut rows =
        margin_collateral_field_quality(intents, balances, evidence, problems, observed_at_ms);
    rows.extend(observed_balance_venues(balances).into_iter().map(|venue| {
        let status = if evidence.account_summaries.iter().any(|summary| {
            margin_balance_venue_matches(&summary.venue, &venue) && usable_summary(summary)
        }) {
            AccountFieldQualityStatus::Actual
        } else {
            AccountFieldQualityStatus::Unknown
        };
        AccountFieldQuality::new(
            AccountFieldSubject::account(venue),
            "equity",
            status,
            MARGIN_BALANCE_SOURCE,
            Some(observed_at_ms),
        )
    }));
    let missing_problem = problems.iter().find(balance_field_problem).cloned();
    for venue in missing_margin_venues(balances, intents) {
        let row = AccountFieldQuality::new(
            AccountFieldSubject::account(venue),
            "available",
            AccountFieldQualityStatus::Missing,
            MARGIN_BALANCE_SOURCE,
            Some(observed_at_ms),
        );
        rows.push(match missing_problem.clone() {
            Some(problem) => row.with_problem(problem),
            None => row,
        });
    }
    rows
}

pub(super) fn balance_field_problem(problem: &&ApiProblem) -> bool {
    problem.code == codes::MARGIN_BALANCE_MISSING
        || problem.code == codes::MARGIN_BALANCE_READ_FAILED
}

pub(super) fn balance_read_error(error: &exchange::ExchangeError, venues: &[String]) -> AppError {
    AppError::domain(
        StatusCode::BAD_GATEWAY,
        codes::MARGIN_BALANCE_READ_FAILED,
        format!("保证金余额读取失败: {error}"),
    )
    .with_details(json!({
        "venues": venues,
        "operation": MARGIN_BALANCE_OPERATION,
    }))
}

pub(super) fn missing_balance_error(venues: &[String]) -> AppError {
    AppError::domain(
        StatusCode::BAD_GATEWAY,
        codes::MARGIN_BALANCE_MISSING,
        format!("保证金余额缺少目标交易所数据: {}", venues.join(", ")),
    )
    .with_details(json!({
        "venues": venues,
        "operation": MARGIN_BALANCE_OPERATION,
    }))
}
