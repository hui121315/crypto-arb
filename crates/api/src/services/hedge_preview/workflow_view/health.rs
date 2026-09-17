use shared_types::hedge::HedgeTicketOrderPlanEvidence;
use shared_types::{
    problem::codes, venue_names_equal, ApiProblem, ExecutionGuard, HedgeLegQuote, HedgeLegRole,
    HedgePreflightStatus, HedgeTicket, MarketDataQuality, MarketDataSourceKind, ResourceStatus,
    TradeFeeSnapshot, TradeFeeSource, WorkflowEvidenceHealth,
};

pub(super) fn market_health(quote: &HedgeLegQuote) -> WorkflowEvidenceHealth {
    let Some(evidence) = quote.market_evidence.as_ref() else {
        return missing_health(
            "ticket_market_evidence",
            codes::OPPORTUNITY_MARKET_DATA_DEGRADED,
            "票据腿缺少可执行行情证据",
        );
    };
    let source = market_source(evidence.health.source).to_owned();
    let status = match evidence.health.quality {
        MarketDataQuality::Fresh => ResourceStatus::Ready,
        MarketDataQuality::StaleAllowed => ResourceStatus::Degraded,
        MarketDataQuality::StaleBlocked
        | MarketDataQuality::Missing
        | MarketDataQuality::RateLimited
        | MarketDataQuality::CircuitOpen
        | MarketDataQuality::Unsupported
        | MarketDataQuality::Unverified => ResourceStatus::Error,
    };
    let problem = evidence.health.problem.clone().or_else(|| {
        (status != ResourceStatus::Ready).then(|| {
            ApiProblem::new(
                codes::OPPORTUNITY_MARKET_DATA_DEGRADED,
                evidence
                    .health
                    .last_error
                    .clone()
                    .unwrap_or_else(|| "票据腿行情证据不是 Fresh".to_owned()),
            )
            .with_source(source.clone())
            .with_retry_after_ms(evidence.health.retry_after_ms)
        })
    });
    WorkflowEvidenceHealth {
        status,
        source: Some(source),
        evidence_id: Some(format!(
            "market:{}:{}:{}",
            evidence.venue, evidence.symbol, evidence.health.observed_at_ms
        )),
        observed_at_ms: Some(evidence.health.observed_at_ms),
        retry_after_ms: evidence.health.retry_after_ms,
        request_id: problem
            .as_ref()
            .and_then(|problem| problem.request_id.clone()),
        problem,
    }
}

pub(super) fn fee_health(
    ticket: &HedgeTicket,
    quote: &HedgeLegQuote,
    plan: &shared_types::OrderCompilePlan,
    now_ms: i64,
) -> WorkflowEvidenceHealth {
    let snapshot = ticket.fee_snapshots.iter().find(|snapshot| {
        venue_names_equal(&snapshot.venue, &quote.exchange)
            && snapshot.symbol.eq_ignore_ascii_case(&quote.symbol)
            && snapshot.product == plan.product
    });
    let Some(snapshot) = snapshot else {
        return missing_health(
            "ticket_fee_snapshot",
            codes::PROFITABILITY_FEE_EVIDENCE_MISSING,
            "票据腿缺少匹配 venue/symbol/product 的费率快照",
        );
    };
    fee_snapshot_health(snapshot, now_ms)
}

pub(super) fn fee_snapshot_health(
    snapshot: &TradeFeeSnapshot,
    now_ms: i64,
) -> WorkflowEvidenceHealth {
    let source = fee_source(snapshot).to_owned();
    let ready = snapshot.is_fresh_verified(now_ms);
    let problem = (!ready).then(|| {
        ApiProblem::new(
            codes::PROFITABILITY_FEE_EVIDENCE_MISSING,
            snapshot
                .verification_problem
                .clone()
                .unwrap_or_else(|| "票据腿费率快照已过期或未经验证".to_owned()),
        )
        .with_source(source.clone())
    });
    WorkflowEvidenceHealth {
        status: if ready {
            ResourceStatus::Ready
        } else {
            ResourceStatus::Error
        },
        source: Some(source),
        evidence_id: snapshot
            .evidence
            .as_ref()
            .map(|evidence| evidence.evidence_id.clone())
            .or_else(|| {
                Some(format!(
                    "fee:{}:{}:{}",
                    snapshot.venue, snapshot.symbol, snapshot.fetched_at_ms
                ))
            }),
        observed_at_ms: Some(snapshot.fetched_at_ms),
        retry_after_ms: None,
        request_id: None,
        problem,
    }
}

pub(super) fn preflight_health(
    guard: Option<&ExecutionGuard>,
    venue: &str,
    evidence_kind: &str,
    problem_code: &str,
    missing_message: &str,
) -> WorkflowEvidenceHealth {
    let Some(outcome) = guard.and_then(|guard| guard.preflight_outcome.as_ref()) else {
        return missing_health(evidence_kind, problem_code, missing_message);
    };
    if !outcome.scope.venues.is_empty()
        && !outcome
            .scope
            .venues
            .iter()
            .any(|scoped| venue_names_equal(scoped, venue))
    {
        return missing_health(evidence_kind, problem_code, missing_message);
    }
    let status = match outcome.status {
        HedgePreflightStatus::Passed | HedgePreflightStatus::Skipped => ResourceStatus::Ready,
        HedgePreflightStatus::Blocked | HedgePreflightStatus::Failed => ResourceStatus::Error,
    };
    let source = outcome
        .source
        .clone()
        .unwrap_or_else(|| evidence_kind.to_owned());
    let problem = outcome.problems.first().cloned().or_else(|| {
        (status == ResourceStatus::Error).then(|| {
            ApiProblem::new(
                problem_code,
                outcome
                    .error
                    .clone()
                    .unwrap_or_else(|| missing_message.to_owned()),
            )
            .with_source(source.clone())
            .with_request_id(outcome.request_id.clone())
            .with_retry_after_ms(outcome.retry_after_ms)
        })
    });
    WorkflowEvidenceHealth {
        status,
        source: Some(source),
        evidence_id: Some(format!(
            "{}:{}:{}",
            evidence_kind, venue, outcome.checked_at_ms
        )),
        observed_at_ms: Some(outcome.checked_at_ms),
        retry_after_ms: outcome.retry_after_ms,
        request_id: outcome.request_id.clone(),
        problem,
    }
}

pub(super) fn capability_health(
    guard: Option<&ExecutionGuard>,
    plan: &HedgeTicketOrderPlanEvidence,
) -> WorkflowEvidenceHealth {
    let compile = &plan.compile_plan;
    let mut health = preflight_health(
        guard,
        &compile.exchange,
        "ticket_order_capability",
        codes::UNSUPPORTED_CAPABILITY,
        "票据腿缺少下单能力证据",
    );
    let source = compile.venue_capability.source.trim();
    let compile_ready = compile.blockers.is_empty() && !source.is_empty();
    if !source.is_empty() {
        health.source = Some(source.to_owned());
    }
    health.evidence_id = Some(format!(
        "capability:{}:{}:{}",
        role_token(compile.role),
        compile.exchange,
        compile.symbol
    ));
    if compile_ready && guard.is_none() {
        health.status = ResourceStatus::Ready;
        health.problem = None;
    } else if !compile_ready {
        health.status = ResourceStatus::Error;
        health.problem = Some(
            ApiProblem::new(
                codes::UNSUPPORTED_CAPABILITY,
                compile
                    .blockers
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "票据腿 capability source 缺失".to_owned()),
            )
            .with_source(
                health
                    .source
                    .clone()
                    .unwrap_or_else(|| "ticket_order_capability".to_owned()),
            ),
        );
    }
    health
}

fn missing_health(source: &str, code: &str, message: &str) -> WorkflowEvidenceHealth {
    WorkflowEvidenceHealth {
        status: ResourceStatus::Error,
        source: Some(source.to_owned()),
        problem: Some(ApiProblem::new(code, message).with_source(source)),
        ..WorkflowEvidenceHealth::default()
    }
}

fn market_source(source: MarketDataSourceKind) -> &'static str {
    match source {
        MarketDataSourceKind::WsPush => "ws_push",
        MarketDataSourceKind::RestColdStart => "rest_cold_start",
        MarketDataSourceKind::RestBaseline => "rest_baseline",
        MarketDataSourceKind::RestFallback => "rest_fallback",
        MarketDataSourceKind::LocalCache => "local_cache",
    }
}

fn fee_source(snapshot: &TradeFeeSnapshot) -> &str {
    snapshot
        .evidence
        .as_ref()
        .map(|evidence| evidence.source_name.as_str())
        .filter(|source| !source.trim().is_empty())
        .unwrap_or(match snapshot.source {
            TradeFeeSource::AccountApi => "account_api",
            TradeFeeSource::OfficialSchedule => "official_schedule",
            TradeFeeSource::Manual => "manual",
            TradeFeeSource::Unverified => "unverified",
        })
}

fn role_token(role: HedgeLegRole) -> &'static str {
    match role {
        HedgeLegRole::Long => "long",
        HedgeLegRole::Short => "short",
    }
}
