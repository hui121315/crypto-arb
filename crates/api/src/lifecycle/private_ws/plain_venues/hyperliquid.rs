use super::*;

/// PR-DP-08 D-7/D-8: Hyperliquid private WS text to event dispatch.
fn map_hyperliquid_text(text: &str) -> PrivateWsParse {
    match hyperliquid_ws_user::parse_user_event(text) {
        Ok(Some(event)) => PrivateWsParse::events(
            crate::trading_service::private_ws_mapper::map_hyperliquid_event(event),
        ),
        Ok(None) => PrivateWsParse::ignored(),
        Err(error) => parse_failed("hyperliquid", &error),
    }
}

pub(crate) fn spawn_hyperliquid_private_ws(
    state: AppState,
    credentials: Option<crate::trading_service::HyperliquidAdapterCredentials>,
) -> Option<JoinHandle<()>> {
    let credentials = credentials?;
    let user = hyperliquid_private_subscription_user(&credentials).to_owned();
    let mut messages = Vec::with_capacity(HYPERLIQUID_PRIVATE_WS_SUBSCRIPTIONS);
    let requested = HYPERLIQUID_PRIVATE_WS_SUBSCRIPTIONS;
    for (label, payload) in hyperliquid_private_subscription_payloads(&user) {
        if !push_private_ws_payload(
            &state,
            "hyperliquid",
            &mut messages,
            requested,
            label,
            payload,
        ) {
            return None;
        }
    }
    let _ = credentials.private_key;
    Some(spawn_plain_private_ws(
        state,
        "hyperliquid",
        ws_config(
            "hyperliquid",
            hyperliquid_ws_user::HYPERLIQUID_WS_URL.to_owned(),
            Duration::from_secs(30),
            WsHeartbeat::PingFrame,
            WsInboundCodec::Plain,
            WsServerPing::None,
        ),
        move || Ok(messages.clone()),
        map_hyperliquid_text,
    ))
}

fn hyperliquid_private_subscription_user(
    credentials: &crate::trading_service::HyperliquidAdapterCredentials,
) -> &str {
    credentials
        .vault_address
        .as_deref()
        .unwrap_or(&credentials.account_address)
}

pub(crate) fn hyperliquid_private_subscription_payloads(
    user: &str,
) -> [(&'static str, ExchangeResult<String>); HYPERLIQUID_PRIVATE_WS_SUBSCRIPTIONS] {
    [
        (
            "order_updates",
            hyperliquid_ws_user::subscribe_order_updates_payload(user),
        ),
        (
            "open_orders_core",
            hyperliquid_ws_user::subscribe_open_orders_payload(user, Some("")),
        ),
        (
            "open_orders_xyz",
            hyperliquid_ws_user::subscribe_open_orders_payload(user, Some("xyz")),
        ),
        (
            "open_orders_cash",
            hyperliquid_ws_user::subscribe_open_orders_payload(user, Some("cash")),
        ),
        (
            "open_orders_flx",
            hyperliquid_ws_user::subscribe_open_orders_payload(user, Some("flx")),
        ),
        (
            "open_orders_km",
            hyperliquid_ws_user::subscribe_open_orders_payload(user, Some("km")),
        ),
        (
            "open_orders_vntl",
            hyperliquid_ws_user::subscribe_open_orders_payload(user, Some("vntl")),
        ),
        (
            "user_events",
            hyperliquid_ws_user::subscribe_user_events_payload(user),
        ),
        (
            "user_fills",
            hyperliquid_ws_user::subscribe_user_fills_payload(user, false),
        ),
        (
            "user_fundings",
            hyperliquid_ws_user::subscribe_user_fundings_payload(user),
        ),
        (
            "clearinghouse",
            hyperliquid_ws_user::subscribe_clearinghouse_payload(user, None),
        ),
        (
            "all_dexs_clearinghouse",
            hyperliquid_ws_user::subscribe_all_dexs_clearinghouse_payload(user),
        ),
        (
            "spot_state",
            hyperliquid_ws_user::subscribe_spot_state_payload(user, None),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_subscriptions_use_vault_scope_when_configured() {
        let credentials = crate::trading_service::HyperliquidAdapterCredentials {
            account_address: "0xmain".into(),
            private_key: "agent-key".into(),
            vault_address: Some("0xvault".into()),
        };

        assert_eq!(
            hyperliquid_private_subscription_user(&credentials),
            "0xvault"
        );
    }

    #[test]
    fn private_subscriptions_default_to_main_account_scope() {
        let credentials = crate::trading_service::HyperliquidAdapterCredentials {
            account_address: "0xmain".into(),
            private_key: "agent-key".into(),
            vault_address: None,
        };

        assert_eq!(
            hyperliquid_private_subscription_user(&credentials),
            "0xmain"
        );
    }
}
