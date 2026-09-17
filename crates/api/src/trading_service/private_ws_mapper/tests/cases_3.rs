use super::super::*;
use super::fixtures_a::*;

#[test]
fn hyperliquid_open_orders_maps_as_a_complete_venue_snapshot() {
    let events = map_hyperliquid_event(hyperliquid_ws_user::HyperliquidUserWsEvent::OpenOrders(
        hyperliquid_ws_user::HyperliquidOpenOrdersSnapshot {
            venue: "hyperliquid:xyz".to_owned(),
            orders: Vec::new(),
        },
    ));

    assert!(matches!(
        events.as_slice(),
        [PrivateWsEvent::OpenOrders(snapshot)]
            if snapshot.venue == "hyperliquid:xyz" && snapshot.rows.is_empty()
    ));
}

#[test]
fn hyperliquid_all_dexs_clearinghouse_maps_each_dex_snapshot() {
    let events = map_hyperliquid_event(
        hyperliquid_ws_user::HyperliquidUserWsEvent::AllDexsClearinghouse(vec![
            hyperliquid_dex_state(
                "xyz",
                100.0,
                vec![hyperliquid_position_delta("MU", "long", 1.0, 2.0)],
            ),
            hyperliquid_dex_state(
                "km",
                50.0,
                vec![hyperliquid_position_delta("US500", "short", 0.5, -1.0)],
            ),
        ]),
    );

    let balance_venues: Vec<&str> = events
        .iter()
        .filter_map(|event| match event {
            PrivateWsEvent::Balances(snapshot) => Some(snapshot.venue.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(balance_venues, vec!["hyperliquid:xyz", "hyperliquid:km"]);
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, PrivateWsEvent::AccountDirty(_)))
            .count(),
        2
    );
}

#[test]
fn hyperliquid_all_dexs_snapshot_clears_omitted_configured_dex_positions() {
    let events = map_hyperliquid_event(
        hyperliquid_ws_user::HyperliquidUserWsEvent::AllDexsClearinghouse(vec![
            hyperliquid_dex_state("", 100.0, Vec::new()),
            hyperliquid_dex_state(
                "xyz",
                50.0,
                vec![hyperliquid_position_delta("MU", "long", 1.0, 2.0)],
            ),
        ]),
    );

    let mut position_venues = events
        .iter()
        .filter_map(|event| match event {
            PrivateWsEvent::Positions(snapshot) => Some(snapshot.venue.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    position_venues.sort_unstable();
    assert_eq!(position_venues, vec!["hyperliquid"]);
    let dirty_venues = events
        .iter()
        .filter_map(|event| match event {
            PrivateWsEvent::AccountDirty(dirty) => Some(dirty.venue.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(dirty_venues, vec!["hyperliquid:xyz"]);
}
/// PR-DP-08 D-7: 空 positions 的 default dex snapshot → position cache 替换为
/// 空表 + balance cache 替换为单条 USDC 行（`unrealized_pnl = 0`）。
#[test]
fn hyperliquid_clearinghouse_empty_positions_yields_empty_position_snapshot_and_zero_pnl_balance() {
    let events = map_hyperliquid_event(hyperliquid_ws_user::HyperliquidUserWsEvent::Clearinghouse(
        hyperliquid_clearinghouse_state(None, 50.0, 0.0, 50.0, Vec::new()),
    ));
    let positions = events.iter().find_map(|event| match event {
        PrivateWsEvent::Positions(snapshot) => Some(snapshot),
        _ => None,
    });
    let balances = events.iter().find_map(|event| match event {
        PrivateWsEvent::Balances(snapshot) => Some(snapshot.as_ref()),
        _ => None,
    });
    assert_eq!(positions.map(|snapshot| snapshot.rows.len()), Some(0));
    assert_eq!(balances.map(|snapshot| snapshot.rows.len()), Some(1));
    assert_eq!(balances.map(|snapshot| snapshot.rows[0].total), Some(50.0));
    assert_eq!(
        balances.map(|snapshot| snapshot.rows[0].unrealized_pnl),
        Some(0.0)
    );
}
/// PR-DP-08 D-7: positions 中含 `NaN` / `Inf` 的 `unrealized_pnl` 必须被跳过
/// （与 REST `parse_perp_balance` 的 `.filter(|v| v.is_finite())` 同语义），
/// 避免单仓 `NaN` 污染整个账户的 `unrealized_pnl` 累加结果。
#[test]
fn hyperliquid_clearinghouse_filters_non_finite_unrealized_pnl() {
    let events = map_hyperliquid_event(hyperliquid_ws_user::HyperliquidUserWsEvent::Clearinghouse(
        hyperliquid_clearinghouse_state(
            None,
            100.0,
            10.0,
            90.0,
            vec![
                hyperliquid_position_delta("BTC", "long", 1.0, 3.0),
                hyperliquid_position_delta("ETH", "long", 1.0, f64::NAN),
                hyperliquid_position_delta("SOL", "long", 1.0, f64::INFINITY),
            ],
        ),
    ));
    let balances = events.iter().find_map(|event| match event {
        PrivateWsEvent::Balances(snapshot) => Some(snapshot.as_ref()),
        _ => None,
    });
    // NaN 与 Inf 被跳过，仅 BTC 行 3.0 入累加。
    assert_eq!(
        balances.map(|snapshot| snapshot.rows[0].unrealized_pnl),
        Some(3.0)
    );
}
/// Hyperliquid 官方 `spotState` payload 是 `balances[]` 全量 spot state，
/// 映射口径与 REST `spotClearinghouseState` 对齐：`hold` 作为 frozen，
/// `total - hold` 作为 available，写入独立 `hyperliquid:spot` cache entry。
#[test]
fn hyperliquid_spot_state_maps_to_spot_balance_snapshot() {
    let events = map_hyperliquid_event(hyperliquid_ws_user::HyperliquidUserWsEvent::SpotState(
        vec![hyperliquid_spot_balance_delta("USDC", 1.2, 0.2)],
    ));

    let balances = events.iter().find_map(|event| match event {
        PrivateWsEvent::Balances(snapshot) => Some(snapshot.as_ref()),
        _ => None,
    });
    assert_eq!(
        balances.map(|snapshot| snapshot.venue.as_str()),
        Some("hyperliquid:spot")
    );
    assert_eq!(balances.map(|snapshot| snapshot.rows.len()), Some(1));
    assert_eq!(
        balances.map(|snapshot| snapshot.rows[0].venue.as_str()),
        Some("hyperliquid:spot")
    );
    assert_eq!(
        balances.map(|snapshot| snapshot.rows[0].currency.as_str()),
        Some("USDC")
    );
    assert_eq!(balances.map(|snapshot| snapshot.rows[0].total), Some(1.2));
    assert_eq!(
        balances.map(|snapshot| snapshot.rows[0].available),
        Some(1.0)
    );
    assert_eq!(balances.map(|snapshot| snapshot.rows[0].frozen), Some(0.2));
    assert_eq!(
        balances.map(|snapshot| snapshot.rows[0].unrealized_pnl),
        Some(0.0)
    );
    assert!(!events
        .iter()
        .any(|event| matches!(event, PrivateWsEvent::AccountDirty(_))));
}
#[test]
fn hyperliquid_spot_state_empty_replaces_with_empty_balance_snapshot() {
    let events = map_hyperliquid_event(hyperliquid_ws_user::HyperliquidUserWsEvent::SpotState(
        Vec::new(),
    ));

    let balances = events.iter().find_map(|event| match event {
        PrivateWsEvent::Balances(snapshot) => Some(snapshot.as_ref()),
        _ => None,
    });
    assert_eq!(
        balances.map(|snapshot| snapshot.venue.as_str()),
        Some("hyperliquid:spot")
    );
    assert_eq!(balances.map(|snapshot| snapshot.rows.len()), Some(0));
    assert!(!events
        .iter()
        .any(|event| matches!(event, PrivateWsEvent::AccountDirty(_))));
}
