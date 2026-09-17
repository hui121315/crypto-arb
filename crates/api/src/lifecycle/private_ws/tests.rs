use super::binance::should_rotate_binance_stream;
use super::gate::map_gate_text;
use super::gate::{gate_private_subscribe_messages, gate_subscription_control, GateAccountDetail};
use super::kucoin::kucoin_private_subscribe_messages;
use super::kucoin_session::KucoinBulletResponse;
use super::transport::PrivateWsControl;
use super::*;

mod binance_health;
mod hyperliquid_subscriptions;
mod kucoin_finality;
mod projection;
mod projection_support;

#[test]
fn binance_rotation_uses_stream_max_age() {
    let started =
        common::time::now_ms() - (binance_ws_user::USER_STREAM_CONNECTION_MAX_SECS as i64 * 1_000);
    assert!(should_rotate_binance_stream(started));
}

#[test]
fn kucoin_private_bullet_response_builds_ws_url() -> Result<(), Box<dyn std::error::Error>> {
    let raw = r#"{
            "code":"200000",
            "data":{
                "token":"token-1",
                "instanceServers":[{
                    "endpoint":"wss://ws-api-futures.kucoin.com/",
                    "protocol":"websocket",
                    "pingInterval":18000
                }]
            }
        }"#;

    let session = serde_json::from_str::<KucoinBulletResponse>(raw)?.into_session()?;

    assert_eq!(
        session.ws_url,
        "wss://ws-api-futures.kucoin.com/?token=token-1"
    );
    assert_eq!(session.ping_interval_ms, 18_000);
    Ok(())
}

#[test]
fn gate_account_detail_extracts_official_user_id() -> Result<(), Box<dyn std::error::Error>> {
    let detail: GateAccountDetail = serde_json::from_str(
        r#"{
                "user_id":1667201533,
                "ip_whitelist":["127.0.0.1"],
                "key":{"mode":1},
                "tier":2
            }"#,
    )?;

    assert_eq!(detail.user_id()?, "1667201533");
    Ok(())
}

#[test]
fn gate_private_subscribe_messages_use_account_detail_user_id(
) -> Result<(), Box<dyn std::error::Error>> {
    let messages = gate_private_subscribe_messages(gate_ws_user::GateUserWsConfig {
        api_key: "key",
        api_secret: "secret",
        user_id: "1667201533",
        time_offset_secs: 0,
    })?;
    let payloads = messages
        .iter()
        .map(|message| -> Result<serde_json::Value, serde_json::Error> {
            serde_json::from_str(message)
        })
        .collect::<Result<Vec<_>, _>>()?;

    assert_eq!(payloads.len(), 4);
    assert!(payloads
        .iter()
        .all(|value| value["payload"][0] == "1667201533"));
    assert_eq!(payloads[0]["channel"], "futures.orders");
    assert_eq!(payloads[1]["channel"], "futures.positions");
    assert_eq!(payloads[2]["channel"], "futures.balances");
    assert_eq!(payloads[3]["channel"], "futures.usertrades");
    assert_eq!(payloads[0]["payload"][1], "!all");
    assert_eq!(payloads[1]["payload"][1], "!all");
    assert_eq!(payloads[2]["payload"].as_array().map(Vec::len), Some(1));
    assert_eq!(payloads[3]["payload"][1], "!all");
    Ok(())
}

#[test]
fn gate_subscription_success_is_server_ack_evidence() {
    let control = gate_subscription_control(
        r#"{
            "time":1545459681,
            "trace_id":"gate-trace-usertrades",
            "channel":"futures.usertrades",
            "event":"subscribe",
            "error":null,
            "result":{"status":"success"}
        }"#,
    );

    assert!(matches!(
        control,
        Some(PrivateWsControl::SubscribeAck {
            channel,
            expected: 4,
            request_id: Some(request_id),
        }) if channel == "futures.usertrades" && request_id == "gate-trace-usertrades"
    ));
}

#[test]
fn gate_auth_rejection_is_fatal_control_evidence() {
    let control = gate_subscription_control(
        r#"{
            "channel":"futures.orders",
            "event":"subscribe",
            "error":{"code":4,"message":"authentication fail"},
            "result":{"status":"fail"}
        }"#,
    );

    assert!(matches!(
        control,
        Some(PrivateWsControl::SubscribeRejected {
            channel,
            expected: 4,
            authentication_failed: true,
            error,
            ..
        }) if channel == "futures.orders"
            && error == "code=4; message=authentication fail"
    ));
}

#[test]
fn gate_subscription_without_success_status_fails_closed() {
    let control = gate_subscription_control(
        r#"{
            "channel":"futures.positions",
            "event":"subscribe",
            "error":null,
            "result":{}
        }"#,
    );

    assert!(matches!(
        control,
        Some(PrivateWsControl::SubscribeRejected {
            authentication_failed: false,
            ..
        })
    ));
}

#[test]
fn gate_fill_health_waits_for_identity_apply_and_durable_ack() {
    let parsed = map_gate_text(
        r#"{
            "channel":"futures.usertrades",
            "event":"update",
            "result":[{
                "id":"3335259",
                "order_id":"4872460",
                "contract":"BTC_USDT",
                "size":1,
                "price":"40000.4",
                "fee":0.0009290592,
                "point_fee":0,
                "create_time_ms":1628736848321
            }]
        }"#,
    );

    assert!(parsed.health_after_apply());
}

#[test]
fn kucoin_private_subscribe_messages_include_positions() -> Result<(), Box<dyn std::error::Error>> {
    let messages = kucoin_private_subscribe_messages()?;
    let topics = messages
        .iter()
        .map(|message| -> Result<String, serde_json::Error> {
            let value: serde_json::Value = serde_json::from_str(message)?;
            Ok(value["topic"].as_str().unwrap_or_default().to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;

    assert_eq!(
        topics,
        vec![
            "/contractMarket/tradeOrders",
            "/contractAccount/wallet",
            "/contract/positionAll",
        ]
    );
    Ok(())
}
