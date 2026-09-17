use super::*;

/// PR-DP-08 D-3: 空 snapshot（账号无持仓）→ 全量替换为空表。
#[test]
fn bybit_position_empty_snapshot_replaces_with_empty_rows() {
    let events = map_bybit_event(bybit_ws_user::BybitUserEvent::Position(
        bybit_ws_user::BybitPositionUpdate {
            update_type: "snapshot".to_owned(),
            positions: Vec::new(),
        },
    ));
    let snapshot = events.iter().find_map(|event| match event {
        PrivateWsEvent::Positions(snapshot) => Some(snapshot),
        _ => None,
    });
    assert_eq!(snapshot.map(|snapshot| snapshot.rows.len()), Some(0));
}

/// PR-DP-08 D-5: 空 snapshot（账号无持仓）→ 全量替换为空表。
#[test]
fn bitget_position_empty_snapshot_replaces_with_empty_rows() {
    let events = map_bitget_event(bitget_ws_user::BitgetUserEvent::Position(
        bitget_ws_user::BitgetPositionUpdate {
            action: "snapshot".to_owned(),
            positions: Vec::new(),
        },
    ));
    let snapshot = events.iter().find_map(|event| match event {
        PrivateWsEvent::Positions(snapshot) => Some(snapshot),
        _ => None,
    });
    assert_eq!(snapshot.map(|snapshot| snapshot.rows.len()), Some(0));
}
