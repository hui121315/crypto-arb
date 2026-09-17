use super::*;

#[test]
fn okx_private_ws_waits_for_all_server_acknowledgements() {
    for (body, expected_channel, expected_request_id) in [
        (
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../exchange/fixtures/okx/ws_user_login_success.json"
            )),
            "login",
            None,
        ),
        (
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../exchange/fixtures/okx/ws_user_subscribe_account_success.json"
            )),
            "account",
            Some("account"),
        ),
        (
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../exchange/fixtures/okx/ws_user_subscribe_positions_success.json"
            )),
            "positions",
            Some("positions"),
        ),
        (
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../exchange/fixtures/okx/ws_user_subscribe_orders_success.json"
            )),
            "orders",
            Some("orders"),
        ),
    ] {
        assert!(matches!(
            okx_subscription_control(body),
            Ok(Some(PrivateWsControl::SubscribeAck {
                channel,
                expected: OKX_PRIVATE_HANDSHAKE_COUNT,
                request_id,
            })) if channel == expected_channel && request_id.as_deref() == expected_request_id
        ));
    }
}

#[test]
fn okx_login_and_subscription_rejections_fail_closed() {
    assert!(matches!(
        okx_subscription_control(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../exchange/fixtures/okx/ws_user_login_failure.json"
        ))),
        Ok(Some(PrivateWsControl::SubscribeRejected {
            channel,
            expected: OKX_PRIVATE_HANDSHAKE_COUNT,
            authentication_failed: true,
            error,
            ..
        })) if channel == "login" && error.contains("60009")
    ));

    assert!(matches!(
        okx_subscription_control(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../exchange/fixtures/okx/ws_user_subscribe_error.json"
        ))),
        Ok(Some(PrivateWsControl::SubscribeRejected {
            channel,
            expected: OKX_PRIVATE_HANDSHAKE_COUNT,
            authentication_failed: false,
            request_id: Some(request_id),
            error,
        })) if channel == "account" && request_id == "account" && error.contains("60012")
    ));
}

#[test]
fn okx_data_frame_is_not_misclassified_as_control() {
    let body = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../exchange/fixtures/okx/ws_user_orders_partial_fill.json"
    ));
    assert!(matches!(okx_subscription_control(body), Ok(None)));
    assert!(matches!(okx_ws_user::parse_user_event(body), Ok(Some(_))));
}

#[test]
fn okx_plain_pong_is_a_healthy_heartbeat_frame() {
    let parsed = map_okx_text("pong");

    assert!(parsed.is_ignored());
}

#[test]
fn bybit_private_ws_waits_for_auth_and_subscription_acknowledgements() {
    for (body, expected_channel) in [
        (
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../exchange/fixtures/bybit/ws_user_auth_success.json"
            )),
            "auth",
        ),
        (
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../exchange/fixtures/bybit/ws_user_subscribe_success.json"
            )),
            "subscribe",
        ),
    ] {
        assert!(matches!(
            bybit_subscription_control(body),
            Ok(Some(PrivateWsControl::SubscribeAck {
                channel,
                expected: BYBIT_PRIVATE_HANDSHAKE_COUNT,
                ..
            })) if channel == expected_channel
        ));
    }
}

#[test]
fn bybit_auth_and_subscription_rejections_fail_closed() {
    assert!(matches!(
        bybit_subscription_control(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../exchange/fixtures/bybit/ws_user_auth_failure.json"
        ))),
        Ok(Some(PrivateWsControl::SubscribeRejected {
            channel,
            expected: BYBIT_PRIVATE_HANDSHAKE_COUNT,
            authentication_failed: true,
            error,
            ..
        })) if channel == "auth" && error.contains("API key")
    ));

    assert!(matches!(
        bybit_subscription_control(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../exchange/fixtures/bybit/ws_user_subscribe_failure.json"
        ))),
        Ok(Some(PrivateWsControl::SubscribeRejected {
            channel,
            expected: BYBIT_PRIVATE_HANDSHAKE_COUNT,
            authentication_failed: false,
            error,
            ..
        })) if channel == "subscribe" && error.contains("handler not found")
    ));
}

#[test]
fn bybit_data_frame_is_not_misclassified_as_control() {
    let body = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../exchange/fixtures/bybit/ws_user_wallet_snapshot.json"
    ));
    assert!(matches!(bybit_subscription_control(body), Ok(None)));
    assert!(matches!(bybit_ws_user::parse_user_event(body), Ok(Some(_))));
}

#[test]
fn bybit_private_ws_uses_official_json_heartbeat() {
    let config = bybit_private_ws_config();

    assert_eq!(config.heartbeat_interval, Duration::from_secs(20));
    assert_eq!(
        config.heartbeat,
        WsHeartbeat::Text(r#"{"op":"ping"}"#.to_owned())
    );
}

#[test]
fn bitget_private_ws_waits_for_login_and_all_topic_acknowledgements() {
    for (body, expected_channel) in [
        (
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../exchange/fixtures/bitget/uta_ws_login_success.json"
            )),
            "login",
        ),
        (
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../exchange/fixtures/bitget/uta_ws_subscribe_account_success.json"
            )),
            "account",
        ),
    ] {
        assert!(matches!(
            bitget_subscription_control(body),
            Ok(Some(PrivateWsControl::SubscribeAck {
                channel,
                expected: BITGET_PRIVATE_HANDSHAKE_COUNT,
                ..
            })) if channel == expected_channel
        ));
    }
}

#[test]
fn bitget_login_and_subscription_rejections_fail_closed() {
    assert!(matches!(
        bitget_subscription_control(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../exchange/fixtures/bitget/uta_ws_login_failure.json"
        ))),
        Ok(Some(PrivateWsControl::SubscribeRejected {
            expected: BITGET_PRIVATE_HANDSHAKE_COUNT,
            authentication_failed: true,
            error,
            ..
        })) if error.contains("30005")
    ));
    assert!(matches!(
        bitget_subscription_control(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../exchange/fixtures/bitget/uta_ws_subscribe_failure.json"
        ))),
        Ok(Some(PrivateWsControl::SubscribeRejected {
            channel,
            expected: BITGET_PRIVATE_HANDSHAKE_COUNT,
            authentication_failed: false,
            ..
        })) if channel == "position"
    ));
}

#[test]
fn bitget_data_frame_is_not_misclassified_as_control() {
    let body = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../exchange/fixtures/bitget/uta_ws_order_filled.json"
    ));
    assert!(matches!(bitget_subscription_control(body), Ok(None)));
    assert!(matches!(
        bitget_ws_user::parse_user_event(body),
        Ok(Some(_))
    ));
}
