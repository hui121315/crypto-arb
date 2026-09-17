use super::transport::{
    parse_failed, run_confirmed_private_ws, ws_config, PrivateWsControl, PrivateWsParse,
};
use super::*;

const GATE_SPOT_HANDSHAKE_COUNT: usize = 1;

pub(super) fn spawn_gate_spot_private_ws(
    state: AppState,
    credentials: Option<(String, String)>,
) -> Option<JoinHandle<()>> {
    let (api_key, api_secret) = credentials?;
    Some(tokio::spawn(run_confirmed_private_ws(
        state,
        "gate",
        ws_config(
            "gate-spot-private",
            gate_spot_ws_user::GATE_SPOT_PRIVATE_WS_URL.to_owned(),
            Duration::from_secs(20),
            WsHeartbeat::PingFrame,
            WsInboundCodec::Plain,
            WsServerPing::None,
        ),
        move || {
            Ok(vec![gate_spot_ws_user::subscribe_orders_payload(
                &api_key,
                &api_secret,
            )?])
        },
        map_gate_spot_text,
    )))
}

fn map_gate_spot_text(text: &str) -> PrivateWsParse {
    match gate_spot_ws_user::parse_message(text) {
        Ok(gate_spot_ws_user::GateSpotUserMessage::Control(control)) => {
            PrivateWsParse::control(match control {
                gate_spot_ws_user::GateSpotUserControl::Acknowledged { request_id } => {
                    PrivateWsControl::SubscribeAck {
                        channel: gate_spot_ws_user::GATE_SPOT_ORDER_CHANNEL.to_owned(),
                        expected: GATE_SPOT_HANDSHAKE_COUNT,
                        request_id,
                    }
                }
                gate_spot_ws_user::GateSpotUserControl::Rejected {
                    request_id,
                    authentication_failed,
                    error,
                } => PrivateWsControl::SubscribeRejected {
                    channel: gate_spot_ws_user::GATE_SPOT_ORDER_CHANNEL.to_owned(),
                    expected: GATE_SPOT_HANDSHAKE_COUNT,
                    authentication_failed,
                    error,
                    request_id,
                },
            })
        }
        Ok(gate_spot_ws_user::GateSpotUserMessage::Orders(rows)) => {
            PrivateWsParse::events_after_apply(
                crate::trading_service::private_ws_mapper::map_gate_spot_orders(rows),
            )
        }
        Ok(gate_spot_ws_user::GateSpotUserMessage::Ignored) => PrivateWsParse::ignored(),
        Err(error) => parse_failed("gate", &error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spot_terminal_health_waits_for_durable_apply() {
        let text = r#"{
            "time_ms":1780000000100,"channel":"spot.orders","event":"update",
            "result":[{"id":"1","text":"t-cid","create_time_ms":"1780000000000",
            "update_time_ms":"1780000000100","currency_pair":"SOL_USDT","type":"limit",
            "side":"sell","amount":"0.1","price":"150","time_in_force":"gtc",
            "left":"0","avg_deal_price":"150","fee":"0","event":"finish",
            "finish_as":"filled"}]
        }"#;
        assert!(map_gate_spot_text(text).health_after_apply());
    }
}
