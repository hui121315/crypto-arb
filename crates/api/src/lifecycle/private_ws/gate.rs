use super::transport::{
    parse_failed, run_confirmed_private_ws, ws_config, PrivateWsControl, PrivateWsParse,
};
use super::*;

const GATE_PRIVATE_SUBSCRIPTION_COUNT: usize = 4;

pub(super) fn spawn_gate_private_ws(
    state: AppState,
    credentials: Option<(String, String)>,
) -> Option<JoinHandle<()>> {
    let (api_key, api_secret) = credentials?;
    let session = PrivateWsSession::capture(&state, "gate");
    Some(tokio::spawn(async move {
        let health_state = state.clone();
        if let Err(error) = run_gate_private_ws(state, session.clone(), api_key, api_secret).await {
            let Some(_account) = session.lock(&health_state).await else {
                return;
            };
            health_state
                .private_ws_health()
                .record_disconnected("gate", &error.to_string());
            warn!(%error, "gate private ws stopped");
        }
    }))
}

async fn run_gate_private_ws(
    state: AppState,
    session: PrivateWsSession,
    api_key: String,
    api_secret: String,
) -> ExchangeResult<()> {
    {
        let Some(_account) = session.lock(&state).await else {
            return Ok(());
        };
        state.private_ws_health().record_task_started("gate");
    }
    let user_id = match fetch_gate_user_id(&api_key, &api_secret).await {
        Ok(user_id) => user_id,
        Err(error) => {
            let Some(_account) = session.lock(&state).await else {
                return Ok(());
            };
            state
                .private_ws_health()
                .record_auth_failed("gate", &format!("account/detail: {error}"));
            warn!(%error, "gate private ws account identity authentication failed");
            return Ok(());
        }
    };
    run_confirmed_private_ws(
        state,
        session,
        "gate",
        ws_config(
            "gate",
            gate_ws_user::GATE_PRIVATE_WS_URL.to_owned(),
            Duration::from_secs(20),
            WsHeartbeat::PingFrame,
            WsInboundCodec::Plain,
            WsServerPing::None,
        ),
        move || {
            gate_private_subscribe_messages(gate_ws_user::GateUserWsConfig {
                api_key: &api_key,
                api_secret: &api_secret,
                user_id: &user_id,
                time_offset_secs: 0,
            })
        },
        map_gate_text,
    )
    .await;
    Ok(())
}

pub(super) fn gate_private_subscribe_messages(
    cfg: gate_ws_user::GateUserWsConfig<'_>,
) -> ExchangeResult<Vec<String>> {
    Ok(vec![
        gate_ws_user::subscribe_orders_payload(cfg, "!all")?,
        gate_ws_user::subscribe_positions_payload(cfg, "!all")?,
        gate_ws_user::subscribe_balances_payload(cfg)?,
        gate_ws_user::subscribe_usertrades_payload(cfg, "!all")?,
    ])
}

async fn fetch_gate_user_id(api_key: &str, api_secret: &str) -> ExchangeResult<String> {
    let timestamp = common::time::now_secs().to_string();
    let signature = exchange::signing::gate::sign(
        api_secret.as_bytes(),
        "GET",
        GATE_ACCOUNT_DETAIL_PATH,
        "",
        "",
        &timestamp,
    );
    let http = HttpClient::new("gate-private-ws")?;
    let url = format!("{GATE_API_BASE}{GATE_ACCOUNT_DETAIL_PATH}");
    let response = http
        .execute_with_retry(|| {
            http.request(Method::GET, &url)
                .header("KEY", api_key)
                .header("Timestamp", &timestamp)
                .header("SIGN", &signature)
        })
        .await?;
    let detail: GateAccountDetail = response
        .json()
        .await
        .map_err(|error| ExchangeError::Parse(format!("gate account/detail json: {error}")))?;
    detail.user_id()
}
pub(super) fn map_gate_text(text: &str) -> PrivateWsParse {
    if let Some(control) = gate_subscription_control(text) {
        return PrivateWsParse::control(control);
    }
    match gate_ws_user::parse_user_event(text) {
        Ok(Some(event)) => PrivateWsParse::events_after_apply(
            crate::trading_service::private_ws_mapper::map_gate_event(event),
        ),
        Ok(None) => PrivateWsParse::ignored(),
        Err(error) => parse_failed("gate", &error),
    }
}

pub(super) fn gate_subscription_control(text: &str) -> Option<PrivateWsControl> {
    let envelope = serde_json::from_str::<GateWsControlEnvelope>(text).ok()?;
    if envelope.event != "subscribe" || !gate_private_channel(&envelope.channel) {
        return None;
    }
    let request_id = envelope.request_id();
    match envelope.error {
        Some(error) => Some(PrivateWsControl::SubscribeRejected {
            channel: envelope.channel,
            expected: GATE_PRIVATE_SUBSCRIPTION_COUNT,
            authentication_failed: error.code == 4,
            error: format!("code={}; message={}", error.code, error.message),
            request_id,
        }),
        None if envelope.result.status.as_deref() == Some("success") => {
            Some(PrivateWsControl::SubscribeAck {
                channel: envelope.channel,
                expected: GATE_PRIVATE_SUBSCRIPTION_COUNT,
                request_id,
            })
        }
        None => Some(PrivateWsControl::SubscribeRejected {
            channel: envelope.channel,
            expected: GATE_PRIVATE_SUBSCRIPTION_COUNT,
            authentication_failed: false,
            error: "missing successful subscription status".to_owned(),
            request_id,
        }),
    }
}

fn gate_private_channel(channel: &str) -> bool {
    matches!(
        channel,
        "futures.orders" | "futures.positions" | "futures.balances" | "futures.usertrades"
    )
}

#[derive(Debug, Deserialize)]
struct GateWsControlEnvelope {
    #[serde(default)]
    id: Value,
    #[serde(default)]
    trace_id: String,
    #[serde(default)]
    channel: String,
    #[serde(default)]
    event: String,
    #[serde(default)]
    error: Option<GateWsControlError>,
    #[serde(default)]
    result: GateWsControlResult,
}

impl GateWsControlEnvelope {
    fn request_id(&self) -> Option<String> {
        let trace_id = self.trace_id.trim();
        if !trace_id.is_empty() {
            return Some(trace_id.to_owned());
        }
        match &self.id {
            Value::String(value) if !value.trim().is_empty() => Some(value.trim().to_owned()),
            Value::Number(value) => Some(value.to_string()),
            _ => None,
        }
    }
}

#[derive(Debug, Deserialize)]
struct GateWsControlError {
    code: i64,
    #[serde(default)]
    message: String,
}

#[derive(Debug, Default, Deserialize)]
struct GateWsControlResult {
    #[serde(default)]
    status: Option<String>,
}
#[derive(Debug, Deserialize)]
pub(super) struct GateAccountDetail {
    #[serde(default)]
    user_id: Value,
}

impl GateAccountDetail {
    pub(super) fn user_id(self) -> ExchangeResult<String> {
        let id = match self.user_id {
            Value::Number(number) => number.to_string(),
            Value::String(text) => text.trim().to_owned(),
            _ => String::new(),
        };
        if id.is_empty() {
            Err(ExchangeError::Parse(
                "gate account/detail missing user_id".into(),
            ))
        } else {
            Ok(id)
        }
    }
}
