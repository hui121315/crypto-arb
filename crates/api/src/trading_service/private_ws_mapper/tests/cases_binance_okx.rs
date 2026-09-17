use super::super::*;
use super::fixtures_b::*;
use shared_types::{LiveOrderState, OrderStatus};

#[test]
fn binance_account_update_projects_positions_and_scopes_balance_refresh() {
    let events = map_binance_event(binance_ws_user::BinanceUserEvent::Account(
        binance_ws_user::BinanceAccountUpdate {
            event_time_ms: 10,
            transaction_time_ms: 11,
            reason: "ORDER".to_owned(),
            balances: vec![binance_ws_user::BinanceAccountBalanceDelta {
                asset: "USDT".to_owned(),
                wallet_balance: 100.0,
                cross_wallet_balance: 80.0,
                balance_change: -1.0,
            }],
            positions: vec![binance_ws_user::BinancePositionDelta {
                symbol: "SOL".to_owned(),
                side: "BOTH".to_owned(),
                quantity: -2.5,
                entry_price: 180.0,
                accumulated_realized: 1.0,
                unrealized_pnl: 3.0,
                margin_type: "cross".to_owned(),
                isolated_wallet: 0.0,
            }],
        },
    ));

    let patch = events.iter().find_map(|event| match event {
        PrivateWsEvent::PositionPatch(patch) => Some(patch),
        _ => None,
    });
    let dirty = events.iter().find_map(|event| match event {
        PrivateWsEvent::AccountDirty(dirty) => Some(dirty),
        _ => None,
    });

    assert!(patch.is_some(), "binance position patch missing");
    if let Some(patch) = patch {
        assert_eq!(patch.venue, "binance");
        assert!(patch
            .rows
            .iter()
            .any(|row| row.side == "short" && row.quantity == 2.5));
        assert!(patch
            .rows
            .iter()
            .any(|row| row.side == "long" && row.quantity == 0.0));
        let short = patch.rows.iter().find(|row| row.side == "short");
        assert!(short.is_some(), "short row missing");
        if let Some(short) = short {
            assert_eq!(short.entry_price, 180.0);
            assert_eq!(short.unrealized_pnl, 3.0);
            assert_eq!(short.position_mode.as_deref(), Some("one_way"));
            assert_eq!(short.margin_mode.as_deref(), Some("cross"));
        }
    }
    assert_eq!(
        dirty.map(|dirty| dirty.scope),
        Some(PrivateAccountScope::Balances)
    );
}

#[test]
fn binance_position_only_update_does_not_force_rest_position_refresh() {
    let events = map_binance_event(binance_ws_user::BinanceUserEvent::Account(
        binance_ws_user::BinanceAccountUpdate {
            event_time_ms: 10,
            transaction_time_ms: 11,
            reason: "ORDER".to_owned(),
            balances: Vec::new(),
            positions: vec![binance_ws_user::BinancePositionDelta {
                symbol: "SOL".to_owned(),
                side: "LONG".to_owned(),
                quantity: 0.0,
                entry_price: 0.0,
                accumulated_realized: 1.0,
                unrealized_pnl: 0.0,
                margin_type: "isolated".to_owned(),
                isolated_wallet: 0.0,
            }],
        },
    ));

    assert!(matches!(
        events.as_slice(),
        [PrivateWsEvent::PositionPatch(_)]
    ));
}

#[test]
fn order_update_with_client_id_maps_to_order_delta() {
    let events = map_binance_event(binance_ws_user::BinanceUserEvent::Order(Box::new(
        binance_ws_user::BinanceOrderTradeUpdate {
            event_time_ms: 10,
            transaction_time_ms: 12,
            trade_time_ms: 13,
            client_order_id: "cid-1".to_owned(),
            execution_type: "TRADE".to_owned(),
            order_status: "FILLED".to_owned(),
            reject_reason: Some("NONE".to_owned()),
            trade_id: Some(777),
            last_filled_quantity: 1.0,
            last_filled_price: 100.0,
            commission_asset: Some("USDT".to_owned()),
            live_state: LiveOrderState::Filled,
            order: order_info("binance", OrderStatus::Filled),
        },
    )));

    let delta = events.iter().find_map(|event| match event {
        PrivateWsEvent::BinanceOrderTrade(delta) => Some(&delta.order),
        _ => None,
    });
    assert_eq!(
        delta.map(|delta| delta.client_order_id.as_str()),
        Some("cid-1")
    );
    assert_eq!(delta.map(|delta| delta.received_at_ms), Some(12));
    assert_eq!(
        delta.map(|delta| delta.order.status),
        Some(OrderStatus::Filled)
    );
}

#[test]
fn binance_trade_update_maps_to_private_fill_delta() {
    let mut order = order_info("binance", OrderStatus::PartiallyFilled);
    order.order_id = "8886774".to_owned();
    order.fees = 0.12;
    let events = map_binance_event(binance_ws_user::BinanceUserEvent::Order(Box::new(
        binance_ws_user::BinanceOrderTradeUpdate {
            event_time_ms: 1_568_879_465_651,
            transaction_time_ms: 1_568_879_465_650,
            trade_time_ms: 1_568_879_465_652,
            client_order_id: "cid-1".to_owned(),
            execution_type: "TRADE".to_owned(),
            order_status: "PARTIALLY_FILLED".to_owned(),
            reject_reason: Some("NONE".to_owned()),
            trade_id: Some(1_234_567),
            last_filled_quantity: 0.003,
            last_filled_price: 50_010.0,
            commission_asset: Some("USDT".to_owned()),
            live_state: LiveOrderState::PartiallyFilled,
            order,
        },
    )));

    let fill = events.iter().find_map(|event| match event {
        PrivateWsEvent::BinanceOrderTrade(delta) => delta.fill.as_ref(),
        _ => None,
    });
    assert!(matches!(
        events.as_slice(),
        [PrivateWsEvent::BinanceOrderTrade(_)]
    ));
    assert_eq!(fill.map(|fill| fill.venue.as_str()), Some("binance"));
    assert_eq!(
        fill.map(|fill| fill.exchange_order_id.as_str()),
        Some("8886774")
    );
    assert_eq!(
        fill.and_then(|fill| fill.client_order_id.as_deref()),
        Some("cid-1")
    );
    assert_eq!(fill.and_then(|fill| fill.symbol.as_deref()), Some("BTC"));
    assert_eq!(fill.and_then(|fill| fill.side), Some(OrderSide::Buy));
    assert_eq!(
        fill.map(|fill| fill.venue_event_id.as_str()),
        Some("binance_trade:8886774:1234567")
    );
    assert_eq!(fill.map(|fill| fill.quantity), Some(0.003));
    assert_eq!(fill.map(|fill| fill.price), Some(50_010.0));
    assert_eq!(fill.and_then(|fill| fill.fee_amount), Some(0.12));
    assert_eq!(
        fill.and_then(|fill| fill.fee_currency.as_deref()),
        Some("USDT")
    );
    assert_eq!(
        fill.map(|fill| fill.occurred_at_ms),
        Some(1_568_879_465_652)
    );
}

#[test]
fn order_update_without_client_id_preserves_exchange_order_id() {
    let events = map_okx_event(okx_ws_user::OkxUserEvent::Order(vec![
        okx_ws_user::OkxOrderUpdate {
            client_order_id: String::new(),
            live_state: LiveOrderState::Filled,
            updated_time_ms: 20,
            fill: None,
            order: order_info("okx", OrderStatus::Filled),
        },
    ]));

    let order = events.iter().find_map(|event| match event {
        PrivateWsEvent::Order(order) => Some(order),
        _ => None,
    });
    assert_eq!(order.map(|order| order.client_order_id.as_str()), Some(""));
    assert_eq!(order.map(|order| order.order.order_id.as_str()), Some("o1"));
    assert_eq!(order.map(|order| order.received_at_ms), Some(20));
}

#[test]
fn okx_trade_update_maps_to_private_fill_delta() {
    let mut order = order_info("okx", OrderStatus::PartiallyFilled);
    order.order_id = "680800019749904384".to_owned();
    order.fees = -0.01;
    let events = map_okx_event(okx_ws_user::OkxUserEvent::Order(vec![
        okx_ws_user::OkxOrderUpdate {
            client_order_id: "cid-1".to_owned(),
            live_state: LiveOrderState::PartiallyFilled,
            updated_time_ms: 1_708_587_373_362,
            fill: Some(okx_ws_user::OkxOrderFillUpdate {
                trade_id: "751159184".to_owned(),
                fill_price: 51_858.0,
                fill_size: 0.2,
                fill_fee: Some(0.004),
                fill_fee_currency: Some("USDT".to_owned()),
                fill_time_ms: 1_708_587_373_361,
            }),
            order,
        },
    ]));

    let fill = events.iter().find_map(|event| match event {
        PrivateWsEvent::Fill(fill) => Some(fill),
        _ => None,
    });
    assert!(events
        .iter()
        .any(|event| matches!(event, PrivateWsEvent::Order(_))));
    assert_eq!(fill.map(|fill| fill.venue.as_str()), Some("okx"));
    assert_eq!(
        fill.map(|fill| fill.exchange_order_id.as_str()),
        Some("680800019749904384")
    );
    assert_eq!(
        fill.and_then(|fill| fill.client_order_id.as_deref()),
        Some("cid-1")
    );
    assert_eq!(fill.and_then(|fill| fill.symbol.as_deref()), Some("BTC"));
    assert_eq!(fill.and_then(|fill| fill.side), Some(OrderSide::Buy));
    assert_eq!(
        fill.map(|fill| fill.venue_event_id.as_str()),
        Some("okx_trade:680800019749904384:751159184")
    );
    assert_eq!(fill.map(|fill| fill.quantity), Some(0.2));
    assert_eq!(fill.map(|fill| fill.price), Some(51_858.0));
    assert_eq!(fill.and_then(|fill| fill.fee_amount), Some(0.004));
    assert_eq!(
        fill.and_then(|fill| fill.fee_currency.as_deref()),
        Some("USDT")
    );
    assert_eq!(
        fill.map(|fill| fill.occurred_at_ms),
        Some(1_708_587_373_361)
    );
}
