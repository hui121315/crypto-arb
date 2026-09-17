use super::*;

#[test]
fn ready_preview_without_ticket_identity_cannot_submit() {
    let mut response = preview_response();
    response.ticket_order_plans = Some(hyperliquid_ticket_order_plans());
    let mut preview = from_api_preview(
        response,
        &PreviewSeed::from_selection(&ExecutionSelection::empty()),
        &preview_input(),
    );
    assert!(preview.can_submit());

    preview.ticket_id = None;
    assert!(!preview.can_submit());
    preview.ticket_id = Some("  ".to_owned());
    assert!(!preview.can_submit());
}

#[test]
fn ready_preview_fails_closed_near_ticket_expiry() {
    let mut response = preview_response();
    response.ticket_order_plans = Some(hyperliquid_ticket_order_plans());
    response.ticket.expires_at_ms = 60_000;
    let preview = from_api_preview(
        response,
        &PreviewSeed::from_selection(&ExecutionSelection::empty()),
        &preview_input(),
    );

    assert!(preview.can_submit_at(57_999));
    assert!(!preview.can_submit_at(58_000));
    assert!(preview.ticket_needs_refresh_at(58_000));
}

#[test]
fn retryable_market_blocker_is_distinct_from_permanent_execution_blocker() {
    let mut response = preview_response();
    response.ticket_order_plans = Some(hyperliquid_ticket_order_plans());
    response
        .ticket
        .blockers
        .push("bybit BTC orderbook 超过 30s 未更新".into());
    let mut preview = from_api_preview(
        response,
        &PreviewSeed::from_selection(&ExecutionSelection::empty()),
        &preview_input(),
    );

    assert!(preview.has_retryable_market_blocker());

    preview.risk.blockers = vec!["交易所未通过实盘路由校验".into()];
    assert!(!preview.has_retryable_market_blocker());
}
