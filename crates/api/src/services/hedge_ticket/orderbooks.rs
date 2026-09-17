use super::*;

pub(super) const EXECUTION_ORDERBOOK_ROWS: u32 = 20;

pub(crate) struct CachedLegOrderbook {
    pub(crate) venue: String,
    pub(crate) symbol: String,
    pub(crate) read: MarketRead<OrderBookInfo>,
}

pub(crate) fn cached_opportunity_orderbooks(
    state: &AppState,
    opportunity: &ArbitrageOpportunityDto,
    now_ms: i64,
) -> (CachedLegOrderbook, CachedLegOrderbook) {
    let long_spec = LegSpec::from_opp(opportunity, HedgeLegRole::Long);
    let short_spec = LegSpec::from_opp(opportunity, HedgeLegRole::Short);
    (
        cached_leg(state, long_spec, now_ms),
        cached_leg(state, short_spec, now_ms),
    )
}

fn cached_leg(state: &AppState, spec: LegSpec, now_ms: i64) -> CachedLegOrderbook {
    let read = match spec.book_kind {
        LegBookKind::Perp => {
            state
                .market_data()
                .orderbook_read(&spec.exchange, &spec.symbol, now_ms)
        }
        LegBookKind::Spot => {
            state
                .market_data()
                .spot_orderbook_read(&spec.exchange, &spec.symbol, now_ms)
        }
        LegBookKind::Unresolved => unresolved_leg_orderbook(),
    };
    CachedLegOrderbook {
        venue: spec.exchange,
        symbol: spec.symbol,
        read,
    }
}

pub(super) async fn fetch_leg_orderbook(
    state: &AppState,
    spec: &LegSpec,
    depth: u32,
    now_ms: i64,
) -> MarketRead<OrderBookInfo> {
    match spec.book_kind {
        LegBookKind::Perp => {
            state
                .market_data()
                .refresh_orderbook_from_ws(
                    state.aggregator(),
                    &spec.exchange,
                    &spec.symbol,
                    depth,
                    now_ms,
                )
                .await
        }
        LegBookKind::Spot => {
            state
                .market_data()
                .refresh_spot_orderbook_from_ws(
                    state.aggregator(),
                    &spec.exchange,
                    &spec.symbol,
                    depth,
                    now_ms,
                )
                .await
        }
        LegBookKind::Unresolved => unresolved_leg_orderbook(),
    }
}

fn unresolved_leg_orderbook() -> MarketRead<OrderBookInfo> {
    MarketRead {
        value: None,
        quality: MarketQuality::Unsupported,
        freshness_ms: None,
        source: MarketSource::LocalCache,
        retry_after_ms: None,
        last_error: Some("execution leg market is unresolved".to_owned()),
    }
}
