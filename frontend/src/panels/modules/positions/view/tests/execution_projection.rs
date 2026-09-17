use super::*;
use shared_types::{PositionOrigin, PositionPairEvidence, PositionPairEvidenceSource};

#[test]
fn execution_projection_is_fresh_when_private_positions_are_unconfigured() {
    let problem = ApiProblem::new(
        "POSITION_EVIDENCE_MISSING",
        "private position rows are unavailable",
    );
    let mut snapshot = snapshot_with_position_status(ListStatus::Degraded, vec![problem]);
    snapshot.positions[0].origin = PositionOrigin::ExecutionLedger;
    snapshot.positions[0].pair_evidence = Some(PositionPairEvidence {
        source: PositionPairEvidenceSource::ExecutionRun,
        run_id: "run-1".into(),
        ticket_id: "ticket-1".into(),
        opportunity_id: "opp-1".into(),
        venue: "Gate".into(),
        symbol: "BTC".into(),
        side: PositionSide::Long,
        partner_venue: "OKX".into(),
        partner_symbol: "BTC".into(),
        partner_side: PositionSide::Short,
        leg_filled_quantity: 1.0,
        partner_filled_quantity: 1.0,
        matched_notional_usd: 100.0,
        updated_at_ms: 42,
    });

    let section = position_snapshot_section(&LoadState::Ready(snapshot.clone()));

    assert!(section.has_fresh_value());
    assert_eq!(section.value.len(), 1);
    assert!(position_surface_evidence(&LoadState::Ready(snapshot)).is_none());
}
