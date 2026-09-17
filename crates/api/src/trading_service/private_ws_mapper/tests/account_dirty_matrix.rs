use super::super::*;
use super::fixtures_a::{hyperliquid_clearinghouse_state, hyperliquid_position_delta};
use super::fixtures_b::{bybit_position_delta, kucoin_balance_delta, okx_position_delta};

#[test]
fn seven_venue_account_dirty_matrix_preserves_scope_and_reason() {
    let cases = [
        dirty_from(map_binance_event(
            binance_ws_user::BinanceUserEvent::Account(binance_ws_user::BinanceAccountUpdate {
                event_time_ms: 1,
                transaction_time_ms: 2,
                reason: "ORDER".to_owned(),
                balances: Vec::new(),
                positions: Vec::new(),
            }),
        )),
        dirty_from(map_okx_event(okx_ws_user::OkxUserEvent::Position(
            okx_ws_user::OkxPositionUpdate {
                event_type: "regular update".to_owned(),
                last_page: true,
                positions: vec![okx_position_delta("BTC-USDT-SWAP", 1.0, "long")],
            },
        ))),
        dirty_from(map_bybit_event(bybit_ws_user::BybitUserEvent::Position(
            bybit_ws_user::BybitPositionUpdate {
                update_type: "delta".to_owned(),
                positions: vec![bybit_position_delta("BTC", "linear", 1.0, "long")],
            },
        ))),
        dirty_from(map_bitget_event(bitget_ws_user::BitgetUserEvent::Account(
            bitget_ws_user::BitgetAccountUpdate {
                action: "update".to_owned(),
                total_equity: 100.0,
                effective_equity: 90.0,
                initial_margin: 10.0,
                maintenance_margin: 2.0,
                margin_ratio: 0.1,
                position_margin_ratio: 0.02,
                unrealized_pnl: 0.0,
                accounts: Vec::new(),
            },
        ))),
        dirty_from(map_gate_event(gate_ws_user::GateUserEvent::Balance(
            Vec::new(),
        ))),
        dirty_from(map_kucoin_event(kucoin_ws_user::KucoinUserEvent::Balance(
            kucoin_balance_delta("availableBalance.change", 100.0, 90.0),
        ))),
        dirty_from(map_hyperliquid_event(
            hyperliquid_ws_user::HyperliquidUserWsEvent::Clearinghouse(
                hyperliquid_clearinghouse_state(
                    Some("builder"),
                    100.0,
                    10.0,
                    90.0,
                    vec![hyperliquid_position_delta("BTC", "long", 1.0, 2.0)],
                ),
            ),
        )),
    ];

    let expected = [
        ("binance", PrivateAccountScope::All),
        ("okx", PrivateAccountScope::Positions),
        ("bybit", PrivateAccountScope::Positions),
        ("bitget", PrivateAccountScope::Balances),
        ("gate", PrivateAccountScope::Balances),
        ("kucoin", PrivateAccountScope::Balances),
        ("hyperliquid:builder", PrivateAccountScope::Positions),
    ];
    for (dirty, (venue, scope)) in cases.iter().zip(expected) {
        assert_eq!(dirty.as_ref().map(|item| item.venue.as_str()), Some(venue));
        assert_eq!(dirty.as_ref().map(|item| item.scope), Some(scope));
        assert!(dirty
            .as_ref()
            .is_some_and(|item| !item.reason.trim().is_empty()));
    }
}

fn dirty_from(events: Vec<PrivateWsEvent>) -> Option<PrivateAccountDirty> {
    events.into_iter().find_map(|event| match event {
        PrivateWsEvent::AccountDirty(dirty) => Some(dirty),
        _ => None,
    })
}
