use super::*;
use shared_types::SpotLegMode;

pub(super) struct LegSpec {
    pub(super) exchange: String,
    pub(super) symbol: String,
    pub(super) book_kind: LegBookKind,
    pub(super) side: OrderSide,
    pub(super) fallback_price: Option<f64>,
    pub(super) funding_rate: f64,
    pub(super) next_funding_time: i64,
    pub(super) funding_interval_hours: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LegBookKind {
    Perp,
    Spot,
    Unresolved,
}

impl LegSpec {
    pub(super) fn from_opp(opp: &ArbitrageOpportunityDto, role: HedgeLegRole) -> Self {
        let (
            exchange,
            evidence,
            fallback_price,
            side,
            funding_rate,
            next_funding_time,
            funding_interval_hours,
        ) = match role {
            HedgeLegRole::Long => (
                &opp.long_exchange,
                opp.long_leg_market_evidence.as_ref(),
                opp.long_price,
                OrderSide::Buy,
                opp.long_rate,
                opp.long_next_funding_time,
                opp.long_funding_interval,
            ),
            HedgeLegRole::Short => (
                &opp.short_exchange,
                opp.short_leg_market_evidence.as_ref(),
                opp.short_price,
                OrderSide::Sell,
                opp.short_rate,
                opp.short_next_funding_time,
                opp.short_funding_interval,
            ),
        };
        let (symbol, fallback_price) =
            evidence_bound_identity(exchange, evidence, &opp.symbol, fallback_price);
        Self {
            exchange: exchange.to_owned(),
            symbol,
            book_kind: leg_book_kind(opp.strategy_kind, opp.spot_leg_mode, role),
            side,
            fallback_price,
            funding_rate,
            next_funding_time,
            funding_interval_hours,
        }
    }

    pub(super) fn from_ticket_leg(ticket: &HedgeTicket, leg: &HedgeLegQuote) -> Self {
        Self {
            exchange: leg.exchange.clone(),
            symbol: leg.symbol.clone(),
            book_kind: leg_book_kind(ticket.strategy, ticket.spot_leg_mode, leg.role),
            side: leg.side,
            fallback_price: leg.reference_price,
            funding_rate: leg.funding_bps.unwrap_or_default() / 10_000.0,
            next_funding_time: leg.next_funding_time,
            funding_interval_hours: leg.funding_interval_hours,
        }
    }
}

fn leg_book_kind(
    strategy: Option<StrategyKind>,
    spot_mode: Option<SpotLegMode>,
    role: HedgeLegRole,
) -> LegBookKind {
    match p0_hedge_leg_product(strategy, spot_mode, role) {
        Some(FeeProduct::Perp) => LegBookKind::Perp,
        Some(FeeProduct::Spot) => LegBookKind::Spot,
        Some(FeeProduct::Margin | FeeProduct::Unknown) | None => LegBookKind::Unresolved,
    }
}

fn evidence_bound_identity(
    exchange: &str,
    evidence: Option<&OpportunityLegMarketEvidence>,
    fallback_symbol: &str,
    fallback_price: Option<f64>,
) -> (String, Option<f64>) {
    let evidence = evidence.filter(|evidence| {
        shared_types::venue_names_equal(&evidence.venue, exchange)
            && !evidence.symbol.trim().is_empty()
    });
    let Some(evidence) = evidence else {
        return (fallback_symbol.to_owned(), fallback_price);
    };
    (
        evidence.symbol.trim().to_owned(),
        evidence
            .price
            .filter(|price| price.is_finite() && *price > 0.0),
    )
}
