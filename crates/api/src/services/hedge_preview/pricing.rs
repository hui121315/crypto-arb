use super::*;
use shared_types::{p0_hedge_leg_product, FeeProduct};

pub(crate) fn preview_funding_yield(ticket: &HedgeTicket) -> f64 {
    match ticket.strategy {
        Some(StrategyKind::PerpCross) => ticket
            .cost
            .as_ref()
            .map_or(0.0, |cost| cost.gross_edge_bps / 10_000.0),
        Some(StrategyKind::SpotPerp | StrategyKind::CrossSpotPerp) => {
            ticket_perp_funding_yield(ticket)
        }
        Some(StrategyKind::PerpPriceSpread | StrategyKind::SpotCross) | None => 0.0,
        Some(
            StrategyKind::Triangular
            | StrategyKind::FundingCarry
            | StrategyKind::CashAndCarry
            | StrategyKind::OptionsPerpBasis
            | StrategyKind::QuarterlyPerp
            | StrategyKind::OnchainDepeg,
        ) => 0.0,
    }
}

fn ticket_perp_funding_yield(ticket: &HedgeTicket) -> f64 {
    [&ticket.long_leg, &ticket.short_leg]
        .into_iter()
        .find(|leg| {
            p0_hedge_leg_product(ticket.strategy, ticket.spot_leg_mode, leg.role)
                == Some(FeeProduct::Perp)
        })
        .and_then(|leg| {
            leg.funding_bps.map(|bps| match leg.side {
                shared_types::OrderSide::Buy => -bps / 10_000.0,
                shared_types::OrderSide::Sell => bps / 10_000.0,
            })
        })
        .unwrap_or_default()
}

pub(crate) fn gross_edge_usd(ticket: &HedgeTicket, notional_usd: f64) -> f64 {
    let gross_yield = ticket
        .cost
        .as_ref()
        .map(|cost| cost.gross_edge_bps / 10_000.0)
        .unwrap_or_default();
    notional_usd * gross_yield
}

#[derive(Clone, Copy, Default)]
pub(crate) struct PreviewCosts {
    pub(crate) open_usd: f64,
    pub(crate) close_usd: f64,
    pub(crate) slippage_usd: f64,
}

impl PreviewCosts {
    pub(crate) fn total_usd(self) -> f64 {
        self.open_usd + self.close_usd + self.slippage_usd
    }
}

pub(crate) fn estimated_costs_usd(ticket: &HedgeTicket, notional: f64) -> PreviewCosts {
    let Some(cost) = ticket.cost.as_ref() else {
        return PreviewCosts::default();
    };
    let one = &cost.one_cycle;
    let open_fee_bps = one.open_fee_bps.max(0.0);
    let close_fee_bps = one.close_fee_bps.max(0.0);
    let non_fee_bps = (cost.total_cost_bps - open_fee_bps - close_fee_bps).max(0.0);
    PreviewCosts {
        open_usd: notional * open_fee_bps / 10_000.0,
        close_usd: notional * close_fee_bps / 10_000.0,
        slippage_usd: notional * non_fee_bps / 10_000.0,
    }
}

pub(super) fn record_blocked_preview(
    state: &AppState,
    opp: &ArbitrageOpportunityDto,
    preview: &HedgePreviewResponse,
) {
    if preview_ready(preview) {
        return;
    }
    let Some(strategy) = opp
        .strategy_kind
        .or_else(|| p0_strategy_from_arb_type(opp.arb_type))
    else {
        return;
    };
    let id = format!("missed-{}", preview.idempotency_key);
    let row = MissedOpportunity {
        id: id.clone(),
        opportunity_id: preview.opportunity_id.clone(),
        strategy,
        symbol: opp.symbol.clone(),
        detected_at_ms: common::time::now_ms(),
        expected_pnl_usd: expected_pnl(preview),
        reason: MissReason::RiskBlocked,
        detail: preview_blocked_detail(preview),
    };
    state.missed_opportunities().insert(id, row);
}

fn preview_ready(preview: &HedgePreviewResponse) -> bool {
    preview.long_risk.allowed
        && preview.short_risk.allowed
        && crate::services::hedge_ticket::ticket_ready(&preview.ticket)
}

pub(crate) fn p0_strategy_from_arb_type(arb_type: ArbitrageType) -> Option<StrategyKind> {
    match arb_type {
        ArbitrageType::CrossExchange => Some(StrategyKind::PerpCross),
        ArbitrageType::SpotFutures => Some(StrategyKind::SpotPerp),
        ArbitrageType::CrossSpotFutures => Some(StrategyKind::CrossSpotPerp),
        ArbitrageType::SpotCross => Some(StrategyKind::SpotCross),
        ArbitrageType::Triangular
        | ArbitrageType::FundingCarry
        | ArbitrageType::OptionsPerpBasis => None,
    }
}

fn expected_pnl(preview: &HedgePreviewResponse) -> f64 {
    preview.estimated_gross_edge_usd
        - preview.estimated_open_cost_usd
        - preview.estimated_close_cost_usd
        - preview.estimated_slippage_usd
}

pub(crate) fn blocked_detail(long: &RiskDecision, short: &RiskDecision) -> String {
    format!("多腿: {}; 空腿: {}", risk_detail(long), risk_detail(short))
}

fn preview_blocked_detail(preview: &HedgePreviewResponse) -> String {
    let risk = blocked_detail(&preview.long_risk, &preview.short_risk);
    if preview.ticket.blockers.is_empty() {
        risk
    } else {
        format!("{risk}; 票据: {}", preview.ticket.blockers.join("; "))
    }
}

fn risk_detail(decision: &RiskDecision) -> String {
    if decision.allowed {
        return "通过".into();
    }
    if !decision.evidence.is_empty() {
        return decision
            .evidence
            .iter()
            .map(shared_types::RiskBlockEvidence::compact_summary)
            .collect::<Vec<_>>()
            .join(", ");
    }
    decision
        .reasons
        .iter()
        .map(|reason| format!("{reason:?}"))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
#[path = "pricing/tests.rs"]
mod tests;
