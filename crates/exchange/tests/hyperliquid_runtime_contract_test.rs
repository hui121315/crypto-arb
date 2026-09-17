#![allow(clippy::expect_used, clippy::panic)]

use exchange::adapters::hyperliquid_ws_user::{
    parse_user_event, HyperliquidLiquidationMethod, HyperliquidUserWsEvent,
};

const LIQUIDATION_FILL: &str =
    include_str!("../fixtures/hyperliquid/ws_user_events_liquidation_fill.json");

#[test]
fn official_user_fill_fixture_preserves_closed_pnl_and_liquidation_evidence() {
    let event = parse_user_event(LIQUIDATION_FILL)
        .expect("official user fill fixture")
        .expect("recognized user fill channel");
    let HyperliquidUserWsEvent::Fill(fills) = event else {
        panic!("expected user fill event");
    };

    assert_eq!(fills.len(), 1);
    let fill = &fills[0];
    assert_eq!(fill.venue, "hyperliquid");
    assert_eq!(fill.order_id, "11223344");
    assert_eq!(fill.closed_pnl, -12.375);
    let liquidation = fill.liquidation.as_ref().expect("liquidation evidence");
    assert_eq!(
        liquidation.liquidated_user.as_deref(),
        Some("0x2222222222222222222222222222222222222222")
    );
    assert_eq!(liquidation.mark_price, 101_200.25);
    assert_eq!(liquidation.method, HyperliquidLiquidationMethod::Backstop);
}

#[test]
fn user_fill_liquidation_rejects_unknown_official_method() {
    let fixture = LIQUIDATION_FILL.replace("backstop", "socialized");
    let error = parse_user_event(&fixture).expect_err("unknown method must fail closed");

    assert!(error
        .to_string()
        .contains("unsupported fill liquidation method: socialized"));
}
