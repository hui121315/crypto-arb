use shared_types::{
    normalized_venue_name, AccountDataHealth, AccountFieldSubjectKind, ExecutionEnvironment,
    ExecutionRun, ExecutionRunLeg, ExecutionRunState, HedgeLegRole, ListStatus, PortfolioSnapshot,
    PositionPairEvidence, PositionRow, PositionSide,
};

use super::MAX_SNAPSHOT_AGE_MS;

pub(super) fn eligible_execution_run(
    run: &ExecutionRun,
    evidence: &PositionPairEvidence,
    anchor: &PositionRow,
    partner: &PositionRow,
) -> Option<()> {
    (run.run_id == evidence.run_id
        && run.ticket_id == evidence.ticket_id
        && run.opportunity_id == evidence.opportunity_id
        && run.state == ExecutionRunState::Hedged
        && run.unwind_problem.is_none()
        && run.finality_problem.is_none()
        && run_leg_matches(&run.long_leg, PositionSide::Long, anchor, partner)
        && run_leg_matches(&run.short_leg, PositionSide::Short, anchor, partner))
    .then_some(())
}

fn run_leg_matches(
    leg: &ExecutionRunLeg,
    side: PositionSide,
    anchor: &PositionRow,
    partner: &PositionRow,
) -> bool {
    let expected_role = match side {
        PositionSide::Long => HedgeLegRole::Long,
        PositionSide::Short => HedgeLegRole::Short,
    };
    leg.role == expected_role
        && [anchor, partner].into_iter().any(|row| {
            row.side == side
                && shared_types::venue_names_equal(&leg.exchange, &row.venue)
                && leg.symbol.eq_ignore_ascii_case(&row.symbol)
        })
}

pub(super) fn pair_position_evidence_observed_at_ms(
    snapshot: &PortfolioSnapshot,
    environment: ExecutionEnvironment,
    anchor: &PositionRow,
    partner: &PositionRow,
) -> Option<i64> {
    if environment == ExecutionEnvironment::Paper {
        return Some(snapshot.server_now_ms);
    }
    let row_health = &snapshot.account_state.positions.row_health;
    if row_health.is_empty() && snapshot.account_state.positions.status == ListStatus::Fresh {
        return fresh_evidence_time(
            snapshot.server_now_ms,
            snapshot.account_state.positions.observed_at_ms,
        );
    }
    let anchor_ms = position_row_evidence_observed_at_ms(snapshot, anchor)?;
    let partner_ms = position_row_evidence_observed_at_ms(snapshot, partner)?;
    Some(anchor_ms.min(partner_ms))
}

fn position_row_evidence_observed_at_ms(
    snapshot: &PortfolioSnapshot,
    row: &PositionRow,
) -> Option<i64> {
    snapshot
        .account_state
        .positions
        .row_health
        .iter()
        .filter(|health| account_health_matches_position(health, row))
        .filter_map(account_health_success_ms)
        .filter_map(|success_ms| fresh_evidence_time(snapshot.server_now_ms, success_ms))
        .max()
}

fn fresh_evidence_time(server_now_ms: i64, observed_at_ms: i64) -> Option<i64> {
    server_now_ms
        .checked_sub(observed_at_ms)
        .filter(|age_ms| (0..=MAX_SNAPSHOT_AGE_MS).contains(age_ms))
        .map(|_| observed_at_ms)
}

fn account_health_matches_position(health: &AccountDataHealth, row: &PositionRow) -> bool {
    health.subject.kind == AccountFieldSubjectKind::Position
        && health
            .subject
            .venue
            .as_deref()
            .is_some_and(|venue| normalized_venue_name(venue) == normalized_venue_name(&row.venue))
        && health
            .subject
            .symbol
            .as_deref()
            .is_some_and(|symbol| symbol.eq_ignore_ascii_case(&row.symbol))
        && health
            .subject
            .side
            .as_deref()
            .is_some_and(|side| side.eq_ignore_ascii_case(side_key(row.side)))
}

fn account_health_success_ms(health: &AccountDataHealth) -> Option<i64> {
    health.last_success_ms.or_else(|| {
        health.last_error.is_none().then_some(())?;
        let freshness_ms = health.freshness_ms?;
        (freshness_ms >= 0).then(|| health.observed_at_ms.saturating_sub(freshness_ms))
    })
}

pub(super) fn unique_partner<'a>(
    rows: &'a [PositionRow],
    anchor: &PositionRow,
) -> Option<&'a PositionRow> {
    let evidence = anchor.pair_evidence.as_ref()?;
    let mut matches = rows
        .iter()
        .filter(|row| row_matches_partner(row, evidence) && reciprocal_pair(anchor, row, evidence));
    let partner = matches.next()?;
    matches.next().is_none().then_some(partner)
}

fn row_matches_partner(row: &PositionRow, evidence: &PositionPairEvidence) -> bool {
    shared_types::venue_names_equal(&row.venue, &evidence.partner_venue)
        && row.symbol.eq_ignore_ascii_case(&evidence.partner_symbol)
        && row.side == evidence.partner_side
}

fn reciprocal_pair(
    anchor: &PositionRow,
    partner: &PositionRow,
    anchor_evidence: &PositionPairEvidence,
) -> bool {
    partner.pair_evidence.as_ref().is_some_and(|evidence| {
        evidence.run_id == anchor_evidence.run_id
            && evidence.ticket_id == anchor_evidence.ticket_id
            && evidence.opportunity_id == anchor_evidence.opportunity_id
            && evidence_matches_row(partner, evidence)
            && shared_types::venue_names_equal(&evidence.partner_venue, &anchor.venue)
            && evidence.partner_symbol.eq_ignore_ascii_case(&anchor.symbol)
            && evidence.partner_side == anchor.side
    })
}

pub(super) fn is_canonical_anchor(row: &PositionRow, evidence: &PositionPairEvidence) -> bool {
    evidence_matches_row(row, evidence)
        && leg_key(&row.venue, &row.symbol, row.side)
            <= leg_key(
                &evidence.partner_venue,
                &evidence.partner_symbol,
                evidence.partner_side,
            )
}

fn evidence_matches_row(row: &PositionRow, evidence: &PositionPairEvidence) -> bool {
    shared_types::venue_names_equal(&row.venue, &evidence.venue)
        && row.symbol.eq_ignore_ascii_case(&evidence.symbol)
        && row.side == evidence.side
}

fn leg_key(venue: &str, symbol: &str, side: PositionSide) -> (String, String, u8) {
    (
        normalized_venue_name(venue),
        symbol.trim().to_ascii_uppercase(),
        match side {
            PositionSide::Long => 0,
            PositionSide::Short => 1,
        },
    )
}

const fn side_key(side: PositionSide) -> &'static str {
    match side {
        PositionSide::Long => "long",
        PositionSide::Short => "short",
    }
}
