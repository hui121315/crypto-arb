use super::super::*;
use super::fixtures_b::*;

fn bitget_account_update(
    action: &str,
    accounts: Vec<bitget_ws_user::BitgetAccountDelta>,
) -> bitget_ws_user::BitgetAccountUpdate {
    bitget_ws_user::BitgetAccountUpdate {
        action: action.to_owned(),
        total_equity: 108.0,
        effective_equity: 80.0,
        initial_margin: 20.0,
        maintenance_margin: 5.0,
        margin_ratio: 0.2,
        position_margin_ratio: 0.05,
        unrealized_pnl: 1.5,
        accounts,
    }
}

/// PR-DP-08 D-6: subscribe 后首推 `action=snapshot` + UTA account → 全量替换
/// balance cache（按 venue 分桶），省一次 REST 余额拉取。
#[test]
fn bitget_account_snapshot_maps_to_balance_snapshot() {
    let events = map_bitget_event(bitget_ws_user::BitgetUserEvent::Account(
        bitget_account_update(
            "snapshot",
            vec![bitget_account_delta("USDT", 100.0, 80.0, 5.0, 108.0, 1.5)],
        ),
    ));
    let snapshot = events.iter().find_map(|event| match event {
        PrivateWsEvent::Balances(snapshot) => Some(snapshot.as_ref()),
        _ => None,
    });
    assert_eq!(snapshot.map(|snapshot| snapshot.rows.len()), Some(1));
    assert_eq!(
        snapshot.map(|snapshot| snapshot.rows[0].venue.as_str()),
        Some("bitget")
    );
    assert_eq!(
        snapshot.map(|snapshot| snapshot.rows[0].currency.as_str()),
        Some("USDT")
    );
    // USDT 行 usdt_equity (108.0) > equity (100.0) → 选 usdt_equity 作为 total，
    // 与 REST `bitget_uta_private_data::parse_balances` 行为一致。
    assert_eq!(snapshot.map(|snapshot| snapshot.rows[0].total), Some(108.0));
    assert_eq!(
        snapshot.map(|snapshot| snapshot.rows[0].available),
        Some(80.0)
    );
    assert_eq!(snapshot.map(|snapshot| snapshot.rows[0].frozen), Some(5.0));
    assert_eq!(
        snapshot.map(|snapshot| snapshot.rows[0].unrealized_pnl),
        Some(1.5)
    );
    // PR-DP-08 D-8：venue 决定写入哪个 per-venue cache entry，必须是 "bitget"。
    assert_eq!(
        snapshot.map(|snapshot| snapshot.venue.as_str()),
        Some("bitget")
    );
    let valuations = events.iter().find_map(|event| match event {
        PrivateWsEvent::AssetValuations(snapshot) => Some(snapshot.as_ref()),
        _ => None,
    });
    assert_eq!(valuations.map(|snapshot| snapshot.rows.len()), Some(1));
    assert_eq!(
        valuations.map(|snapshot| snapshot.rows[0].usd_value),
        Some(108.0)
    );
}
/// PR-DP-08 D-6: incremental `update` 不能盲合并 → 保留 dirty 全量重拉。
#[test]
fn bitget_account_update_marks_dirty() {
    let events = map_bitget_event(bitget_ws_user::BitgetUserEvent::Account(
        bitget_account_update(
            "update",
            vec![bitget_account_delta("USDT", 100.0, 80.0, 5.0, 108.0, 1.5)],
        ),
    ));
    assert!(matches!(
        events.as_slice(),
        [PrivateWsEvent::AccountDirty(_)]
    ));
}
/// PR-DP-08 D-6: 非 USDT 行（BTC / ETH UTA 钱包币种）→ 不 filter，与 REST
/// `get_balance(None)` 全 coin 返回一致；`coin != "USDT"` 行使用 `equity`
/// 而非 `usdt_equity` 作为 total，避免误把 USDT 折价价当原币总量。
#[test]
fn bitget_account_snapshot_keeps_non_usdt_coins_with_equity_as_total() {
    let events = map_bitget_event(bitget_ws_user::BitgetUserEvent::Account(
        bitget_account_update(
            "snapshot",
            vec![bitget_account_delta("BTC", 0.01, 0.005, 0.0, 600.0, 0.0)],
        ),
    ));
    let snapshot = events.iter().find_map(|event| match event {
        PrivateWsEvent::Balances(snapshot) => Some(snapshot.as_ref()),
        _ => None,
    });
    assert_eq!(snapshot.map(|snapshot| snapshot.rows.len()), Some(1));
    assert_eq!(
        snapshot.map(|snapshot| snapshot.rows[0].currency.as_str()),
        Some("BTC")
    );
    // BTC 行：equity=0.01，usdt_equity=600（折价），mapper 使用 equity（原币量）。
    assert_eq!(snapshot.map(|snapshot| snapshot.rows[0].total), Some(0.01));
}
/// PR-DP-08 D-6: 空 snapshot（账号无任何 coin balance）→ 全量替换为空表。
#[test]
fn bitget_account_empty_snapshot_replaces_with_empty_rows() {
    let events = map_bitget_event(bitget_ws_user::BitgetUserEvent::Account(
        bitget_account_update("snapshot", Vec::new()),
    ));
    let snapshot = events.iter().find_map(|event| match event {
        PrivateWsEvent::Balances(snapshot) => Some(snapshot.as_ref()),
        _ => None,
    });
    assert_eq!(snapshot.map(|snapshot| snapshot.rows.len()), Some(0));
}
#[test]
fn bitget_fill_maps_to_private_fill_delta() {
    let events = map_bitget_event(bitget_ws_user::BitgetUserEvent::Fill(vec![
        bitget_fill_delta("1288888888888888888", "1288888888888888887"),
    ]));
    let fill = events.iter().find_map(|event| match event {
        PrivateWsEvent::Fill(fill) => Some(fill),
        _ => None,
    });
    assert_eq!(
        fill.map(|fill| fill.exchange_order_id.as_str()),
        Some("1288888888888888888")
    );
    assert_eq!(fill.map(|fill| fill.venue.as_str()), Some("bitget"));
    assert_eq!(
        fill.and_then(|fill| fill.client_order_id.as_deref()),
        Some("cid-1")
    );
    assert_eq!(fill.and_then(|fill| fill.symbol.as_deref()), Some("BTC"));
    assert_eq!(fill.and_then(|fill| fill.side), Some(OrderSide::Buy));
    assert_eq!(
        fill.map(|fill| fill.venue_event_id.as_str()),
        Some("bitget_fill:1288888888888888888:1288888888888888887")
    );
    assert_eq!(fill.map(|fill| fill.quantity), Some(0.01));
    assert_eq!(fill.map(|fill| fill.price), Some(94_993.0));
    assert_eq!(fill.and_then(|fill| fill.fee_amount), Some(0.569958));
    assert_eq!(
        fill.and_then(|fill| fill.fee_currency.as_deref()),
        Some("USDT")
    );
    assert_eq!(
        fill.map(|fill| fill.occurred_at_ms),
        Some(1_736_378_720_623)
    );
    assert!(!events
        .iter()
        .any(|event| matches!(event, PrivateWsEvent::AccountDirty(_))));
}
