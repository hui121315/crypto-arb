use super::*;
use pretty_assertions::assert_eq;

#[test]
fn login_payload_uses_official_uta_shape() {
    let payload = login_payload(BitgetUserWsConfig {
        api_key: "key",
        api_secret: "secret",
        passphrase: "pass",
    })
    .expect("login payload");
    let value: serde_json::Value = serde_json::from_str(&payload).expect("login json");
    assert_eq!(value["op"], "login");
    assert_eq!(value["args"][0]["apiKey"], "key");
    assert_eq!(value["args"][0]["passphrase"], "pass");
    assert!(value["args"][0]["timestamp"].as_str().is_some());
    assert!(value["args"][0]["sign"].as_str().is_some());
}

#[test]
fn private_subscriptions_are_all_uta_global_without_v2_wildcards() {
    let payload = subscribe_private_payload().expect("subscribe payload");
    let value: serde_json::Value = serde_json::from_str(&payload).expect("subscribe json");
    assert_eq!(value["op"], "subscribe");
    let args = value["args"].as_array().expect("args");
    assert_eq!(args.len(), 4);
    assert_eq!(
        args.iter()
            .map(|arg| arg["topic"].as_str().expect("topic"))
            .collect::<Vec<_>>(),
        vec!["account", "order", "position", "fill"]
    );
    for arg in args {
        assert_eq!(arg["instType"], "UTA");
        assert!(arg.get("symbol").is_none());
        assert!(arg.get("coin").is_none());
    }
}

#[test]
fn control_fixtures_confirm_login_and_subscription_and_reject_failures() {
    let login = include_str!("../../fixtures/bitget/uta_ws_login_success.json");
    assert_eq!(
        parse_user_control(login).expect("login control"),
        Some(BitgetUserControl::Acknowledged {
            channel: "login".to_owned(),
            request_id: None,
        })
    );

    let subscribe = include_str!("../../fixtures/bitget/uta_ws_subscribe_account_success.json");
    assert_eq!(
        parse_user_control(subscribe).expect("subscribe control"),
        Some(BitgetUserControl::Acknowledged {
            channel: "account".to_owned(),
            request_id: None,
        })
    );

    let login_failure = include_str!("../../fixtures/bitget/uta_ws_login_failure.json");
    let Some(BitgetUserControl::Rejected {
        authentication_failed,
        error,
        ..
    }) = parse_user_control(login_failure).expect("login failure control")
    else {
        panic!("expected login rejection");
    };
    assert!(authentication_failed);
    assert!(error.contains("30005"));

    let subscription_failure = include_str!("../../fixtures/bitget/uta_ws_subscribe_failure.json");
    let Some(BitgetUserControl::Rejected {
        channel,
        authentication_failed,
        ..
    }) = parse_user_control(subscription_failure).expect("subscription failure control")
    else {
        panic!("expected subscription rejection");
    };
    assert_eq!(channel, "position");
    assert!(!authentication_failed);
}

#[test]
fn heartbeat_pong_is_ignored_by_control_and_event_parsers() {
    assert_eq!(parse_user_control("pong").expect("heartbeat control"), None);
    assert_eq!(parse_user_event("pong").expect("heartbeat event"), None);
}

#[test]
fn account_fixture_preserves_aggregate_equity_and_nested_assets() {
    let fixture = include_str!("../../fixtures/bitget/uta_ws_account_snapshot.json");
    let Some(BitgetUserEvent::Account(update)) = parse_user_event(fixture).expect("account event")
    else {
        panic!("expected account event");
    };
    assert_eq!(update.action, "snapshot");
    assert_eq!(update.total_equity, 1250.5);
    assert_eq!(update.effective_equity, 1100.25);
    assert_eq!(update.initial_margin, 100.0);
    assert_eq!(update.maintenance_margin, 25.0);
    assert_eq!(update.margin_ratio, 0.08);
    assert_eq!(update.position_margin_ratio, 0.02);
    assert_eq!(update.accounts.len(), 2);
    assert_eq!(update.accounts[0].frozen, 100.0);
    assert_eq!(update.accounts[0].unrealized_pnl, 12.5);
    assert_eq!(update.accounts[1].coin, "USDC");
    assert_eq!(update.accounts[1].unrealized_pnl, 0.0);
}

#[test]
fn position_fixture_uses_official_v3_field_names_and_native_identity() {
    let fixture = include_str!("../../fixtures/bitget/uta_ws_position_snapshot.json");
    let Some(BitgetUserEvent::Position(update)) =
        parse_user_event(fixture).expect("position event")
    else {
        panic!("expected position event");
    };
    assert_eq!(update.positions.len(), 1);
    let row = &update.positions[0];
    assert_eq!(row.symbol, "BTCPERP");
    assert_eq!(row.margin_coin, "USDC");
    assert_eq!(row.size, 0.02);
    assert_eq!(row.entry_price, 50_000.0);
    assert_eq!(row.hold_mode, "hedge_mode");
    assert_eq!(row.side, "long");
    assert_eq!(row.position_status, "opening");
}

#[test]
fn position_status_accepts_official_uta_opening_and_ended_values() {
    let mut position: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/bitget/uta_ws_position_snapshot.json"
    ))
    .expect("position fixture");

    for status in ["opening", "ended"] {
        position["data"][0]["positionStatus"] = status.into();
        position["data"][0]["mmr"] = if status == "ended" { "" } else { "0.005" }.into();
        let Some(BitgetUserEvent::Position(update)) =
            parse_user_event(&position.to_string()).expect("official position status")
        else {
            panic!("expected position event");
        };
        assert_eq!(update.positions[0].position_status, status);
        assert_eq!(
            update.positions[0].maintenance_margin_rate,
            if status == "ended" { 0.0 } else { 0.005 }
        );
    }
}

#[test]
fn fill_rpi_flag_accepts_official_yes_and_no_values() {
    let mut fill: serde_json::Value =
        serde_json::from_str(include_str!("../../fixtures/bitget/uta_ws_fill.json"))
            .expect("fill fixture");

    for (raw, expected) in [("yes", true), ("no", false)] {
        fill["data"][0]["isRPI"] = raw.into();
        let Some(BitgetUserEvent::Fill(rows)) =
            parse_user_event(&fill.to_string()).expect("official isRPI value")
        else {
            panic!("expected fill event");
        };
        assert_eq!(rows[0].is_rpi, Some(expected));
    }
}

#[test]
fn empty_position_snapshot_is_preserved_as_valid_account_evidence() {
    let fixture = r#"{
        "arg":{"instType":"UTA","topic":"position"},
        "action":"snapshot",
        "data":[],
        "ts":1785335184756
    }"#;
    let Some(BitgetUserEvent::Position(update)) =
        parse_user_event(fixture).expect("empty position snapshot")
    else {
        panic!("expected empty position snapshot");
    };

    assert_eq!(update.action, "snapshot");
    assert!(update.positions.is_empty());
}

fn assert_filled_order_fixture() {
    let filled = include_str!("../../fixtures/bitget/uta_ws_order_filled.json");
    let Some(BitgetUserEvent::Order(rows)) = parse_user_event(filled).expect("filled order") else {
        panic!("expected order event");
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].live_state, LiveOrderState::Filled);
    assert_eq!(rows[0].category, "USDC-FUTURES");
    assert_eq!(rows[0].hold_mode, "hedge_mode");
    assert_eq!(rows[0].hold_side, "long");
    assert_eq!(rows[0].trade_side, "open");
    assert_eq!(rows[0].order.filled_quantity, 0.02);
    assert_eq!(rows[0].order.fees, -0.6);
}

fn assert_cancelled_order_fixture() {
    let cancelled = include_str!("../../fixtures/bitget/uta_ws_order_cancelled.json");
    let Some(BitgetUserEvent::Order(rows)) = parse_user_event(cancelled).expect("cancelled order")
    else {
        panic!("expected order event");
    };
    assert_eq!(rows[0].live_state, LiveOrderState::Cancelled);
    assert_eq!(rows[0].cancel_reason.as_deref(), Some("user_cancel"));
}

fn assert_fill_fixture() {
    let fill = include_str!("../../fixtures/bitget/uta_ws_fill.json");
    let Some(BitgetUserEvent::Fill(rows)) = parse_user_event(fill).expect("fill") else {
        panic!("expected fill event");
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].category, "USDC-FUTURES");
    assert_eq!(rows[0].exec_id, "exec-10001");
    assert_eq!(rows[0].value, 1000.0);
    assert_eq!(rows[0].fee, -0.6);
    assert_eq!(rows[0].fee_currency.as_deref(), Some("USDC"));
    assert_eq!(rows[0].is_rpi, Some(false));
}

#[test]
fn order_and_fill_fixtures_preserve_terminal_finality_and_fees() {
    assert_filled_order_fixture();
    assert_cancelled_order_fixture();
    assert_fill_fixture();
}

#[test]
fn private_parser_fails_closed_on_missing_or_unknown_contract_fields() {
    let mut order: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/bitget/uta_ws_order_filled.json"
    ))
    .expect("order fixture");
    order["data"][0]["orderStatus"] = "mystery".into();
    assert!(parse_user_event(&order.to_string()).is_err());

    let mut account: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/bitget/uta_ws_account_snapshot.json"
    ))
    .expect("account fixture");
    account["data"][0]
        .as_object_mut()
        .expect("account row")
        .remove("imr");
    assert!(parse_user_event(&account.to_string()).is_err());

    let mut position: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/bitget/uta_ws_position_snapshot.json"
    ))
    .expect("position fixture");
    position["data"][0]["positionStatus"] = "unknown".into();
    assert!(parse_user_event(&position.to_string()).is_err());

    let mut fill: serde_json::Value =
        serde_json::from_str(include_str!("../../fixtures/bitget/uta_ws_fill.json"))
            .expect("fill fixture");
    fill["data"][0]["isRPI"] = "unknown".into();
    assert!(parse_user_event(&fill.to_string()).is_err());
}
