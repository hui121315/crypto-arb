use super::*;
use pretty_assertions::assert_eq;
use serde_json::json;

const TEST_KEY: &str = "0101010101010101010101010101010101010101010101010101010101010101";

#[test]
fn trade_session_uses_official_json_ping_heartbeat() {
    assert_eq!(
        hyperliquid_trade_heartbeat(),
        WsHeartbeat::Text(r#"{"method":"ping"}"#.to_owned())
    );
    assert!(hyperliquid_trade_heartbeat_response(
        r#"{"channel":"pong"}"#
    ));
    assert!(!hyperliquid_trade_heartbeat_response(
        r#"{"channel":"post","data":{"id":1}}"#
    ));
}

#[test]
fn signed_post_request_matches_hyperliquid_ws_schema() {
    let request = signed_post_request(SignedPostParams {
        id: 7,
        private_key: TEST_KEY,
        action: json!({"type": "cancel", "cancels": [{"a": 0, "o": 123}]}),
        nonce: 1_700_000_000_000,
        network: HyperliquidNetwork::Testnet,
        vault_address: None,
        expires_after: None,
    })
    .expect("signed request");
    let value = serde_json::to_value(request).expect("json");

    assert_eq!(value["method"], "post");
    assert_eq!(value["id"], 7);
    assert_eq!(value["request"]["type"], "action");
    assert_eq!(value["request"]["payload"]["nonce"], 1_700_000_000_000_u64);
    assert_eq!(value["request"]["payload"]["action"]["type"], "cancel");
    assert!(value["request"]["payload"].get("vaultAddress").is_none());
    assert!(value["request"]["payload"].get("expiresAfter").is_none());
    let r = value["request"]["payload"]["signature"]["r"]
        .as_str()
        .unwrap();
    assert!(r.starts_with("0x") && (3..=66).contains(&r.len()));
}

#[test]
fn signed_post_order_request_matches_hyperliquid_ws_schema() {
    let request = signed_post_request(SignedPostParams {
        id: 256,
        private_key: TEST_KEY,
        action: json!({
            "type": "order",
            "orders": [
                {
                    "a": 4,
                    "b": true,
                    "p": "1100",
                    "s": "0.2",
                    "r": false,
                    "t": {"limit": {"tif": "Gtc"}}
                }
            ],
            "grouping": "na"
        }),
        nonce: 1_713_825_891_591,
        network: HyperliquidNetwork::Testnet,
        vault_address: Some("0x0123456789abcdef0123456789abcdef01234567"),
        expires_after: None,
    })
    .expect("signed request");
    let value = serde_json::to_value(request).expect("json");

    assert_eq!(value["method"], "post");
    assert_eq!(value["id"], 256);
    assert_eq!(value["request"]["type"], "action");
    assert_eq!(value["request"]["payload"]["action"]["type"], "order");
    assert_eq!(
        value["request"]["payload"]["action"]["orders"][0]["t"]["limit"]["tif"],
        "Gtc"
    );
    assert_eq!(
        value["request"]["payload"]["vaultAddress"],
        "0x0123456789abcdef0123456789abcdef01234567"
    );
    let r = value["request"]["payload"]["signature"]["r"]
        .as_str()
        .unwrap();
    assert!(r.starts_with("0x") && (3..=66).contains(&r.len()));
}

#[test]
fn signed_post_request_includes_vault_and_expires() {
    let action = json!({"type": "cancel", "cancels": [{"a": 0, "o": 123}]});
    let nonce = 1_700_000_000_000;
    let base = signed_post_request(SignedPostParams {
        id: 7,
        private_key: TEST_KEY,
        action: action.clone(),
        nonce,
        network: HyperliquidNetwork::Testnet,
        vault_address: None,
        expires_after: None,
    })
    .unwrap();
    let with_vault = signed_post_request(SignedPostParams {
        id: 7,
        private_key: TEST_KEY,
        action,
        nonce,
        network: HyperliquidNetwork::Testnet,
        vault_address: Some("0x0123456789abcdef0123456789abcdef01234567"),
        expires_after: Some(nonce + 5_000),
    })
    .unwrap();
    let base_v = serde_json::to_value(&base).unwrap();
    let v_v = serde_json::to_value(&with_vault).unwrap();
    assert_eq!(
        v_v["request"]["payload"]["vaultAddress"],
        "0x0123456789abcdef0123456789abcdef01234567"
    );
    assert_eq!(v_v["request"]["payload"]["expiresAfter"], nonce + 5_000);
    assert_ne!(
        base_v["request"]["payload"]["signature"]["r"],
        v_v["request"]["payload"]["signature"]["r"]
    );
}

#[test]
fn next_request_id_monotonic_and_unique() {
    let a = next_request_id();
    let b = next_request_id();
    let c = next_request_id();
    assert!(b > a, "expected monotonic: {a} -> {b}");
    assert!(c > b, "expected monotonic: {b} -> {c}");
    assert_ne!(a, b);
    assert_ne!(b, c);
}

#[test]
fn parses_resting_order_response() {
    let response = parse_post_response(
        r#"{"channel":"post","data":{"id":7,"response":{"type":"action","payload":{"status":"ok","response":{"type":"order","data":{"statuses":[{"resting":{"oid":12345}}]}}}}}}"#,
    )
    .expect("parse")
    .expect("post");

    assert!(response.matches_id(7));
    let row = response.into_result(7).expect("row");
    assert_eq!(row.order_id.as_deref(), Some("12345"));
    assert_eq!(row.message.as_deref(), Some("resting"));
    assert_eq!(row.native_request_id.as_deref(), Some("7"));
    assert_eq!(row.native_response_id.as_deref(), Some("7"));
}

#[test]
fn hyperliquid_place_order_ack_parses_official_fixture() {
    let value: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/hyperliquid/exchange_order_resting.json"
    ))
    .expect("fixture");
    assert_eq!(value["status"], "ok");

    let response = value.get("response").expect("response");
    let row = row_from_action_response(response).expect("row");
    assert_eq!(row.order_id.as_deref(), Some("77738308"));
    assert_eq!(row.message.as_deref(), Some("resting"));
}

#[test]
fn parses_cancel_success_response() {
    let response = parse_post_response(
        r#"{"channel":"post","data":{"id":8,"response":{"type":"action","payload":{"status":"ok","response":{"type":"cancel","data":{"statuses":["success"]}}}}}}"#,
    )
    .expect("parse")
    .expect("post");

    let row = response.into_result(8).expect("row");
    assert_eq!(row.order_id, None);
    assert_eq!(row.message.as_deref(), Some("success"));
    assert_eq!(row.native_request_id.as_deref(), Some("8"));
    assert_eq!(row.native_response_id.as_deref(), Some("8"));
}

#[test]
fn hyperliquid_ws_place_order_ack_parses_official_fixture() {
    let response = parse_post_response(include_str!(
        "../../fixtures/hyperliquid/ws_post_order_resting.json"
    ))
    .expect("parse")
    .expect("post");

    assert!(response.matches_id(256));
    let row = response.into_result(256).expect("row");
    assert_eq!(row.order_id.as_deref(), Some("88383"));
    assert_eq!(row.message.as_deref(), Some("resting"));
    assert_eq!(row.native_request_id.as_deref(), Some("256"));
    assert_eq!(row.native_response_id.as_deref(), Some("256"));
}

#[test]
fn hyperliquid_ws_cancel_order_ack_parses_official_fixture() {
    let response = parse_post_response(include_str!(
        "../../fixtures/hyperliquid/ws_post_cancel_success.json"
    ))
    .expect("parse")
    .expect("post");

    assert!(response.matches_id(257));
    let row = response.into_result(257).expect("row");
    assert_eq!(row.order_id, None);
    assert_eq!(row.message.as_deref(), Some("success"));
    assert_eq!(row.native_request_id.as_deref(), Some("257"));
    assert_eq!(row.native_response_id.as_deref(), Some("257"));
}

#[test]
fn ack_records_venue_client_order_id() {
    let row = OrderAckRow {
        order_id: Some("12345".into()),
        message: Some("resting".into()),
        native_request_id: Some("req-42".into()),
        native_response_id: Some("resp-42".into()),
    };
    let ack = ack_from_result(
        "internal-1".into(),
        "public-cid".into(),
        "0xeb9a1d290f7f8020d7658c7985e7223c".into(),
        row,
        LiveOrderState::Accepted,
        None,
    );

    assert_eq!(ack.internal_order_id, "internal-1");
    assert_eq!(ack.exchange_order_id.as_deref(), Some("12345"));
    assert_eq!(ack.client_order_id, "public-cid");
    assert_eq!(
        ack.identity_update.public_client_order_id.as_deref(),
        Some("public-cid")
    );
    assert_eq!(
        ack.identity_update.venue_client_order_id.as_deref(),
        Some("0xeb9a1d290f7f8020d7658c7985e7223c")
    );
    assert_eq!(
        ack.identity_update.exchange_order_id.as_deref(),
        Some("12345")
    );
    assert_eq!(
        ack.identity_update
            .transport_metadata
            .native_transport
            .as_deref(),
        Some("hyperliquid_ws_post")
    );
    assert_eq!(
        ack.identity_update
            .transport_metadata
            .native_request_id
            .as_deref(),
        Some("req-42")
    );
    assert_eq!(
        ack.identity_update
            .transport_metadata
            .native_response_id
            .as_deref(),
        Some("resp-42")
    );
}

#[test]
fn ignores_non_post_messages() {
    let parsed = parse_post_response(r#"{"channel":"pong"}"#).expect("parse");
    assert!(parsed.is_none());
}

#[test]
fn next_nonce_is_strictly_monotonic_per_signer() {
    let key = signer_nonce_key(HyperliquidNetwork::Testnet, TEST_KEY);
    let a = next_nonce(&key);
    let b = next_nonce(&key);
    let c = next_nonce(&key);
    assert!(b > a, "expected strictly increasing nonce: {a} -> {b}");
    assert!(c > b, "expected strictly increasing nonce: {b} -> {c}");
}

#[test]
fn next_nonce_never_collides_within_same_millisecond() {
    // 同一 signer 在同一毫秒内连续分配也必须各不相同：1000 次远超单毫秒可由
    // now_ms() 提供的取值，因此能逼出 prev+1 递增路径，验证不撞车、不回退。
    let key = signer_nonce_key(HyperliquidNetwork::Mainnet, TEST_KEY);
    let mut seen = std::collections::HashSet::new();
    let mut prev = 0_u64;
    for _ in 0..1_000 {
        let n = next_nonce(&key);
        assert!(n > prev, "expected monotonic: {prev} -> {n}");
        assert!(seen.insert(n), "duplicate nonce {n}");
        prev = n;
    }
}

#[test]
fn signer_nonce_key_distinguishes_network_and_is_signer_scoped() {
    let base = signer_nonce_key(HyperliquidNetwork::Mainnet, TEST_KEY);
    let testnet = signer_nonce_key(HyperliquidNetwork::Testnet, TEST_KEY);
    assert_ne!(base, testnet, "network must scope the nonce sequence");
    assert!(base.contains("0x1a642f0e3c3af545e7acbd38b07251b3990914f1"));
    assert!(!base.contains(TEST_KEY));
}

#[test]
fn signer_session_health_records_scope_nonce_and_last_error() {
    let cfg = WsTradeConfig {
        url: "wss://example.invalid/ws",
        account_address: "0x1111111111111111111111111111111111111111",
        private_key: TEST_KEY,
        timeout_secs: 1,
        network: HyperliquidNetwork::Testnet,
        vault_address: Some("0x2222222222222222222222222222222222222222"),
        action_expires_after_ms: None,
    };
    let error = ExchangeError::WsClosed("fixture writer stopped".into());
    record_signer_session_result(cfg, 42, Some(&error));

    let failed = hyperliquid_signer_session_health()
        .into_iter()
        .find(|row| row.account_address == cfg.account_address && row.last_nonce == 42)
        .expect("session health");
    assert_eq!(failed.vault_address.as_deref(), cfg.vault_address);
    assert!(failed
        .last_error
        .as_deref()
        .is_some_and(|message| message.contains("fixture writer stopped")));

    record_signer_session_result(cfg, 43, None);
    let recovered = hyperliquid_signer_session_health()
        .into_iter()
        .find(|row| row.account_address == cfg.account_address && row.last_nonce == 43)
        .expect("recovered session health");
    assert!(recovered.last_error.is_none());
}

#[test]
fn signer_nonce_contract_declares_single_process_api_wallet_ownership() {
    assert_eq!(
        SIGNER_OWNERSHIP_BOUNDARY,
        "one_api_wallet_per_trading_process"
    );
}

// PR-EQ: official Hyperliquid exchange-action place-order response fixtures for
// the finality-critical paths the existing resting/cancel tests skip. The action
// payload carries status="ok"/"err" and a statuses[] array; a top-level status
// != "ok" and a per-item {error:...} are exchange rejections that must fail
// closed (never a silent ack), while a filled status must surface the oid.
#[test]
fn parses_filled_order_response() {
    let response = parse_post_response(
        r#"{"channel":"post","data":{"id":9,"response":{"type":"action","payload":{"status":"ok","response":{"type":"order","data":{"statuses":[{"filled":{"oid":99887766,"totalSz":"1","avgPx":"30000"}}]}}}}}}"#,
    )
    .expect("parse")
    .expect("post");
    let row = response.into_result(9).expect("row");
    assert_eq!(row.order_id.as_deref(), Some("99887766"));
    assert_eq!(row.message.as_deref(), Some("filled"));
    assert_eq!(row.native_request_id.as_deref(), Some("9"));
    assert_eq!(row.native_response_id.as_deref(), Some("9"));
}

#[test]
fn filled_post_response_remains_a_non_final_transport_ack() {
    let row = OrderAckRow {
        order_id: Some("99887766".into()),
        message: Some("filled".into()),
        native_request_id: Some("9".into()),
        native_response_id: Some("9".into()),
    };
    let ack = ack_from_result(
        "internal-1".into(),
        "public-cid".into(),
        "0x01010101010101010101010101010101".into(),
        row,
        LiveOrderState::Accepted,
        None,
    );

    assert_eq!(ack.state, LiveOrderState::Accepted);
    assert_eq!(ack.exchange_order_id.as_deref(), Some("99887766"));
    assert_eq!(ack.filled_quantity, None);
    assert_eq!(ack.filled_price, None);
    assert_eq!(ack.filled_fee, None);
}

#[test]
fn rejects_per_item_error_status_fails_closed() {
    let response = parse_post_response(
        r#"{"channel":"post","data":{"id":10,"response":{"type":"action","payload":{"status":"ok","response":{"type":"order","data":{"statuses":[{"error":"Order could not immediately match against any resting orders."}]}}}}}}"#,
    )
    .expect("parse")
    .expect("post");
    let Err(error) = response.into_result(10) else {
        panic!("per-item error must fail closed, never a silent ack");
    };
    assert!(
        error.to_string().contains("could not immediately match"),
        "error should carry the venue reason: {error}"
    );
}

#[test]
fn rejects_non_ok_action_status_fails_closed() {
    let response = parse_post_response(
        r#"{"channel":"post","data":{"id":11,"response":{"type":"action","payload":{"status":"err","response":"Insufficient margin to place order."}}}}"#,
    )
    .expect("parse")
    .expect("post");
    let Err(error) = response.into_result(11) else {
        panic!("non-ok action status must fail closed");
    };
    assert!(
        error.to_string().contains("Insufficient margin"),
        "error should carry the venue reason: {error}"
    );
}
