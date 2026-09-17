use super::super::*;
use super::{assert_single_account_dirty, fixtures_b::*};
use shared_types::{LiveOrderState, OrderStatus};

#[test]
fn kucoin_wallet_balance_change_maps_to_balance_patch() {
    let events = map_kucoin_event(kucoin_ws_user::KucoinUserEvent::Balance(
        kucoin_balance_delta("walletBalance.change", 100.0, 90.0),
    ));

    let patch = events.iter().find_map(|event| match event {
        PrivateWsEvent::BalancePatch(patch) => Some(patch.as_ref()),
        _ => None,
    });
    assert_eq!(patch.map(|patch| patch.venue.as_str()), Some("kucoin"));
    assert_eq!(patch.map(|patch| patch.rows[0].total), Some(100.0));
    assert_eq!(patch.map(|patch| patch.rows[0].frozen), Some(10.0));
}

#[test]
fn kucoin_partial_balance_subject_marks_dirty() {
    let events = map_kucoin_event(kucoin_ws_user::KucoinUserEvent::Balance(
        kucoin_balance_delta("availableBalance.change", 100.0, 90.0),
    ));
    assert_single_account_dirty(&events);
}

#[test]
fn kucoin_cross_position_change_requires_metadata_backed_refresh() {
    let events = map_kucoin_event(kucoin_ws_user::KucoinUserEvent::Position(
        kucoin_position_delta("position.change", "CROSS", true, 1.0),
    ));
    assert_single_account_dirty(&events);
}

#[test]
fn kucoin_closed_position_change_marks_dirty() {
    let events = map_kucoin_event(kucoin_ws_user::KucoinUserEvent::Position(
        kucoin_position_delta("position.change", "CROSS", false, 0.0),
    ));
    assert_single_account_dirty(&events);
}

#[test]
fn kucoin_funding_settlement_invalidates_balances_and_positions() {
    let events = map_kucoin_event(kucoin_ws_user::KucoinUserEvent::Position(
        kucoin_ws_user::KucoinPositionDelta::Settlement {
            native_symbol: Some("XBTUSDTM".to_owned()),
            current_contracts: -2.0,
            updated_time_ms: 1_771_488_018_495,
        },
    ));

    assert!(matches!(
        events.as_slice(),
        [PrivateWsEvent::AccountDirty(dirty)]
            if dirty.scope == PrivateAccountScope::All
                && dirty.reason == "funding_settlement_requires_account_refresh"
    ));
}

#[test]
fn kucoin_failed_risk_limit_notice_does_not_invalidate_account_cache() {
    let events = map_kucoin_event(kucoin_ws_user::KucoinUserEvent::Position(
        kucoin_ws_user::KucoinPositionDelta::RiskLimitAdjustment { success: false },
    ));

    assert!(events.is_empty());
}

#[test]
fn kucoin_nonterminal_order_update_requires_authoritative_query() {
    let events = map_kucoin_event(kucoin_ws_user::KucoinUserEvent::Order(Box::new(
        kucoin_ws_user::KucoinOrderUpdate {
            client_order_id: "kucoin-c1".to_owned(),
            event_type: "match".to_owned(),
            event_time_ms: 42,
            terminal: false,
            live_state: LiveOrderState::PartiallyFilled,
            order: order_info("kucoin", OrderStatus::PartiallyFilled),
            fill: None,
        },
    )));

    assert_single_account_dirty(&events);
}

#[test]
fn kucoin_classic_match_does_not_project_contract_counts_as_base_quantity() {
    let event = parse_kucoin_event(include_str!(
        "../../../../../exchange/fixtures/kucoin/classic_ws_trade_orders_match.json"
    ));
    let events = map_kucoin_event(event);

    assert_single_account_dirty(&events);
    assert!(!events
        .iter()
        .any(|event| matches!(event, PrivateWsEvent::Fill(_))));
}

#[test]
fn kucoin_classic_terminal_fixture_maps_order_without_duplicate_fill() {
    let event = parse_kucoin_event(include_str!(
        "../../../../../exchange/fixtures/kucoin/classic_ws_trade_orders_filled.json"
    ));
    let events = map_kucoin_event(event);

    assert!(events.iter().any(|event| matches!(
        event, PrivateWsEvent::Order(order)
            if order.order.status == OrderStatus::Filled
                && order.order.quantity == 0.0
                && order.order.filled_quantity == 0.0
    )));
    assert!(!events
        .iter()
        .any(|event| matches!(event, PrivateWsEvent::Fill(_))));
}

#[allow(clippy::expect_used)]
fn parse_kucoin_event(body: &str) -> kucoin_ws_user::KucoinUserEvent {
    kucoin_ws_user::parse_user_event(body)
        .expect("official KuCoin fixture parses")
        .expect("known KuCoin order event")
}
