use super::super::*;
use super::fixtures_a::*;
use super::fixtures_b::*;
use shared_types::FundingPaymentData;

#[test]
fn private_rest_funding_payment_rejects_empty_or_zero_delta() {
    assert!(funding_payment_delta(FundingPaymentData {
        venue: "okx".to_owned(),
        symbol: "BTC".to_owned(),
        amount: 0.0,
        currency: "USDT".to_owned(),
        funding_time_ms: 1,
        venue_event_id: "okx_funding:1".to_owned(),
    })
    .is_none());
}

#[test]
fn hyperliquid_liquidation_maps_to_typed_account_event() {
    let events = map_hyperliquid_event(hyperliquid_ws_user::HyperliquidUserWsEvent::Liquidation(
        hyperliquid_liquidation(7, 100.0, 10.0),
    ));

    let liquidation = events.iter().find_map(|event| match event {
        PrivateWsEvent::Liquidation(liquidation) => Some(liquidation),
        _ => None,
    });
    assert_eq!(
        liquidation.map(|event| event.venue_event_id.as_str()),
        Some("hyperliquid_liquidation:7")
    );
    assert_eq!(
        liquidation.map(|event| event.notional_position),
        Some(100.0)
    );
    assert_eq!(liquidation.map(|event| event.account_value), Some(10.0));
}

#[test]
fn hyperliquid_non_user_cancel_maps_to_order_cancel_event() {
    let events = map_hyperliquid_event(hyperliquid_ws_user::HyperliquidUserWsEvent::NonUserCancel(
        vec![hyperliquid_non_user_cancel("BTC", "12345")],
    ));

    let cancel = events.iter().find_map(|event| match event {
        PrivateWsEvent::NonUserCancel(cancel) => Some(cancel),
        _ => None,
    });
    assert_eq!(
        cancel.map(|cancel| cancel.venue_event_id.as_str()),
        Some("hyperliquid_non_user_cancel:hyperliquid:BTC:12345")
    );
    assert_eq!(
        cancel.map(|cancel| cancel.exchange_order_id.as_str()),
        Some("12345")
    );
    assert_eq!(cancel.map(|cancel| cancel.coin.as_str()), Some("BTC"));
}
/// PR-DP-08 D-1: OKX `eventType=snapshot` + `lastPage=true` →
/// 全量替换 position cache（不触发 dirty / 不等下一轮 REST）。
#[test]
fn okx_position_snapshot_last_page_maps_to_position_snapshot() {
    let events = map_okx_event(okx_ws_user::OkxUserEvent::Position(
        okx_ws_user::OkxPositionUpdate {
            event_type: "snapshot".to_owned(),
            last_page: true,
            positions: vec![okx_position_delta("BTC-USDT-SWAP", 1.5, "long")],
        },
    ));

    let snapshot = events.iter().find_map(|event| match event {
        PrivateWsEvent::Positions(snapshot) => Some(snapshot),
        _ => None,
    });
    assert_eq!(snapshot.map(|snapshot| snapshot.rows.len()), Some(1));
    assert_eq!(
        snapshot.map(|snapshot| snapshot.rows[0].exchange.as_str()),
        Some("okx")
    );
    assert_eq!(
        snapshot.map(|snapshot| snapshot.rows[0].symbol.as_str()),
        Some("BTC")
    );
    assert_eq!(
        snapshot.map(|snapshot| snapshot.rows[0].quantity),
        Some(1.5)
    );
    assert_eq!(
        snapshot.map(|snapshot| snapshot.rows[0].side.as_str()),
        Some("long")
    );
}
/// PR-DP-08 D-1: snapshot 但 `last_page=false`（分页中间帧）→ dirty，等下一页或 REST。
#[test]
fn okx_position_snapshot_mid_page_still_marks_dirty() {
    let events = map_okx_event(okx_ws_user::OkxUserEvent::Position(
        okx_ws_user::OkxPositionUpdate {
            event_type: "snapshot".to_owned(),
            last_page: false,
            positions: vec![okx_position_delta("BTC-USDT-SWAP", 1.5, "long")],
        },
    ));
    assert!(matches!(
        events.as_slice(),
        [PrivateWsEvent::AccountDirty(_)]
    ));
}
/// PR-DP-08 D-1: incremental `regular update` 不能盲合并 → 保留 dirty 全量重拉。
#[test]
fn okx_position_regular_update_marks_dirty() {
    let events = map_okx_event(okx_ws_user::OkxUserEvent::Position(
        okx_ws_user::OkxPositionUpdate {
            event_type: "regular update".to_owned(),
            last_page: true,
            positions: vec![okx_position_delta("BTC-USDT-SWAP", 1.5, "long")],
        },
    ));
    assert!(matches!(
        events.as_slice(),
        [PrivateWsEvent::AccountDirty(_)]
    ));
}
/// PR-DP-08 D-1: 空 snapshot 也是合法状态（已平仓 / 新账号），全量替换为空表，
/// 避免遗留旧 cache 行被 stale fallback 当作"还有仓位"。
#[test]
fn okx_position_empty_snapshot_replaces_with_empty_rows() {
    let events = map_okx_event(okx_ws_user::OkxUserEvent::Position(
        okx_ws_user::OkxPositionUpdate {
            event_type: "snapshot".to_owned(),
            last_page: true,
            positions: Vec::new(),
        },
    ));
    let snapshot = events.iter().find_map(|event| match event {
        PrivateWsEvent::Positions(snapshot) => Some(snapshot),
        _ => None,
    });
    assert_eq!(snapshot.map(|snapshot| snapshot.rows.len()), Some(0));
}
/// PR-DP-08 D-3: 完整 snapshot（V5 envelope `type=snapshot`）→ 全量替换
/// position cache（仅保留 `category=linear` 行）。
#[test]
fn bybit_position_snapshot_maps_to_position_snapshot() {
    let events = map_bybit_event(bybit_ws_user::BybitUserEvent::Position(
        bybit_ws_user::BybitPositionUpdate {
            update_type: "snapshot".to_owned(),
            positions: vec![bybit_position_delta("BTC", "linear", 0.02, "short")],
        },
    ));

    let snapshot = events.iter().find_map(|event| match event {
        PrivateWsEvent::Positions(snapshot) => Some(snapshot),
        _ => None,
    });
    assert_eq!(snapshot.map(|snapshot| snapshot.rows.len()), Some(1));
    assert_eq!(
        snapshot.map(|snapshot| snapshot.rows[0].exchange.as_str()),
        Some("bybit")
    );
    assert_eq!(
        snapshot.map(|snapshot| snapshot.rows[0].symbol.as_str()),
        Some("BTC")
    );
    assert_eq!(
        snapshot.map(|snapshot| snapshot.rows[0].quantity),
        Some(0.02)
    );
    assert_eq!(
        snapshot.map(|snapshot| snapshot.rows[0].side.as_str()),
        Some("short")
    );
}
/// PR-DP-08 D-3: incremental `delta` 不能盲合并 → 保留 dirty 全量重拉。
#[test]
fn bybit_position_delta_marks_dirty() {
    let events = map_bybit_event(bybit_ws_user::BybitUserEvent::Position(
        bybit_ws_user::BybitPositionUpdate {
            update_type: "delta".to_owned(),
            positions: vec![bybit_position_delta("BTC", "linear", 0.02, "short")],
        },
    ));
    assert!(matches!(
        events.as_slice(),
        [PrivateWsEvent::AccountDirty(_)]
    ));
}
/// PR-DP-08 D-3: snapshot 但全是 inverse/option 行 → filter linear 后变成
/// 空表，替换 cache 为空，避免历史 linear 残留被 stale fallback 误当作"还有仓位"。
#[test]
fn bybit_position_snapshot_filters_non_linear_categories() {
    let events = map_bybit_event(bybit_ws_user::BybitUserEvent::Position(
        bybit_ws_user::BybitPositionUpdate {
            update_type: "snapshot".to_owned(),
            positions: vec![
                bybit_position_delta("BTC", "inverse", 1.0, "long"),
                bybit_position_delta("ETH", "option", 1.0, "long"),
            ],
        },
    ));
    let snapshot = events.iter().find_map(|event| match event {
        PrivateWsEvent::Positions(snapshot) => Some(snapshot),
        _ => None,
    });
    assert_eq!(snapshot.map(|snapshot| snapshot.rows.len()), Some(0));
}
