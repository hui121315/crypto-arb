use super::*;

#[path = "build/dry_run.rs"]
mod dry_run;
use dry_run::rows_from_dry_run_orders;

#[cfg(test)]
pub(super) fn rows_from_positions(
    positions: Vec<PositionInfo>,
    rates: &[FundingRateData],
    now_ms: i64,
    warn_pct: f64,
    danger_pct: f64,
) -> Vec<PositionRow> {
    rows_from_sources(
        positions,
        &[],
        rates,
        false,
        &[],
        RiskAnnotation {
            now_ms,
            warn_pct,
            danger_pct,
        },
    )
}

#[cfg(test)]
pub(super) fn rows_from_sources(
    positions: Vec<PositionInfo>,
    orders: &[OrderRecord],
    rates: &[FundingRateData],
    include_dry_run: bool,
    pair_evidence: &[PositionPairEvidence],
    annotation: RiskAnnotation,
) -> Vec<PositionRow> {
    rows_from_sources_with_dry_run_marks(
        positions,
        orders,
        rates,
        pair_evidence,
        DryRunRows {
            enabled: include_dry_run,
            marks: &HashMap::new(),
        },
        annotation,
    )
}

#[derive(Debug, Clone, Copy)]
pub(super) struct DryRunRows<'a> {
    pub(super) enabled: bool,
    pub(super) marks: &'a HashMap<(String, String), f64>,
}

pub(super) fn rows_from_sources_with_dry_run_marks(
    positions: Vec<PositionInfo>,
    orders: &[OrderRecord],
    rates: &[FundingRateData],
    pair_evidence: &[PositionPairEvidence],
    dry_run: DryRunRows<'_>,
    annotation: RiskAnnotation,
) -> Vec<PositionRow> {
    let funding = funding_evidence_index(rates, annotation.now_ms);
    let mut rows: Vec<PositionRow> = positions
        .into_iter()
        .filter_map(|position| row_from_position(position, &funding, annotation.now_ms))
        .collect();
    if dry_run.enabled {
        rows.extend(rows_from_dry_run_orders(orders, &funding, dry_run.marks));
    }
    finalize_rows(rows, pair_evidence, annotation)
}

pub(super) fn finalize_rows(
    mut rows: Vec<PositionRow>,
    pair_evidence: &[PositionPairEvidence],
    annotation: RiskAnnotation,
) -> Vec<PositionRow> {
    pair_positions(&mut rows, pair_evidence);
    annotate_liquidation_distance(&mut rows);
    annotate_row_risk(&mut rows, annotation);
    rows
}

#[derive(Debug, Clone, Copy)]
pub(super) struct RiskAnnotation {
    pub(super) now_ms: i64,
    pub(super) warn_pct: f64,
    pub(super) danger_pct: f64,
}

pub(super) fn row_from_position(
    position: PositionInfo,
    funding: &PositionFundingIndex,
    now_ms: i64,
) -> Option<PositionRow> {
    let side = parse_side(&position.side)?;
    let quantity = position.quantity.abs();
    if quantity <= f64::EPSILON {
        return None;
    }
    let mark_price = market_price(&position);
    let margin_usd = margin_usd(&position, quantity, mark_price);
    let funding_rate_8h = funding_rate_for(&position, funding);
    let funding_rate_verified = funding_rate_8h.is_some();
    let next_funding_ms = next_funding_for_position(&position, funding, now_ms);
    let maintenance_margin_ratio = maintenance_margin_ratio(&position);
    let liquidation_price = position
        .liquidation_price
        .filter(|value| value.is_finite() && *value > 0.0);

    Some(PositionRow {
        venue: position.exchange,
        symbol: position.symbol,
        origin: PositionOrigin::AccountPrivate,
        side,
        quantity,
        entry_price: position.entry_price,
        mark_price,
        leverage: position.leverage,
        unrealized_pnl_usd: position.unrealized_pnl,
        // A venue-confirmed zero remains in account field quality, while the
        // workstation mirrors the venue UI and renders the numeric value as `--`.
        liquidation_price,
        liquidation_distance_pct: position.liquidation_distance_pct,
        next_funding_ms,
        funding_rate_8h: funding_rate_8h.unwrap_or(0.0),
        funding_rate_verified,
        maintenance_margin_ratio,
        pair_evidence: None,
        paired_with: None,
        margin_usd,
        severity: PositionSeverity::Ok,
        seconds_until_funding: None,
    })
}

pub(super) fn valid_positive(value: f64) -> Option<f64> {
    (value.is_finite() && value > f64::EPSILON).then_some(value)
}
