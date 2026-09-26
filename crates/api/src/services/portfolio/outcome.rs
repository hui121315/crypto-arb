use super::*;

pub(crate) fn close_snapshot_version(state: &AppState, rows: &[PositionRow]) -> String {
    let service = state.trading_service();
    format!(
        "{}:{}:{}:{}",
        positions_version(rows),
        service.adapter_name(),
        service.risk_config().live_trading_enabled,
        service.account_cache_epoch(),
    )
}

pub(crate) fn positions_version(rows: &[PositionRow]) -> String {
    let mut keys = rows.iter().map(position_version_key).collect::<Vec<_>>();
    keys.sort_unstable();
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for key in &keys {
        update_fnv64(&mut hash, key.as_bytes());
        update_fnv64(&mut hash, &[0xff]);
    }
    format!("pos-{}-{hash:016x}", rows.len())
}

pub(super) fn operation_health_degraded(rows: &[VenueOperationHealth]) -> bool {
    rows.iter()
        .any(account_quality::account_data_operation_degrades_snapshot)
}

pub(super) fn position_version_key(row: &PositionRow) -> String {
    format!(
        "{}|{}|{}|{}|{}|{:?}",
        normalized_venue_name(&row.venue),
        row.symbol.to_ascii_uppercase(),
        position_side_key(row.side),
        stable_number(row.quantity),
        pair_version_key(row),
        row.origin,
    )
}

pub(super) fn pair_version_key(row: &PositionRow) -> String {
    let Some(pair) = row.pair_evidence.as_ref() else {
        return String::new();
    };
    format!(
        "{}|{}|{}|{}",
        pair.run_id,
        normalized_venue_name(&pair.partner_venue),
        pair.partner_symbol.to_ascii_uppercase(),
        position_side_key(pair.partner_side)
    )
}

pub(super) fn position_side_key(side: PositionSide) -> &'static str {
    match side {
        PositionSide::Long => "long",
        PositionSide::Short => "short",
    }
}

pub(super) fn stable_number(value: f64) -> String {
    if value.is_finite() {
        format!("{value:.8}")
    } else {
        "nan".to_owned()
    }
}

pub(super) fn update_fnv64(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
}

pub(crate) struct PositionsOutcome {
    pub(crate) rows: Vec<PositionRow>,
    pub(crate) problems: Vec<RuntimeProblem>,
    pub(crate) field_quality: Vec<AccountFieldQuality>,
}

pub(super) fn funding_rows(state: &AppState) -> Vec<FundingRateData> {
    state.market_data().funding_rows_snapshot()
}

pub(super) fn execution_pair_evidence(state: &AppState) -> Vec<PositionPairEvidence> {
    let environment = execution_environment::active(state);
    let mut rows = state
        .execution_runs()
        .iter()
        .filter(|entry| entry.value().state == ExecutionRunState::Hedged)
        .filter(|entry| execution_environment::run_matches(state, entry.value(), environment))
        .filter_map(|entry| execution_run_pair_evidence(entry.value()))
        .flat_map(IntoIterator::into_iter)
        .collect::<Vec<_>>();
    rows.sort_by_key(|row| Reverse(row.updated_at_ms));
    rows
}

pub(super) fn recent_close_runs(state: &AppState) -> Vec<CloseRun> {
    let rows = state
        .close_runs()
        .iter()
        .map(|entry| entry.value().clone())
        .collect::<Vec<_>>();
    recent_close_runs_from_rows(rows)
}

pub(super) fn recent_close_runs_from_rows(mut rows: Vec<CloseRun>) -> Vec<CloseRun> {
    rows.sort_by_key(|run| Reverse(run.updated_at_ms));
    rows.truncate(RECENT_CLOSE_RUN_LIMIT);
    rows
}

pub(super) fn execution_run_pair_evidence(run: &ExecutionRun) -> Option<[PositionPairEvidence; 2]> {
    if run.state != ExecutionRunState::Hedged {
        return None;
    }
    if run.long_leg.role != HedgeLegRole::Long || run.short_leg.role != HedgeLegRole::Short {
        return None;
    }
    let long_quantity = confirmed_leg_quantity(&run.long_leg)?;
    let short_quantity = confirmed_leg_quantity(&run.short_leg)?;
    let long_notional = confirmed_leg_notional(&run.long_leg)?;
    let short_notional = confirmed_leg_notional(&run.short_leg)?;
    let matched_notional_usd = long_notional.min(short_notional);
    Some([
        pair_evidence_row(
            run,
            EvidenceLeg::new(&run.long_leg, PositionSide::Long, long_quantity),
            EvidenceLeg::new(&run.short_leg, PositionSide::Short, short_quantity),
            matched_notional_usd,
        ),
        pair_evidence_row(
            run,
            EvidenceLeg::new(&run.short_leg, PositionSide::Short, short_quantity),
            EvidenceLeg::new(&run.long_leg, PositionSide::Long, long_quantity),
            matched_notional_usd,
        ),
    ])
}

#[derive(Clone, Copy)]
pub(super) struct EvidenceLeg<'a> {
    pub(super) leg: &'a ExecutionRunLeg,
    pub(super) side: PositionSide,
    pub(super) quantity: f64,
}

impl<'a> EvidenceLeg<'a> {
    pub(super) fn new(leg: &'a ExecutionRunLeg, side: PositionSide, quantity: f64) -> Self {
        Self {
            leg,
            side,
            quantity,
        }
    }
}

pub(super) fn confirmed_leg_quantity(leg: &ExecutionRunLeg) -> Option<f64> {
    if leg.state != LiveOrderState::Filled || leg.confirmed_filled_at_ms.is_none() {
        return None;
    }
    valid_positive(leg.filled_quantity?)
}

pub(super) fn confirmed_leg_notional(leg: &ExecutionRunLeg) -> Option<f64> {
    valid_positive(leg.filled_notional_usd?)
}

pub(super) fn pair_evidence_row(
    run: &ExecutionRun,
    leg: EvidenceLeg<'_>,
    partner: EvidenceLeg<'_>,
    matched_notional_usd: f64,
) -> PositionPairEvidence {
    PositionPairEvidence {
        source: PositionPairEvidenceSource::ExecutionRun,
        run_id: run.run_id.clone(),
        ticket_id: run.ticket_id.clone(),
        opportunity_id: run.opportunity_id.clone(),
        venue: leg.leg.exchange.clone(),
        symbol: exchange::strip_common_suffixes(&leg.leg.symbol),
        side: leg.side,
        partner_venue: partner.leg.exchange.clone(),
        partner_symbol: exchange::strip_common_suffixes(&partner.leg.symbol),
        partner_side: partner.side,
        leg_filled_quantity: leg.quantity,
        partner_filled_quantity: partner.quantity,
        matched_notional_usd,
        updated_at_ms: run.updated_at_ms,
    }
}
