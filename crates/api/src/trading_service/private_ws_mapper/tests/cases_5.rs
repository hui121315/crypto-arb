use super::super::*;
use super::assert_single_account_dirty;
use super::fixtures_b::*;

mod empty_snapshots;
/// PR-DP-08 D-4: subscribe 后首推 `type=snapshot` + UNIFIED account →
/// 全量替换 balance cache（按 venue 分桶），省一次 REST 余额拉取。
#[test]
fn bybit_wallet_snapshot_unified_maps_to_balance_snapshot() {
    let events = map_bybit_event(bybit_ws_user::BybitUserEvent::Wallet(
        bybit_ws_user::BybitWalletUpdate {
            update_type: "snapshot".to_owned(),
            observed_at_ms: 1_700_000_000_000,
            accounts: vec![bybit_wallet_account_with_available(
                "UNIFIED",
                950.0,
                vec![bybit_wallet_coin("USDT", 1_000.0, Some(1.0))],
            )],
        },
    ));

    let snapshot = events.iter().find_map(|event| match event {
        PrivateWsEvent::Balances(snapshot) => Some(snapshot),
        _ => None,
    });
    assert_eq!(snapshot.map(|snapshot| snapshot.rows.len()), Some(1));
    assert_eq!(
        snapshot.map(|snapshot| snapshot.rows[0].venue.as_str()),
        Some("bybit")
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
    // PR-DP-08 D-8：venue 决定写入哪个 per-venue cache entry，必须是 "bybit"。
    assert_eq!(
        snapshot.map(|snapshot| snapshot.venue.as_str()),
        Some("bybit")
    );
    let valuations = events.iter().find_map(|event| match event {
        PrivateWsEvent::AssetValuations(snapshot) => Some(snapshot.as_ref()),
        _ => None,
    });
    assert_eq!(valuations.map(|snapshot| snapshot.rows.len()), Some(1));
    assert_eq!(
        valuations.map(|snapshot| snapshot.rows[0].usd_value),
        Some(1_000.0)
    );
}
/// PR-DP-08 D-4: incremental `delta` 不能盲合并 → 保留 dirty 全量重拉。
#[test]
fn bybit_wallet_delta_marks_dirty() {
    let events = map_bybit_event(bybit_ws_user::BybitUserEvent::Wallet(
        bybit_ws_user::BybitWalletUpdate {
            update_type: "delta".to_owned(),
            observed_at_ms: 1_700_000_000_000,
            accounts: vec![bybit_wallet_account(
                "UNIFIED",
                vec![bybit_wallet_coin("USDT", 1_000.0, Some(950.0))],
            )],
        },
    ));
    assert_single_account_dirty(&events);
}
/// PR-DP-08 D-4: snapshot 但只有 CONTRACT/SPOT 行（Classic 账户）→ filter
/// UNIFIED 后变成空表，与 REST default `accountType=UNIFIED` 行为对齐。
#[test]
fn bybit_wallet_snapshot_filters_non_unified_accounts() {
    let events = map_bybit_event(bybit_ws_user::BybitUserEvent::Wallet(
        bybit_ws_user::BybitWalletUpdate {
            update_type: "snapshot".to_owned(),
            observed_at_ms: 1_700_000_000_000,
            accounts: vec![
                bybit_wallet_account(
                    "CONTRACT",
                    vec![bybit_wallet_coin("USDT", 100.0, Some(90.0))],
                ),
                bybit_wallet_account("SPOT", vec![bybit_wallet_coin("BTC", 0.001, Some(0.001))]),
            ],
        },
    ));
    assert_single_account_dirty(&events);
}
/// PR-EM: `availableToWithdraw` 在 UNIFIED 上已废弃；WS mapper 必须读
/// account-level `totalAvailableBalance`，不能把 coin 字段空值伪装成 0 可用。
#[test]
fn bybit_wallet_snapshot_uses_total_available_balance_for_stablecoin() {
    let events = map_bybit_event(bybit_ws_user::BybitUserEvent::Wallet(
        bybit_ws_user::BybitWalletUpdate {
            update_type: "snapshot".to_owned(),
            observed_at_ms: 1_700_000_000_000,
            accounts: vec![bybit_wallet_account_with_available(
                "UNIFIED",
                475.0,
                vec![
                    bybit_wallet_coin("USDT", 500.0, None),
                    bybit_wallet_coin("BTC", 1.0, Some(0.5)),
                ],
            )],
        },
    ));
    let snapshot = events.iter().find_map(|event| match event {
        PrivateWsEvent::Balances(snapshot) => Some(snapshot),
        _ => None,
    });
    assert_eq!(
        snapshot.map(|snapshot| snapshot.rows[0].available),
        Some(475.0)
    );
    assert_eq!(
        snapshot.map(|snapshot| snapshot.rows[1].available),
        Some(0.0)
    );
    assert_eq!(snapshot.map(|snapshot| snapshot.rows[0].total), Some(500.0));
}
/// PR-EM: UNIFIED wallet snapshot 同帧发出账户事实与 coin balance，来源和观测时间
/// 必须可追踪，不能把 account-level margin/equity 丢在 mapper 边界。
#[test]
fn bybit_wallet_snapshot_emits_account_summary_contract() {
    let events = map_bybit_event(bybit_ws_user::BybitUserEvent::Wallet(
        bybit_ws_user::BybitWalletUpdate {
            update_type: "snapshot".to_owned(),
            observed_at_ms: 1_700_000_000_000,
            accounts: vec![bybit_wallet_account_with_available(
                "UNIFIED",
                475.0,
                vec![bybit_wallet_coin("USDT", 500.0, Some(475.0))],
            )],
        },
    ));
    let summary = events.iter().find_map(|event| match event {
        PrivateWsEvent::AccountSummary(summary) => Some(summary),
        _ => None,
    });
    assert_eq!(summary.map(|row| row.venue.as_str()), Some("bybit"));
    assert_eq!(summary.map(|row| row.total_equity_usd), Some(1_000.0));
    assert_eq!(
        summary.map(|row| row.total_available_balance_usd),
        Some(475.0)
    );
    assert_eq!(summary.map(|row| row.total_initial_margin_usd), Some(40.0));
    assert_eq!(
        summary.map(|row| row.total_maintenance_margin_usd),
        Some(20.0)
    );
    assert_eq!(summary.map(|row| row.account_im_rate), Some(0.04));
    assert_eq!(summary.map(|row| row.account_mm_rate), Some(0.02));
    assert_eq!(
        summary.map(|row| row.observed_at_ms),
        Some(1_700_000_000_000)
    );
}
#[test]
fn bybit_execution_maps_to_private_fill_delta() {
    let events = map_bybit_event(bybit_ws_user::BybitUserEvent::Execution(vec![
        bybit_execution_delta(
            "9aac161b-8ed6-450d-9cab-c5cc67c21784",
            "exec-1",
            Some("USDT"),
        ),
        bybit_execution_delta("9aac161b-8ed6-450d-9cab-c5cc67c21784", "exec-2", None),
    ]));
    let fills: Vec<&PrivateFillDelta> = events
        .iter()
        .filter_map(|event| match event {
            PrivateWsEvent::Fill(fill) => Some(fill),
            _ => None,
        })
        .collect();

    assert_eq!(fills.len(), 2);
    assert_eq!(
        fills[0].exchange_order_id,
        "9aac161b-8ed6-450d-9cab-c5cc67c21784"
    );
    assert_eq!(fills[0].venue, "bybit");
    assert_eq!(fills[0].client_order_id.as_deref(), Some("cid-1"));
    assert_eq!(fills[0].symbol.as_deref(), Some("BTC"));
    assert_eq!(fills[0].side, Some(OrderSide::Sell));
    assert_eq!(
        fills[0].venue_event_id,
        "bybit_execution:9aac161b-8ed6-450d-9cab-c5cc67c21784:exec-1"
    );
    assert_eq!(fills[0].quantity, 0.5);
    assert_eq!(fills[0].price, 95_900.1);
    assert_eq!(fills[0].fee_amount, Some(26.3725275));
    assert_eq!(fills[0].fee_currency.as_deref(), Some("USDT"));
    assert_eq!(fills[0].occurred_at_ms, 1_746_270_400_353);
    assert_eq!(
        fills[1].venue_event_id,
        "bybit_execution:9aac161b-8ed6-450d-9cab-c5cc67c21784:exec-2"
    );
    assert_eq!(fills[1].fee_currency, None);
}
/// PR-DP-08 D-5: subscribe 后首推 `action=snapshot` + USDT-FUTURES 行 →
/// 全量替换 position cache（symbol+side 字段映射），省一次 REST 仓位拉取。
#[test]
fn bitget_position_snapshot_maps_to_position_snapshot() {
    let events = map_bitget_event(bitget_ws_user::BitgetUserEvent::Position(
        bitget_ws_user::BitgetPositionUpdate {
            action: "snapshot".to_owned(),
            positions: vec![bitget_position_delta("BTC", "USDT", 0.5, "long")],
        },
    ));
    let snapshot = events.iter().find_map(|event| match event {
        PrivateWsEvent::Positions(snapshot) => Some(snapshot),
        _ => None,
    });
    assert_eq!(snapshot.map(|snapshot| snapshot.rows.len()), Some(1));
    assert_eq!(
        snapshot.map(|snapshot| snapshot.rows[0].exchange.as_str()),
        Some("bitget")
    );
    assert_eq!(
        snapshot.map(|snapshot| snapshot.rows[0].symbol.as_str()),
        Some("BTC")
    );
    assert_eq!(
        snapshot.map(|snapshot| snapshot.rows[0].side.as_str()),
        Some("long")
    );
    assert_eq!(
        snapshot.map(|snapshot| snapshot.rows[0].quantity),
        Some(0.5)
    );
    assert_eq!(snapshot.map(|snapshot| snapshot.rows[0].margin), Some(50.0));
}
/// Bitget UTA documents `update` as an incremental symbol/side position row.
/// Merge it locally so periodic mark-price updates do not force full REST reads.
#[test]
fn bitget_position_update_maps_to_position_patch() {
    let events = map_bitget_event(bitget_ws_user::BitgetUserEvent::Position(
        bitget_ws_user::BitgetPositionUpdate {
            action: "update".to_owned(),
            positions: vec![bitget_position_delta("BTC", "USDT", 0.5, "long")],
        },
    ));
    let patch = events.iter().find_map(|event| match event {
        PrivateWsEvent::PositionPatch(patch) => Some(patch),
        _ => None,
    });
    assert_eq!(patch.map(|patch| patch.rows.len()), Some(1));
    assert_eq!(
        patch.map(|patch| patch.rows[0].symbol.as_str()),
        Some("BTC")
    );
    assert_eq!(patch.map(|patch| patch.rows[0].quantity), Some(0.5));
}

#[test]
fn bitget_zero_position_update_maps_to_removal_patch() {
    let events = map_bitget_event(bitget_ws_user::BitgetUserEvent::Position(
        bitget_ws_user::BitgetPositionUpdate {
            action: "update".to_owned(),
            positions: vec![bitget_position_delta("BTC", "USDT", 0.0, "long")],
        },
    ));
    let patch = events.iter().find_map(|event| match event {
        PrivateWsEvent::PositionPatch(patch) => Some(patch),
        _ => None,
    });
    assert_eq!(patch.map(|patch| patch.rows[0].quantity), Some(0.0));
}
/// PR-EN: UTA global position snapshot preserves USDC and COIN native rows;
/// REST now fans out the same three futures categories.
#[test]
fn bitget_position_snapshot_preserves_all_uta_margin_coins() {
    let events = map_bitget_event(bitget_ws_user::BitgetUserEvent::Position(
        bitget_ws_user::BitgetPositionUpdate {
            action: "snapshot".to_owned(),
            positions: vec![
                bitget_position_delta("BTC", "USDC", 1.0, "long"),
                bitget_position_delta("ETH", "BTC", 1.0, "long"),
            ],
        },
    ));
    let snapshot = events.iter().find_map(|event| match event {
        PrivateWsEvent::Positions(snapshot) => Some(snapshot),
        _ => None,
    });
    assert_eq!(snapshot.map(|snapshot| snapshot.rows.len()), Some(2));
    assert_eq!(
        snapshot.map(|snapshot| snapshot.rows[0].position_mode.as_deref()),
        Some(Some("hedge_mode"))
    );
    assert_eq!(
        snapshot.map(|snapshot| snapshot.rows[0].margin_mode.as_deref()),
        Some(Some("crossed"))
    );
}
