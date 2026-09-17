use super::*;

const FUNDING_SETTLEMENT_ROLLOVER_GRACE_MS: i64 = 120_000;

pub(super) fn annotate_row_risk(rows: &mut [PositionRow], annotation: RiskAnnotation) {
    for row in rows {
        row.severity = position_severity(
            row.liquidation_distance_pct,
            annotation.warn_pct,
            annotation.danger_pct,
        );
        row.seconds_until_funding = row
            .next_funding_ms
            .map(|ts| ((ts - annotation.now_ms).max(0) / 1_000).min(u32::MAX as i64) as u32);
    }
}

pub(super) fn position_severity(
    distance: Option<f64>,
    warn_pct: f64,
    danger_pct: f64,
) -> PositionSeverity {
    match distance {
        Some(value) if !value.is_finite() => PositionSeverity::Unknown,
        // 距离改为有符号：负值 = mark 已越过强平价，是最强的危险信号
        //（此前负值不可能出现，被当作垃圾数据判 Unknown）。
        Some(value) if value <= danger_pct => PositionSeverity::Danger,
        Some(value) if value <= warn_pct => PositionSeverity::Warn,
        Some(_) => PositionSeverity::Ok,
        None => PositionSeverity::Unknown,
    }
}

pub(super) fn maintenance_margin_ratio(position: &PositionInfo) -> f64 {
    position.maintenance_margin_ratio.max(0.0)
}

#[derive(Debug, Clone, Copy)]
pub(super) struct PositionFundingEvidence {
    pub(super) rate_8h: Option<f64>,
    pub(super) next_funding_ms: Option<i64>,
    observed_at_ms: i64,
}

pub(super) type PositionFundingIndex = HashMap<(String, String), PositionFundingEvidence>;

pub(super) fn funding_evidence_index(
    rates: &[FundingRateData],
    now_ms: i64,
) -> PositionFundingIndex {
    let mut index = HashMap::with_capacity(rates.len());
    for rate in rates {
        let candidate = PositionFundingEvidence {
            rate_8h: rate.rate_8h.is_finite().then_some(rate.rate_8h),
            next_funding_ms: current_funding_timestamp(rate.next_funding_time, now_ms),
            observed_at_ms: rate.timestamp,
        };
        let key = (
            normalized_venue_name(&rate.exchange),
            base_symbol(&rate.symbol).to_ascii_uppercase(),
        );
        index
            .entry(key)
            .and_modify(|current: &mut PositionFundingEvidence| {
                if candidate.observed_at_ms >= current.observed_at_ms {
                    *current = candidate;
                }
            })
            .or_insert(candidate);
    }
    index
}

pub(super) fn dry_run_mark_prices(
    state: &AppState,
    orders: &[OrderRecord],
    now_ms: i64,
) -> HashMap<(String, String), f64> {
    let mut prices = HashMap::new();
    for order in orders.iter().filter(|order| {
        order.intent.mode == ExecutionMode::DryRun && order.state == LiveOrderState::Filled
    }) {
        let canonical_symbol = exchange::strip_common_suffixes(&order.intent.symbol);
        let price = [&order.intent.symbol, &canonical_symbol]
            .into_iter()
            .find_map(|symbol| {
                let read = state
                    .market_data()
                    .ticker_read(&order.intent.exchange, symbol, now_ms);
                (read.quality == MarketQuality::Fresh)
                    .then(|| read.value.as_ref().and_then(ticker_mark_price))
                    .flatten()
            });
        let Some(price) = price else {
            continue;
        };
        prices.insert(
            (
                normalized_venue_name(&order.intent.exchange),
                canonical_symbol.to_ascii_uppercase(),
            ),
            price,
        );
    }
    prices
}

pub(super) fn ticker_mark_price(ticker: &TickerInfo) -> Option<f64> {
    let midpoint = (ticker.bid.is_finite()
        && ticker.ask.is_finite()
        && ticker.bid > 0.0
        && ticker.ask >= ticker.bid)
        .then_some((ticker.bid + ticker.ask) / 2.0);
    midpoint
        .filter(|price| price.is_finite() && *price > 0.0)
        .or_else(|| (ticker.last.is_finite() && ticker.last > 0.0).then_some(ticker.last))
}

pub(super) fn funding_rate_for(
    position: &PositionInfo,
    funding: &PositionFundingIndex,
) -> Option<f64> {
    funding_evidence_for_key(&position.exchange, &position.symbol, funding)
        .and_then(|evidence| evidence.rate_8h)
}

pub(super) fn funding_evidence_for_key(
    venue: &str,
    symbol: &str,
    funding: &PositionFundingIndex,
) -> Option<PositionFundingEvidence> {
    funding
        .get(&(
            normalized_venue_name(venue),
            base_symbol(symbol).to_ascii_uppercase(),
        ))
        .copied()
}

pub(super) fn next_funding_for_position(
    position: &PositionInfo,
    funding: &PositionFundingIndex,
    now_ms: i64,
) -> Option<i64> {
    let private = position
        .next_funding_ms
        .and_then(|timestamp| current_funding_timestamp(timestamp, now_ms));
    let public = funding_evidence_for_key(&position.exchange, &position.symbol, funding)
        .and_then(|evidence| evidence.next_funding_ms);

    private
        .filter(|timestamp| *timestamp > now_ms)
        .or_else(|| public.filter(|timestamp| *timestamp > now_ms))
        .or(private)
        .or(public)
}

fn current_funding_timestamp(timestamp: i64, now_ms: i64) -> Option<i64> {
    let rollover_floor = now_ms.saturating_sub(FUNDING_SETTLEMENT_ROLLOVER_GRACE_MS);
    (timestamp > 0 && timestamp >= rollover_floor).then_some(timestamp)
}

pub(super) fn base_symbol(symbol: &str) -> &str {
    for delimiter in ['/', '-', '_'] {
        if let Some((base, _)) = symbol.split_once(delimiter) {
            return base;
        }
    }
    ["USDT", "USDC", "USD", "PERP"]
        .iter()
        .find_map(|suffix| symbol.strip_suffix(suffix))
        .unwrap_or(symbol)
}

pub(super) fn parse_side(side: &str) -> Option<PositionSide> {
    if side.eq_ignore_ascii_case("long") || side.eq_ignore_ascii_case("buy") {
        Some(PositionSide::Long)
    } else if side.eq_ignore_ascii_case("short") || side.eq_ignore_ascii_case("sell") {
        Some(PositionSide::Short)
    } else {
        None
    }
}

pub(super) fn market_price(position: &PositionInfo) -> f64 {
    if position.mark_price > 0.0 {
        position.mark_price
    } else {
        position.entry_price
    }
}

pub(super) fn margin_usd(position: &PositionInfo, quantity: f64, mark_price: f64) -> f64 {
    if position.margin > 0.0 {
        return position.margin;
    }
    if position.leverage > f64::EPSILON && mark_price > 0.0 {
        return quantity * mark_price / position.leverage;
    }
    0.0
}

pub(super) fn funding_field_quality(
    rows: &[PositionRow],
    observed_at_ms: i64,
) -> Vec<AccountFieldQuality> {
    rows.iter()
        .flat_map(|row| {
            let mut quality = Vec::with_capacity(2);
            if !row.funding_rate_verified {
                quality.push(unavailable_funding_field(
                    row,
                    "fundingRate8h",
                    observed_at_ms,
                ));
            }
            if row.next_funding_ms.is_none() {
                quality.push(unavailable_funding_field(
                    row,
                    "nextFundingMs",
                    observed_at_ms,
                ));
            }
            quality
        })
        .collect()
}

pub(super) fn field_quality_degraded(rows: &[AccountFieldQuality]) -> bool {
    account_quality::account_fields_degrade_snapshot(rows)
}

pub(super) fn unavailable_funding_field(
    row: &PositionRow,
    field: &'static str,
    observed_at_ms: i64,
) -> AccountFieldQuality {
    AccountFieldQuality::new(
        AccountFieldSubject::position(&row.venue, &row.symbol, position_side_key(row.side)),
        field,
        AccountFieldQualityStatus::Missing,
        PORTFOLIO_FUNDING_SOURCE,
        Some(observed_at_ms),
    )
    .with_problem(funding_field_problem(row, field, observed_at_ms))
}

pub(super) fn funding_field_problem(
    row: &PositionRow,
    field: &'static str,
    observed_at_ms: i64,
) -> ApiProblem {
    let mut problem = ApiProblem::new(
        shared_types::problem::codes::POSITION_FIELD_UNAVAILABLE,
        format!("position {field} is missing"),
    )
    .with_status(axum::http::StatusCode::OK.as_u16())
    .with_request_id(common::request_id::current())
    .with_source(PORTFOLIO_FUNDING_SOURCE);
    problem.details = Some(serde_json::json!({
        "venue": row.venue.as_str(),
        "symbol": row.symbol.as_str(),
        "side": position_side_key(row.side),
        "field": field,
        "status": AccountFieldQualityStatus::Missing,
        "operation": "funding_rates",
        "source": PORTFOLIO_FUNDING_SOURCE,
        "observedAtMs": observed_at_ms,
    }));
    problem
}
