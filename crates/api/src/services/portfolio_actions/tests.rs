use super::*;

use super::intent::{close_intent, close_order_key_for_context, close_order_plan};
use super::request::{select_pair_positions, select_position, validate_close_context};
use super::run::close_run;

mod context;
mod intent_build;
mod run_outcome;
mod selector;

fn close_context_for_test(expected_leg_count: usize) -> CloseRequestContext {
    CloseRequestContext {
        snapshot_version: "pos-test".to_owned(),
        expected_leg_count,
        reason: Some("positions.test".to_owned()),
        idempotency_key: None,
        scope: CloseRunScope::Single,
        execution: None,
    }
}

fn close_context_with_key(key: &str) -> CloseRequestContext {
    close_context_with_key_and_scope(key, CloseRunScope::Single)
}

fn close_context_with_key_and_scope(key: &str, scope: CloseRunScope) -> CloseRequestContext {
    CloseRequestContext {
        snapshot_version: "pos-test".to_owned(),
        expected_leg_count: 1,
        reason: Some("positions.test".to_owned()),
        idempotency_key: Some(key.to_owned()),
        scope,
        execution: None,
    }
}

fn close_leg_for_test(status: CloseLegStatus, notional_usd: f64) -> CloseLeg {
    CloseLeg {
        venue: "Binance".into(),
        symbol: "MUUSDT".into(),
        side: PositionSide::Long,
        status,
        quantity: 1.0,
        mark_price: notional_usd,
        notional_usd,
        order: None,
        finality_source: None,
        confirmed_filled_at_ms: None,
        problem: if matches!(status, CloseLegStatus::Failed) {
            Some(ApiProblem::new("TEST_FAILED", "failed"))
        } else {
            None
        },
        pair_evidence: None,
        cost_events: Vec::new(),
    }
}

fn position(side: PositionSide) -> PositionRow {
    PositionRow {
        venue: "Binance".into(),
        symbol: "MUUSDT".into(),
        origin: Default::default(),
        side,
        quantity: 2.0,
        entry_price: 660.0,
        mark_price: 664.25,
        leverage: 10.0,
        unrealized_pnl_usd: 8.5,
        liquidation_price: None,
        liquidation_distance_pct: None,
        next_funding_ms: None,
        funding_rate_8h: 0.0,
        funding_rate_verified: true,
        maintenance_margin_ratio: 0.0,
        pair_evidence: None,
        paired_with: None,
        margin_usd: 132.85,
        severity: Default::default(),
        seconds_until_funding: None,
    }
}

fn pair_evidence(row: &PositionRow, paired: &PositionRow) -> shared_types::PositionPairEvidence {
    shared_types::PositionPairEvidence {
        source: shared_types::PositionPairEvidenceSource::ExecutionRun,
        run_id: "run-1".into(),
        ticket_id: "ticket-1".into(),
        opportunity_id: "opp-1".into(),
        venue: row.venue.clone(),
        symbol: row.symbol.clone(),
        side: row.side,
        partner_venue: paired.venue.clone(),
        partner_symbol: paired.symbol.clone(),
        partner_side: paired.side,
        leg_filled_quantity: row.quantity,
        partner_filled_quantity: paired.quantity,
        matched_notional_usd: row.quantity.min(paired.quantity)
            * row.mark_price.min(paired.mark_price),
        updated_at_ms: 1,
    }
}
