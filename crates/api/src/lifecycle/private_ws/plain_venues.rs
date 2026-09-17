use super::transport::{
    parse_failed, push_private_ws_payload, run_confirmed_private_ws, spawn_plain_private_ws,
    ws_config, PrivateWsControl, PrivateWsParse,
};
use super::*;

mod hyperliquid;

#[cfg(test)]
pub(super) use hyperliquid::hyperliquid_private_subscription_payloads;
pub(super) use hyperliquid::spawn_hyperliquid_private_ws;

const OKX_PRIVATE_HANDSHAKE_COUNT: usize = 4;
const BYBIT_PRIVATE_HANDSHAKE_COUNT: usize = 2;
const BITGET_PRIVATE_HANDSHAKE_COUNT: usize = 5;

pub(super) fn spawn_okx_private_ws(
    state: AppState,
    credentials: Option<(String, String, String)>,
) -> Option<JoinHandle<()>> {
    let (api_key, api_secret, passphrase) = credentials?;
    Some(tokio::spawn(run_confirmed_private_ws(
        state,
        "okx",
        ws_config(
            "okx",
            okx_ws_user::OKX_PRIVATE_WS_URL.to_owned(),
            Duration::from_secs(25),
            WsHeartbeat::Text("ping".to_owned()),
            WsInboundCodec::Plain,
            WsServerPing::None,
        ),
        move || {
            Ok(okx_private_connect_messages(
                &api_key,
                &api_secret,
                &passphrase,
            ))
        },
        map_okx_text,
    )))
}

pub(super) fn spawn_bybit_private_ws(
    state: AppState,
    credentials: Option<(String, String)>,
) -> Option<JoinHandle<()>> {
    let (api_key, api_secret) = credentials?;
    Some(tokio::spawn(run_confirmed_private_ws(
        state,
        "bybit",
        bybit_private_ws_config(),
        move || bybit_private_connect_messages(&api_key, &api_secret),
        map_bybit_text,
    )))
}

fn bybit_private_ws_config() -> WsConfig {
    ws_config(
        "bybit",
        bybit_ws_user::BYBIT_PRIVATE_WS_URL.to_owned(),
        Duration::from_secs(20),
        WsHeartbeat::Text(r#"{"op":"ping"}"#.to_owned()),
        WsInboundCodec::Plain,
        WsServerPing::None,
    )
}

pub(super) fn spawn_bitget_private_ws(
    state: AppState,
    credentials: Option<(String, String, String)>,
) -> Option<JoinHandle<()>> {
    let (api_key, api_secret, passphrase) = credentials?;
    Some(tokio::spawn(run_confirmed_private_ws(
        state,
        "bitget",
        ws_config(
            "bitget",
            bitget_ws_user::BITGET_PRIVATE_WS_URL.to_owned(),
            Duration::from_secs(25),
            WsHeartbeat::Text("ping".to_owned()),
            WsInboundCodec::Plain,
            WsServerPing::None,
        ),
        move || bitget_private_connect_messages(&api_key, &api_secret, &passphrase),
        map_bitget_text,
    )))
}

fn okx_private_connect_messages(api_key: &str, api_secret: &str, passphrase: &str) -> Vec<String> {
    vec![
        okx_ws_user::login_payload(okx_ws_user::OkxUserWsConfig {
            api_key,
            api_secret,
            passphrase,
        }),
        okx_ws_user::subscribe_account_payload("account", None),
        okx_ws_user::subscribe_positions_payload("positions"),
        okx_ws_user::subscribe_orders_payload("orders"),
    ]
}

fn bybit_private_connect_messages(api_key: &str, api_secret: &str) -> ExchangeResult<Vec<String>> {
    Ok(vec![
        bybit_ws_user::auth_payload(bybit_ws_user::BybitUserWsConfig {
            api_key,
            api_secret,
        })?,
        bybit_ws_user::subscribe_private_payload("private")?,
    ])
}

fn bitget_private_connect_messages(
    api_key: &str,
    api_secret: &str,
    passphrase: &str,
) -> ExchangeResult<Vec<String>> {
    let mut messages = Vec::with_capacity(BITGET_PRIVATE_HANDSHAKE_COUNT);
    messages.push(bitget_ws_user::login_payload(
        bitget_ws_user::BitgetUserWsConfig {
            api_key,
            api_secret,
            passphrase,
        },
    )?);
    for arg in [
        bitget_ws_user::SubscriptionArg::account(),
        bitget_ws_user::SubscriptionArg::order(),
        bitget_ws_user::SubscriptionArg::position(),
        bitget_ws_user::SubscriptionArg::fill(),
    ] {
        messages.push(bitget_ws_user::subscribe_payload(&[arg])?);
    }
    Ok(messages)
}
/// PR-DP-08 D-2/D-8：OKX 私有 WS 文本 → 事件分发。Account/Position/Order 全部
/// 分支由 `map_okx_event` 统一处理（D-8 后 venue 字符串硬编码在 mapper 内）。
fn map_okx_text(text: &str) -> PrivateWsParse {
    if text.trim() == "pong" {
        return PrivateWsParse::ignored();
    }
    match okx_subscription_control(text) {
        Ok(Some(control)) => return PrivateWsParse::control(control),
        Ok(None) => {}
        Err(error) => return parse_failed("okx", &error),
    }
    match okx_ws_user::parse_user_event(text) {
        Ok(Some(event)) => PrivateWsParse::events(
            crate::trading_service::private_ws_mapper::map_okx_event(event),
        ),
        Ok(None) => PrivateWsParse::ignored(),
        Err(error) => parse_failed("okx", &error),
    }
}

fn okx_subscription_control(text: &str) -> ExchangeResult<Option<PrivateWsControl>> {
    Ok(
        okx_ws_user::parse_user_control(text)?.map(|control| match control {
            okx_ws_user::OkxUserControl::Acknowledged {
                channel,
                request_id,
            } => PrivateWsControl::SubscribeAck {
                channel,
                expected: OKX_PRIVATE_HANDSHAKE_COUNT,
                request_id,
            },
            okx_ws_user::OkxUserControl::Rejected {
                channel,
                request_id,
                authentication_failed,
                error,
            } => PrivateWsControl::SubscribeRejected {
                channel,
                expected: OKX_PRIVATE_HANDSHAKE_COUNT,
                authentication_failed,
                error,
                request_id,
            },
        }),
    )
}

/// PR-DP-08 D-4/D-8：Bybit 私有 WS 文本 → 事件分发。Wallet/Position/Order 全部
/// 分支由 `map_bybit_event` 统一处理。
fn map_bybit_text(text: &str) -> PrivateWsParse {
    match bybit_subscription_control(text) {
        Ok(Some(control)) => return PrivateWsParse::control(control),
        Ok(None) => {}
        Err(error) => return parse_failed("bybit", &error),
    }
    match bybit_ws_user::parse_user_event(text) {
        Ok(Some(event)) => PrivateWsParse::events(
            crate::trading_service::private_ws_mapper::map_bybit_event(event),
        ),
        Ok(None) => PrivateWsParse::ignored(),
        Err(error) => parse_failed("bybit", &error),
    }
}

fn bybit_subscription_control(text: &str) -> ExchangeResult<Option<PrivateWsControl>> {
    Ok(
        bybit_ws_user::parse_user_control(text)?.map(|control| match control {
            bybit_ws_user::BybitUserControl::Acknowledged {
                channel,
                request_id,
            } => PrivateWsControl::SubscribeAck {
                channel,
                expected: BYBIT_PRIVATE_HANDSHAKE_COUNT,
                request_id,
            },
            bybit_ws_user::BybitUserControl::Rejected {
                channel,
                request_id,
                authentication_failed,
                error,
            } => PrivateWsControl::SubscribeRejected {
                channel,
                expected: BYBIT_PRIVATE_HANDSHAKE_COUNT,
                authentication_failed,
                error,
                request_id,
            },
        }),
    )
}

/// PR-DP-08 D-6/D-8：Bitget V3 / UTA 私有 WS 文本 → 事件分发。Account/Position/Order/Fill
/// 全部分支由 `map_bitget_event` 统一处理。
fn map_bitget_text(text: &str) -> PrivateWsParse {
    match bitget_subscription_control(text) {
        Ok(Some(control)) => return PrivateWsParse::control(control),
        Ok(None) => {}
        Err(error) => return parse_failed("bitget", &error),
    }
    match bitget_ws_user::parse_user_event(text) {
        Ok(Some(event)) => PrivateWsParse::events(
            crate::trading_service::private_ws_mapper::map_bitget_event(event),
        ),
        Ok(None) => PrivateWsParse::ignored(),
        Err(error) => parse_failed("bitget", &error),
    }
}

fn bitget_subscription_control(text: &str) -> ExchangeResult<Option<PrivateWsControl>> {
    Ok(
        bitget_ws_user::parse_user_control(text)?.map(|control| match control {
            bitget_ws_user::BitgetUserControl::Acknowledged {
                channel,
                request_id,
            } => PrivateWsControl::SubscribeAck {
                channel,
                expected: BITGET_PRIVATE_HANDSHAKE_COUNT,
                request_id,
            },
            bitget_ws_user::BitgetUserControl::Rejected {
                channel,
                request_id,
                authentication_failed,
                error,
            } => PrivateWsControl::SubscribeRejected {
                channel,
                expected: BITGET_PRIVATE_HANDSHAKE_COUNT,
                authentication_failed,
                error,
                request_id,
            },
        }),
    )
}
#[cfg(test)]
#[path = "plain_venues_tests.rs"]
mod tests;
