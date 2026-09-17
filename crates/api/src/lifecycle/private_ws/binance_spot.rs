use super::transport::{
    parse_failed, run_confirmed_private_ws, ws_config, PrivateWsControl, PrivateWsParse,
};
use super::*;

const BINANCE_SPOT_HANDSHAKE_COUNT: usize = 1;

pub(super) fn spawn_binance_spot_user_stream(
    state: AppState,
    credentials: Option<(String, String)>,
) -> Option<JoinHandle<()>> {
    let (api_key, api_secret) = credentials?;
    Some(tokio::spawn(async move {
        run_confirmed_private_ws(
            state,
            "binance",
            ws_config(
                "binance-spot-private",
                binance_spot_ws_user::BINANCE_SPOT_WS_API_URL.to_owned(),
                Duration::from_secs(20),
                WsHeartbeat::PingFrame,
                WsInboundCodec::Plain,
                WsServerPing::None,
            ),
            move || {
                Ok(vec![binance_spot_ws_user::subscription_payload(
                    &api_key,
                    &api_secret,
                    "crossline-spot-user",
                )?])
            },
            map_binance_spot_text,
        )
        .await;
    }))
}

fn map_binance_spot_text(text: &str) -> PrivateWsParse {
    match binance_spot_ws_user::parse_message(text) {
        Ok(binance_spot_ws_user::BinanceSpotUserMessage::Control(control)) => {
            PrivateWsParse::control(match control {
                binance_spot_ws_user::BinanceSpotUserControl::Acknowledged { request_id } => {
                    PrivateWsControl::SubscribeAck {
                        channel: "spot_user_data".to_owned(),
                        expected: BINANCE_SPOT_HANDSHAKE_COUNT,
                        request_id: Some(request_id),
                    }
                }
                binance_spot_ws_user::BinanceSpotUserControl::Rejected {
                    request_id,
                    authentication_failed,
                    error,
                } => PrivateWsControl::SubscribeRejected {
                    channel: "spot_user_data".to_owned(),
                    expected: BINANCE_SPOT_HANDSHAKE_COUNT,
                    authentication_failed,
                    error,
                    request_id: Some(request_id),
                },
            })
        }
        Ok(binance_spot_ws_user::BinanceSpotUserMessage::Event(event)) => {
            PrivateWsParse::events_after_apply(
                crate::trading_service::private_ws_mapper::map_binance_event(event),
            )
        }
        Ok(binance_spot_ws_user::BinanceSpotUserMessage::Ignored) => PrivateWsParse::ignored(),
        Err(error) => parse_failed("binance", &error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_spot_terminal_to_private_order_event() {
        let text = r#"{"event":{"e":"executionReport","E":1780000000100,"T":1780000000090,"s":"SOLUSDT","c":"cid","S":"SELL","o":"MARKET","f":"GTC","q":"0.1","p":"0","x":"TRADE","X":"FILLED","r":"NONE","i":1,"l":"0.1","z":"0.1","L":"150","Z":"15","n":"0","N":"USDT","t":2,"O":1780000000000}}"#;
        assert!(map_binance_spot_text(text).health_after_apply());
    }
}
