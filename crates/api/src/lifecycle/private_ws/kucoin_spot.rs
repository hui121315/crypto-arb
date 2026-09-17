use super::transport::{
    parse_failed, run_confirmed_private_ws, ws_config, PrivateWsControl, PrivateWsParse,
};
use super::*;

const KUCOIN_SPOT_HANDSHAKE_COUNT: usize = 1;

pub(super) fn spawn_kucoin_spot_private_ws(
    state: AppState,
    credentials: Option<(String, String, String)>,
) -> Option<JoinHandle<()>> {
    let (api_key, api_secret, passphrase) = credentials?;
    Some(tokio::spawn(async move {
        let health_state = state.clone();
        if let Err(error) = run_kucoin_spot_private_ws(state, api_key, api_secret, passphrase).await
        {
            health_state
                .private_ws_health()
                .record_disconnected("kucoin", &error.to_string());
            warn!(%error, "kucoin spot private ws stopped");
        }
    }))
}

async fn run_kucoin_spot_private_ws(
    state: AppState,
    api_key: String,
    api_secret: String,
    passphrase: String,
) -> ExchangeResult<()> {
    let session = super::kucoin_session::fetch_private_session(
        KUCOIN_SPOT_BASE,
        kucoin_spot_ws_user::KUCOIN_SPOT_PRIVATE_BULLET_PATH,
        "kucoin-spot-private-ws",
        &api_key,
        &api_secret,
        &passphrase,
    )
    .await?;
    run_confirmed_private_ws(
        state,
        "kucoin",
        ws_config(
            "kucoin-spot-private",
            session.ws_url,
            Duration::from_millis(session.ping_interval_ms),
            WsHeartbeat::Text(r#"{"id":"spot-ping","type":"ping"}"#.to_owned()),
            WsInboundCodec::Plain,
            WsServerPing::None,
        ),
        || {
            Ok(vec![kucoin_spot_ws_user::subscribe_orders_payload(
                "spot-orders",
            )?])
        },
        map_kucoin_spot_text,
    )
    .await;
    Ok(())
}

fn map_kucoin_spot_text(text: &str) -> PrivateWsParse {
    match kucoin_spot_ws_user::parse_message(text) {
        Ok(kucoin_spot_ws_user::KucoinSpotUserMessage::Control(control)) => {
            PrivateWsParse::control(match control {
                kucoin_spot_ws_user::KucoinSpotUserControl::Acknowledged { request_id } => {
                    PrivateWsControl::SubscribeAck {
                        channel: kucoin_spot_ws_user::KUCOIN_SPOT_ORDER_TOPIC.to_owned(),
                        expected: KUCOIN_SPOT_HANDSHAKE_COUNT,
                        request_id: Some(request_id),
                    }
                }
                kucoin_spot_ws_user::KucoinSpotUserControl::Rejected {
                    request_id,
                    authentication_failed,
                    error,
                } => PrivateWsControl::SubscribeRejected {
                    channel: kucoin_spot_ws_user::KUCOIN_SPOT_ORDER_TOPIC.to_owned(),
                    expected: KUCOIN_SPOT_HANDSHAKE_COUNT,
                    authentication_failed,
                    error,
                    request_id,
                },
            })
        }
        Ok(kucoin_spot_ws_user::KucoinSpotUserMessage::Event(event)) => {
            PrivateWsParse::events_after_apply(
                crate::trading_service::private_ws_mapper::map_kucoin_spot_event(event),
            )
        }
        Ok(kucoin_spot_ws_user::KucoinSpotUserMessage::Ignored) => PrivateWsParse::ignored(),
        Err(error) => parse_failed("kucoin", &error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spot_terminal_health_waits_for_durable_apply() {
        let text = r#"{
            "topic":"/spotMarket/tradeOrdersV2","type":"message","subject":"orderChange",
            "data":{"type":"filled","symbol":"SOL-USDT","side":"sell","orderType":"limit",
            "clientOid":"cid","orderId":"order-1","orderTime":1780000000000,
            "status":"done","originSize":"0.1","remainSize":"0","filledSize":"0.1",
            "price":"150","matchPrice":"150","matchSize":"0","tradeId":"",
            "ts":1780000000100000000}
        }"#;
        assert!(map_kucoin_spot_text(text).health_after_apply());
    }
}
