use super::super::*;
use super::{assert_single_account_dirty, fixtures_a::*, fixtures_b::*};
use shared_types::{LiveOrderState, OrderStatus};

#[test]
fn account_delta_marks_account_dirty() {
    let events = map_gate_event(gate_ws_user::GateUserEvent::Balance(Vec::new()));

    assert_single_account_dirty(&events);
}

#[test]
fn gate_positive_position_delta_requires_metadata_backed_refresh() {
    let events = map_gate_event(gate_ws_user::GateUserEvent::Position(vec![
        gate_position_delta("BTC", "long", 2.0),
    ]));
    assert_single_account_dirty(&events);
    assert!(!events
        .iter()
        .any(|event| matches!(event, PrivateWsEvent::PositionPatch(_))));
}

#[test]
fn gate_zero_position_delta_marks_dirty() {
    let events = map_gate_event(gate_ws_user::GateUserEvent::Position(vec![
        gate_position_delta("BTC", "long", 0.0),
    ]));
    assert_single_account_dirty(&events);
}

#[test]
fn gate_empty_position_update_is_ignored() {
    let events = map_gate_event(gate_ws_user::GateUserEvent::Position(Vec::new()));
    assert!(events.is_empty());
}

#[test]
fn gate_usertrade_requires_metadata_backed_order_query() {
    let events = map_gate_event(gate_ws_user::GateUserEvent::UserTrade(vec![
        gate_usertrade_delta("3335259", "4872460"),
    ]));

    assert_single_account_dirty(&events);
    assert!(!events
        .iter()
        .any(|event| matches!(event, PrivateWsEvent::Fill(_))));
}

#[test]
fn gate_terminal_orders_map_finality_by_exchange_id() {
    for (status, live_state) in [
        (OrderStatus::Filled, LiveOrderState::Filled),
        (OrderStatus::Canceled, LiveOrderState::Cancelled),
    ] {
        let mut order = order_info("gate", status);
        order.order_id = "4872460".to_owned();
        order.client_order_id = None;
        let events = map_gate_event(gate_ws_user::GateUserEvent::Order(vec![
            gate_ws_user::GateOrderUpdate {
                client_order_id: String::new(),
                live_state,
                order,
            },
        ]));

        let delta = events.iter().find_map(|event| match event {
            PrivateWsEvent::Order(delta) => Some(delta),
            _ => None,
        });
        assert_eq!(delta.map(|delta| delta.client_order_id.as_str()), Some(""));
        assert_eq!(
            delta.map(|delta| delta.order.order_id.as_str()),
            Some("4872460")
        );
        assert_eq!(delta.map(|delta| delta.order.status), Some(status));
        assert_eq!(delta.map(|delta| delta.order.quantity), Some(0.0));
        assert_eq!(delta.map(|delta| delta.order.filled_quantity), Some(0.0));
    }
}

#[test]
fn gate_non_terminal_order_requires_metadata_backed_query() {
    let events = map_gate_event(gate_ws_user::GateUserEvent::Order(vec![
        gate_ws_user::GateOrderUpdate {
            client_order_id: "t-live".to_owned(),
            live_state: LiveOrderState::Accepted,
            order: order_info("gate", OrderStatus::Open),
        },
    ]));

    assert_single_account_dirty(&events);
    assert!(!events
        .iter()
        .any(|event| matches!(event, PrivateWsEvent::Order(_))));
}

#[test]
fn gate_order_without_any_identity_fails_closed() {
    let mut order = order_info("gate", OrderStatus::Canceled);
    order.order_id.clear();
    order.client_order_id = None;
    let events = map_gate_event(gate_ws_user::GateUserEvent::Order(vec![
        gate_ws_user::GateOrderUpdate {
            client_order_id: String::new(),
            live_state: LiveOrderState::Cancelled,
            order,
        },
    ]));

    assert_single_account_dirty(&events);
}
