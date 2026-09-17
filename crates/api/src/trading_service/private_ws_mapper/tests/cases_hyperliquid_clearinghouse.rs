use super::super::*;
use super::fixtures_a::*;

#[test]
fn default_hyperliquid_clearinghouse_with_positions_does_not_clear_all_dex_caches() {
    let events = map_hyperliquid_event(hyperliquid_ws_user::HyperliquidUserWsEvent::Clearinghouse(
        hyperliquid_clearinghouse_state(
            None,
            100.0,
            10.0,
            90.0,
            vec![hyperliquid_position_delta("BTC", "long", 1.0, 2.0)],
        ),
    ));

    assert!(!events
        .iter()
        .any(|event| matches!(event, PrivateWsEvent::Positions(_))));
}

/// PR-DP-08 D-7: default dex (`dex == None`) clearinghouse 帧 → 非空仓位
/// 缺官方 markPx，不能触发无范围的 position cache dirty；balance 字段仍映射 USDC 单行：
/// `total = account_value`, `available = withdrawable`, `frozen =
/// total_margin_used`, `unrealized_pnl = Σ positions[].unrealized_pnl`，
/// 与 REST `parse_perp_balance` 完全一致。
#[test]
fn hyperliquid_clearinghouse_with_credentials_maps_balance_without_cross_dex_dirty() {
    let events = map_hyperliquid_event(hyperliquid_ws_user::HyperliquidUserWsEvent::Clearinghouse(
        hyperliquid_clearinghouse_state(
            None,
            100.0,
            10.0,
            90.0,
            vec![
                hyperliquid_position_delta("BTC", "long", 1.0, 2.0),
                hyperliquid_position_delta("ETH", "short", 5.0, -1.5),
            ],
        ),
    ));

    let balances = events.iter().find_map(|event| match event {
        PrivateWsEvent::Balances(snapshot) => Some(snapshot.as_ref()),
        _ => None,
    });
    assert!(!events
        .iter()
        .any(|event| matches!(event, PrivateWsEvent::Positions(_))));
    assert_eq!(balances.map(|snapshot| snapshot.rows.len()), Some(1));
    assert_eq!(
        balances.map(|snapshot| snapshot.rows[0].venue.as_str()),
        Some("hyperliquid")
    );
    assert_eq!(
        balances.map(|snapshot| snapshot.rows[0].currency.as_str()),
        Some("USDC")
    );
    assert_eq!(balances.map(|snapshot| snapshot.rows[0].total), Some(100.0));
    assert_eq!(
        balances.map(|snapshot| snapshot.rows[0].available),
        Some(90.0)
    );
    assert_eq!(balances.map(|snapshot| snapshot.rows[0].frozen), Some(10.0));
    assert!(balances
        .map(|snapshot| (snapshot.rows[0].unrealized_pnl - 0.5).abs() < 1e-9)
        .unwrap_or(false));
    assert_eq!(
        balances.map(|snapshot| snapshot.venue.as_str()),
        Some("hyperliquid")
    );
}

#[test]
fn hyperliquid_clearinghouse_sub_dex_maps_to_scoped_snapshots() {
    let events = map_hyperliquid_event(hyperliquid_ws_user::HyperliquidUserWsEvent::Clearinghouse(
        hyperliquid_clearinghouse_state(
            Some("xyz"),
            100.0,
            10.0,
            90.0,
            vec![hyperliquid_position_delta("xyz:BTC", "long", 1.0, 2.0)],
        ),
    ));
    let balances = events.iter().find_map(|event| match event {
        PrivateWsEvent::Balances(snapshot) => Some(snapshot.as_ref()),
        _ => None,
    });
    assert!(!events
        .iter()
        .any(|event| matches!(event, PrivateWsEvent::Positions(_))));
    assert_eq!(
        balances.map(|snapshot| snapshot.venue.as_str()),
        Some("hyperliquid:xyz")
    );
    assert_eq!(
        balances.map(|snapshot| snapshot.rows[0].venue.as_str()),
        Some("hyperliquid:xyz")
    );
}
