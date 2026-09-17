use super::super::*;
use super::*;

#[test]
fn current_liquidation_distance_is_unknown_when_positions_have_no_verified_distance() {
    assert_eq!(current_liquidation_distance_pct(&[]), None);
    assert_eq!(
        current_liquidation_distance_pct(&[position_with_liq_distance(None)]),
        None
    );
}

#[test]
fn current_liquidation_distance_uses_min_valid_position_distance() {
    let rows = vec![
        position_with_liq_distance(Some(25.0)),
        position_with_liq_distance(Some(f64::NAN)),
        position_with_liq_distance(Some(12.5)),
    ];

    assert_eq!(current_liquidation_distance_pct(&rows), Some(12.5));
}

#[test]
fn live_preview_projects_positions_problem_into_blocking_evidence() {
    let envelope = VenuePositionEnvelope::new(
        Vec::new(),
        ListStatus::Degraded,
        "account_position_runtime",
        42,
        vec![shared_types::ApiProblem::new(
            shared_types::problem::codes::POSITION_EVIDENCE_MISSING,
            "positions failed",
        )
        .with_request_id(Some("req-position".into()))],
        Vec::new(),
    );

    let metrics = preview_metrics_from_position_envelope(&envelope, 0.0, &PreviewCosts::default());
    let guard = positions_evidence_guard(
        ExecutionMode::Live,
        &metrics.positions_evidence,
        &metrics.position_rows,
        &[],
    );

    assert_eq!(metrics.used_capital_usd, 0.0);
    assert!(metrics.current_account_liq_distance_pct.is_none());
    assert_eq!(
        metrics.positions_evidence.request_id.as_deref(),
        Some("req-position")
    );
    assert!(!guard.passed);
    assert!(guard.detail.contains("持仓/强平证据阻断"));
}

#[test]
fn dry_run_preview_does_not_require_private_position_evidence() {
    let envelope = VenuePositionEnvelope::new(
        Vec::new(),
        ListStatus::Degraded,
        "account_position_runtime",
        42,
        vec![shared_types::ApiProblem::new(
            shared_types::problem::codes::POSITION_EVIDENCE_MISSING,
            "positions failed",
        )],
        Vec::new(),
    );
    let metrics = preview_metrics_from_position_envelope(&envelope, 0.0, &PreviewCosts::default());

    let guard = positions_evidence_guard(
        ExecutionMode::DryRun,
        &metrics.positions_evidence,
        &metrics.position_rows,
        &[],
    );

    assert!(guard.passed, "{}", guard.detail);
    assert_eq!(guard.detail, "模拟模式：不要求交易所私有持仓/强平证据");
    assert!(
        guard.preflight_outcome.is_some(),
        "paper positions outcome remains auditable"
    );
    let outcome = guard.preflight_outcome.unwrap_or_default();
    assert_eq!(outcome.status, shared_types::HedgePreflightStatus::Passed);
    assert!(outcome.scope.venues.is_empty());
    assert!(outcome.observed_venues.is_empty());
    assert!(outcome.problems.is_empty());
}

#[test]
fn kucoin_current_position_matching_ticket_passes() {
    let intent = kucoin_live_intent(2.0, shared_types::MarginMode::Cross);
    let position = kucoin_position(2.0, Some("cross"));
    let (evidence, rows) = kucoin_position_evidence(vec![position]);

    let guard = positions_evidence_guard(ExecutionMode::Live, &evidence, &rows, &[&intent]);

    assert!(guard.passed, "{}", guard.detail);
}

#[test]
fn kucoin_current_position_conflict_blocks_ticket() {
    let intent = kucoin_live_intent(3.0, shared_types::MarginMode::Isolated);
    let position = kucoin_position(2.0, Some("cross"));
    let (evidence, rows) = kucoin_position_evidence(vec![position]);

    let guard = positions_evidence_guard(ExecutionMode::Live, &evidence, &rows, &[&intent]);

    assert!(!guard.passed);
    assert!(guard.detail.contains("leverage=2"));
    assert!(guard.detail.contains("marginMode=cross"));
}

#[test]
fn kucoin_current_position_missing_margin_mode_fails_visibly() {
    let intent = kucoin_live_intent(2.0, shared_types::MarginMode::Cross);
    let position = kucoin_position(2.0, None);
    let (evidence, rows) = kucoin_position_evidence(vec![position]);

    let guard = positions_evidence_guard(ExecutionMode::Live, &evidence, &rows, &[&intent]);

    assert!(!guard.passed);
    assert!(guard.detail.contains("缺少 marginMode 证据"));
}

#[test]
fn kucoin_opening_without_current_position_stays_allowed() {
    let intent = kucoin_live_intent(3.0, shared_types::MarginMode::Isolated);
    let (evidence, rows) = kucoin_position_evidence(Vec::new());

    let guard = positions_evidence_guard(ExecutionMode::Live, &evidence, &rows, &[&intent]);

    assert!(guard.passed, "{}", guard.detail);
}

fn kucoin_position_evidence(
    rows: Vec<PositionInfo>,
) -> (
    shared_types::HedgePreviewPositionsEvidence,
    Vec<PositionInfo>,
) {
    let envelope = VenuePositionEnvelope::new(
        rows,
        ListStatus::Fresh,
        "account_position_runtime",
        42,
        Vec::new(),
        Vec::new(),
    );
    let metrics = preview_metrics_from_position_envelope(&envelope, 0.0, &PreviewCosts::default());
    (metrics.positions_evidence, metrics.position_rows)
}

fn kucoin_position(leverage: f64, margin_mode: Option<&str>) -> PositionInfo {
    let mut position = position_with_liq_distance(Some(25.0));
    position.symbol = "BTC".to_owned();
    position.exchange = "kucoin".to_owned();
    position.leverage = leverage;
    position.margin_mode = margin_mode.map(str::to_owned);
    position
}

fn kucoin_live_intent(
    leverage: f64,
    margin_mode: shared_types::MarginMode,
) -> shared_types::OrderIntent {
    let opp = blocked_opportunity();
    let params = market_params();
    let mut intent = build_leg(market_leg_build(&opp, &params));
    intent.mode = ExecutionMode::Live;
    intent.exchange = "kucoin".to_owned();
    intent.symbol = "BTC".to_owned();
    intent.leverage = leverage;
    intent.margin_mode = margin_mode;
    intent
}
