use super::super::*;
use super::fixtures_b::{okx_account_summary_delta, okx_balance_delta};

#[test]
fn okx_account_snapshot_last_page_maps_to_balance_snapshot() {
    let events = map_okx_event(okx_ws_user::OkxUserEvent::Account(
        okx_ws_user::OkxAccountUpdate {
            event_type: "snapshot".to_owned(),
            last_page: true,
            summary: Some(okx_account_summary_delta()),
            balances: vec![okx_balance_delta("USDT", 1_000.0, 950.0)],
        },
    ));

    let snapshot = events.iter().find_map(|event| match event {
        PrivateWsEvent::Balances(snapshot) => Some(snapshot),
        _ => None,
    });
    assert_eq!(snapshot.map(|snapshot| snapshot.rows.len()), Some(1));
    assert_eq!(
        snapshot.map(|snapshot| snapshot.rows[0].venue.as_str()),
        Some("okx")
    );
    assert_eq!(
        snapshot.map(|snapshot| snapshot.rows[0].currency.as_str()),
        Some("USDT")
    );
    assert_eq!(
        snapshot.map(|snapshot| snapshot.rows[0].total),
        Some(1_000.0)
    );
    assert_eq!(
        snapshot.map(|snapshot| snapshot.rows[0].available),
        Some(950.0)
    );
    assert_eq!(
        snapshot.map(|snapshot| snapshot.venue.as_str()),
        Some("okx")
    );
    let summary = events.iter().find_map(|event| match event {
        PrivateWsEvent::AccountSummary(summary) => Some(summary),
        _ => None,
    });
    assert_eq!(
        summary.map(|summary| summary.total_equity_usd),
        Some(1_000.0)
    );
    assert_eq!(
        summary.map(|summary| summary.source.as_str()),
        Some("okx.private_ws.account")
    );
}

#[test]
fn okx_account_snapshot_mid_page_still_marks_dirty() {
    let events = map_okx_event(okx_ws_user::OkxUserEvent::Account(
        okx_ws_user::OkxAccountUpdate {
            event_type: "snapshot".to_owned(),
            last_page: false,
            summary: None,
            balances: vec![okx_balance_delta("USDT", 1_000.0, 950.0)],
        },
    ));
    assert!(matches!(
        events.as_slice(),
        [PrivateWsEvent::AccountDirty(_)]
    ));
}

#[test]
fn okx_account_regular_update_marks_dirty() {
    let events = map_okx_event(okx_ws_user::OkxUserEvent::Account(
        okx_ws_user::OkxAccountUpdate {
            event_type: "regular update".to_owned(),
            last_page: true,
            summary: None,
            balances: vec![okx_balance_delta("USDT", 1_000.0, 950.0)],
        },
    ));
    assert!(matches!(
        events.as_slice(),
        [PrivateWsEvent::AccountDirty(_)]
    ));
}

#[test]
fn okx_account_empty_snapshot_replaces_with_empty_rows() {
    let events = map_okx_event(okx_ws_user::OkxUserEvent::Account(
        okx_ws_user::OkxAccountUpdate {
            event_type: "snapshot".to_owned(),
            last_page: true,
            summary: None,
            balances: Vec::new(),
        },
    ));
    let snapshot = events.iter().find_map(|event| match event {
        PrivateWsEvent::Balances(snapshot) => Some(snapshot),
        _ => None,
    });
    assert_eq!(snapshot.map(|snapshot| snapshot.rows.len()), Some(0));
}
