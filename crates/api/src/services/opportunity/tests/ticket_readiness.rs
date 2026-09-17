use super::*;

#[test]
fn positive_spot_perp_can_build_a_ticket_without_becoming_execution_ready() {
    let mut row = dto("spot-perp", StrategyKind::SpotPerp, true);
    row.spot_leg_mode = Some(shared_types::SpotLegMode::BuySpot);
    row.execution_eligible = false;
    row.execution_blockers = vec![shared_types::DEFERRED_SPOT_PERP_TICKET_BLOCKER.to_owned()];

    assert!(!shared_types::is_hedge_preview_ready(&row));
    assert!(is_ticket_build_ready(&row));

    row.execution_blockers
        .push("交易所标的身份未通过".to_owned());
    assert!(!is_ticket_build_ready(&row));
}

#[test]
fn candidate_funding_warmup_stays_off_the_product_list() {
    let mut row = dto("perp-cross", StrategyKind::PerpCross, true);
    let now_ms = chrono::Utc::now().timestamp_millis();

    assert!(has_fresh_ws_market_pair(&row, now_ms));

    row.execution_eligible = false;
    row.execution_blockers = vec![FUNDING_WS_EVIDENCE_BLOCKER.to_owned()];
    assert!(!has_fresh_ws_market_pair(&row, now_ms));
}

#[test]
fn product_list_requires_verified_fees_and_allows_only_deferred_blockers() {
    let mut row = dto("perp-cross", StrategyKind::PerpCross, true);
    let now_ms = chrono::Utc::now().timestamp_millis();

    assert!(is_product_visible_row(&row, now_ms));

    if let Some(round_trip) = row
        .execution_cost
        .as_mut()
        .and_then(|cost| cost.round_trip.as_mut())
    {
        round_trip.long_leg.fee_snapshot = None;
    }
    assert!(!is_product_visible_row(&row, now_ms));

    make_ready(&mut row);
    let refreshed_now_ms = chrono::Utc::now().timestamp_millis();
    row.execution_eligible = false;
    row.execution_blockers = vec!["交易所标的身份未通过".to_owned()];
    assert!(!is_product_visible_row(&row, refreshed_now_ms));

    row.execution_blockers = vec![shared_types::DEFERRED_INVENTORY_OR_BORROW_BLOCKER.to_owned()];
    assert!(is_product_visible_row(&row, refreshed_now_ms));
}
