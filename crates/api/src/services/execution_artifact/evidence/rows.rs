use super::market_evidence_expired;
use shared_types::{
    ExecutionArtifactEvidence, ExecutionEnvironment, HedgeLegQuote, HedgePreviewResponse,
    MarketDataQuality, MarketDataSourceKind, StrategyKind, TRANSFER_ROUTE_EVIDENCE_KEY,
};

pub(super) fn transfer_route_evidence(preview: &HedgePreviewResponse) -> ExecutionArtifactEvidence {
    let guard = preview
        .ticket
        .guards
        .iter()
        .find(|guard| guard.key == TRANSFER_ROUTE_EVIDENCE_KEY);
    if let Some(guard) = guard {
        return evidence_row(
            TRANSFER_ROUTE_EVIDENCE_KEY,
            "充提路径",
            guard.passed,
            guard.detail.clone(),
            Some(preview.ticket.created_at_ms),
        );
    }
    let required = matches!(
        preview.ticket.strategy,
        Some(StrategyKind::SpotCross | StrategyKind::SpotPerp | StrategyKind::CrossSpotPerp)
    );
    evidence_row(
        TRANSFER_ROUTE_EVIDENCE_KEY,
        "充提路径",
        !required,
        if required {
            "现货相关策略未绑定充提路径证据".to_owned()
        } else {
            "当前策略不涉及跨场现货再平衡".to_owned()
        },
        Some(preview.ticket.created_at_ms),
    )
}

pub(super) fn profit_lock_evidence(
    proof: &arbitrage::profit_proof::StrategyProfitProof,
    observed_at_ms: i64,
) -> ExecutionArtifactEvidence {
    evidence_row(
        "profit_lock",
        "策略收益证明",
        proof.passed,
        format!("class={}; {}", proof.class.label(), proof.detail),
        Some(observed_at_ms),
    )
}

pub(super) fn cost_evidence(preview: &HedgePreviewResponse) -> ExecutionArtifactEvidence {
    let values = [
        preview.estimated_gross_edge_usd,
        preview.estimated_open_cost_usd,
        preview.estimated_close_cost_usd,
        preview.estimated_slippage_usd,
    ];
    let total = preview.estimated_open_cost_usd
        + preview.estimated_close_cost_usd
        + preview.estimated_slippage_usd;
    let net = preview.estimated_gross_edge_usd - total;
    evidence_row(
        "costs",
        "成本与净收益",
        values
            .iter()
            .all(|value| value.is_finite() && *value >= 0.0)
            && net.is_finite()
            && net > 0.0,
        format!(
            "gross=${:.4}; costs=${total:.4}; net=${net:.4}",
            preview.estimated_gross_edge_usd
        ),
        Some(preview.ticket.created_at_ms),
    )
}

pub(super) fn order_plan_evidence(preview: &HedgePreviewResponse) -> (bool, String) {
    let Some(plans) = preview.ticket_order_plans.as_ref() else {
        return (false, "ticket-bound order plans are missing".to_owned());
    };
    let Ok(rows) = plans.plans_for_ticket(&preview.ticket.ticket_id) else {
        return (
            false,
            "ticket-bound order plans do not match the ticket".to_owned(),
        );
    };
    for row in rows {
        if !row.compile_plan.blockers.is_empty() || !row.identity_plan.is_execution_ready() {
            return (
                false,
                format!(
                    "{} {} execution plan is blocked",
                    row.compile_plan.exchange, row.compile_plan.symbol
                ),
            );
        }
        if preview.long_leg.mode.environment() == ExecutionEnvironment::Live
            && row.compile_plan.validate_sizing_contract().is_err()
        {
            return (
                false,
                format!(
                    "{} {} sizing contract is invalid",
                    row.compile_plan.exchange, row.compile_plan.symbol
                ),
            );
        }
    }
    (
        true,
        "both ticket-bound order plans are executable".to_owned(),
    )
}

pub(super) fn market_evidence_row(
    key: &str,
    label: &str,
    leg: &HedgeLegQuote,
    now_ms: i64,
) -> ExecutionArtifactEvidence {
    let evidence = leg.market_evidence.as_ref();
    let passed = evidence.is_some_and(|item| {
        item.health.quality == MarketDataQuality::Fresh
            && item.health.source == MarketDataSourceKind::WsPush
            && !market_evidence_expired(leg, now_ms)
    });
    let detail = evidence.map_or_else(
        || "market evidence is missing".to_owned(),
        |item| {
            format!(
                "{} {} {:?}/{:?}",
                item.venue, item.symbol, item.health.quality, item.health.source
            )
        },
    );
    evidence_row(
        key,
        label,
        passed,
        detail,
        evidence.map(|item| item.health.observed_at_ms),
    )
}

pub(super) fn depth_observed_at(preview: &HedgePreviewResponse) -> Option<i64> {
    [
        preview
            .ticket
            .long_leg
            .depth_health
            .as_ref()
            .map(|health| health.observed_at_ms),
        preview
            .ticket
            .short_leg
            .depth_health
            .as_ref()
            .map(|health| health.observed_at_ms),
    ]
    .into_iter()
    .flatten()
    .min()
}

pub(super) fn evidence_row(
    key: &str,
    label: &str,
    passed: bool,
    detail: String,
    observed_at_ms: Option<i64>,
) -> ExecutionArtifactEvidence {
    ExecutionArtifactEvidence {
        key: key.to_owned(),
        label: label.to_owned(),
        passed,
        detail,
        observed_at_ms,
    }
}

pub(super) fn money_or_unknown(value: Option<f64>) -> String {
    value
        .filter(|value| value.is_finite())
        .map_or_else(|| "unknown".to_owned(), |value| format!("${value:.2}"))
}
