use super::*;

mod target;
use target::QuoteTarget;
#[cfg(test)]
mod tests;

pub(super) struct PreparedLegQuote {
    spec: LegSpec,
    role: HedgeLegRole,
    now_ms: i64,
    depth_bps: f64,
    blockers: Vec<String>,
    orderbook: OrderbookQuote,
}

impl PreparedLegQuote {
    pub(super) fn build_for_notional(&self, target_notional: f64) -> HedgeLegQuote {
        self.build(QuoteTarget::Notional(target_notional))
    }

    pub(super) fn build_for_base_quantity(&self, target_base_quantity: f64) -> HedgeLegQuote {
        self.build(QuoteTarget::BaseQuantity(target_base_quantity))
    }

    fn build(&self, target: QuoteTarget) -> HedgeLegQuote {
        build_leg_quote_from_orderbook(
            QuoteBuildInput {
                spec: &self.spec,
                role: self.role,
                target,
                now_ms: self.now_ms,
                depth_bps: self.depth_bps,
                blockers: self.blockers.clone(),
            },
            &self.orderbook,
        )
    }
}

pub(super) async fn prepare_leg_quote(
    state: &AppState,
    opp: &ArbitrageOpportunityDto,
    role: HedgeLegRole,
    now_ms: i64,
    depth_bps: f64,
) -> PreparedLegQuote {
    let spec = LegSpec::from_opp(opp, role);
    prepare_leg_quote_from_spec(state, spec, role, now_ms, depth_bps).await
}

async fn prepare_leg_quote_from_spec(
    state: &AppState,
    spec: LegSpec,
    role: HedgeLegRole,
    now_ms: i64,
    depth_bps: f64,
) -> PreparedLegQuote {
    let blockers = leg_route_blockers(state, &spec);
    let orderbook = orderbook(state, &spec, now_ms).await;
    PreparedLegQuote {
        spec,
        role,
        now_ms,
        depth_bps,
        blockers,
        orderbook,
    }
}

pub(super) async fn refresh_ticket_leg_quote_for_base_quantity(
    state: &AppState,
    ticket: &HedgeTicket,
    leg: &HedgeLegQuote,
    target_base_quantity: f64,
    now_ms: i64,
    depth_bps: f64,
) -> HedgeLegQuote {
    let spec = LegSpec::from_ticket_leg(ticket, leg);
    prepare_leg_quote_from_spec(state, spec, leg.role, now_ms, depth_bps)
        .await
        .build_for_base_quantity(target_base_quantity)
}

fn leg_route_blockers(state: &AppState, spec: &LegSpec) -> Vec<String> {
    let mut blockers = live_route_blockers(state, spec);
    if spec.book_kind == LegBookKind::Unresolved {
        blockers.push(format!(
            "{} {} 执行盘口类型未解析，等待现货腿方向证据",
            spec.exchange, spec.symbol
        ));
    }
    blockers
}

struct QuoteBuildInput<'a> {
    spec: &'a LegSpec,
    role: HedgeLegRole,
    target: QuoteTarget,
    now_ms: i64,
    depth_bps: f64,
    blockers: Vec<String>,
}

fn build_leg_quote_from_orderbook(
    input: QuoteBuildInput<'_>,
    orderbook: &OrderbookQuote,
) -> HedgeLegQuote {
    let QuoteBuildInput {
        spec,
        role,
        target,
        now_ms,
        depth_bps,
        mut blockers,
    } = input;
    blockers.extend(orderbook.blockers.iter().cloned());
    let depth_reason = orderbook
        .blockers
        .first()
        .cloned()
        .filter(|reason| !reason.is_empty());
    let market_health = orderbook.health(now_ms);
    let display_book = orderbook.display_book();
    let executable_book = orderbook.executable_book();
    let book_reference = display_book.and_then(|book| reference_price(book, spec.side));
    let reference = book_reference.or(spec.fallback_price);
    let market_evidence = Some(leg_market_evidence(
        spec,
        reference,
        book_reference.is_some(),
        &market_health,
        now_ms,
    ));
    let depth_health = Some(market_health);
    let depth = executable_book
        .and_then(|book| depth_usd_within_bps(book, spec.side, reference, depth_bps));
    let depth_5bps =
        executable_book.and_then(|book| depth_usd_within_bps(book, spec.side, reference, 5.0));
    let depth_10bps =
        executable_book.and_then(|book| depth_usd_within_bps(book, spec.side, reference, 10.0));
    let depth_20bps =
        executable_book.and_then(|book| depth_usd_within_bps(book, spec.side, reference, 20.0));
    let open_vwap = executable_book.and_then(|book| target.vwap(book, spec.side));
    let required_open_notional = target.required_notional(open_vwap);
    let open_slippage = slippage_bps(reference, open_vwap, spec.side);
    let close_side = reverse_side(spec.side);
    let close_reference = display_book.and_then(|book| reference_price(book, close_side));
    let close_vwap = executable_book.and_then(|book| target.vwap(book, close_side));
    let close_slippage = slippage_bps(close_reference, close_vwap, close_side);
    if stale_market(display_book, now_ms) {
        blockers.push(format!(
            "{} {} orderbook 超过 30s 未更新",
            spec.exchange, spec.symbol
        ));
    }
    if let (Some(depth), Some(required_notional)) = (depth, required_open_notional) {
        if depth + f64::EPSILON < required_notional {
            blockers.push(format!(
                "{} {} {} {:.2}% 滑点带内可成交深度不足，目标 ${:.0}，可用 ${:.0}",
                spec.exchange,
                spec.symbol,
                order_side_label(spec.side),
                depth_bps / 100.0,
                required_notional,
                depth
            ));
        }
    }

    HedgeLegQuote {
        role,
        exchange: spec.exchange.clone(),
        symbol: spec.symbol.clone(),
        side: spec.side,
        reference_price: reference,
        bid: display_book.and_then(OrderBookInfo::best_bid),
        ask: display_book.and_then(OrderBookInfo::best_ask),
        mid: display_book.and_then(mid_price),
        open_vwap_price: open_vwap,
        open_slippage_bps: open_slippage,
        close_vwap_price: close_vwap,
        close_slippage_bps: close_slippage,
        depth_usd_5bps: depth_5bps,
        depth_usd_10bps: depth_10bps,
        depth_usd_20bps: depth_20bps,
        max_notional_usd: depth,
        market_evidence,
        depth_health,
        depth_reason,
        funding_bps: Some(spec.funding_rate * 10_000.0),
        next_funding_time: spec.next_funding_time,
        funding_interval_hours: spec.funding_interval_hours,
        market_timestamp_ms: display_book.map(|book| book.timestamp),
        blockers: dedup(blockers),
    }
}

pub(super) fn leg_market_evidence(
    spec: &LegSpec,
    reference: Option<f64>,
    from_orderbook: bool,
    orderbook_health: &MarketDataHealth,
    now_ms: i64,
) -> OpportunityLegMarketEvidence {
    let price = positive_price(reference);
    let health = if from_orderbook || price.is_none() {
        orderbook_health.clone()
    } else {
        snapshot_fallback_health(spec, orderbook_health, now_ms)
    };
    OpportunityLegMarketEvidence {
        venue: spec.exchange.clone(),
        symbol: spec.symbol.clone(),
        price,
        health,
    }
}

pub(super) fn snapshot_fallback_health(
    spec: &LegSpec,
    orderbook_health: &MarketDataHealth,
    now_ms: i64,
) -> MarketDataHealth {
    MarketDataHealth {
        quality: MarketDataQuality::Unverified,
        source: MarketDataSourceKind::LocalCache,
        freshness_ms: None,
        retry_after_ms: orderbook_health.retry_after_ms,
        last_error: orderbook_health.last_error.clone(),
        observed_at_ms: now_ms,
        coverage: orderbook_health.coverage.clone(),
        problem: Some(ApiProblem::new(
            "LEG_PRICE_SNAPSHOT_FALLBACK",
            format!(
                "{} {} 使用机会快照参考价，等待 fresh orderbook",
                spec.exchange, spec.symbol
            ),
        )),
    }
}

pub(super) fn positive_price(value: Option<f64>) -> Option<f64> {
    value.filter(|price| price.is_finite() && *price > f64::EPSILON)
}

pub(super) fn live_route_blockers(state: &AppState, spec: &LegSpec) -> Vec<String> {
    let risk = state.trading_service().risk_config();
    live_route_blockers_for(&risk, spec)
}

pub(super) fn live_route_blockers_for(risk: &trading::RiskConfig, spec: &LegSpec) -> Vec<String> {
    if !risk.live_trading_enabled
        || trading::exchange_allowed(&risk.allowed_exchanges, &spec.exchange)
    {
        Vec::new()
    } else {
        vec![format!(
            "{} {} 未通过实盘路由校验：请先保存并验证该交易所 API",
            spec.exchange, spec.symbol
        )]
    }
}

pub(super) async fn orderbook(state: &AppState, spec: &LegSpec, now_ms: i64) -> OrderbookQuote {
    let read = fetch_leg_orderbook(state, spec, EXECUTION_ORDERBOOK_ROWS, now_ms).await;
    orderbook_quote_from_read(&spec.exchange, &spec.symbol, read)
}
