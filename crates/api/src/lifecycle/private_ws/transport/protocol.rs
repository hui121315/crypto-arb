use super::super::*;

pub(in super::super) type PrivateWsEvents =
    Vec<crate::trading_service::private_ws_events::PrivateWsEvent>;

#[derive(Debug)]
pub(in super::super) struct PrivateWsParse {
    pub(super) events: Option<PrivateWsEvents>,
    pub(super) error: Option<String>,
    pub(super) control: Option<PrivateWsControl>,
    pub(super) health_after_apply: bool,
}

#[derive(Debug)]
pub(in super::super) enum PrivateWsControl {
    SubscribeAck {
        channel: String,
        expected: usize,
        request_id: Option<String>,
    },
    SubscribeRejected {
        channel: String,
        expected: usize,
        authentication_failed: bool,
        error: String,
        request_id: Option<String>,
    },
}

impl PrivateWsParse {
    pub(in super::super) fn events(events: PrivateWsEvents) -> Self {
        Self {
            events: Some(events),
            error: None,
            control: None,
            health_after_apply: false,
        }
    }

    pub(in super::super) fn events_after_apply(events: PrivateWsEvents) -> Self {
        Self {
            events: Some(events),
            error: None,
            control: None,
            health_after_apply: true,
        }
    }

    pub(in super::super) fn ignored() -> Self {
        Self {
            events: None,
            error: None,
            control: None,
            health_after_apply: false,
        }
    }

    pub(in super::super) fn failed(error: String) -> Self {
        Self {
            events: None,
            error: Some(error),
            control: None,
            health_after_apply: false,
        }
    }

    pub(in super::super) fn control(control: PrivateWsControl) -> Self {
        Self {
            events: None,
            error: None,
            control: Some(control),
            health_after_apply: false,
        }
    }

    #[cfg(test)]
    pub(in super::super) fn health_after_apply(&self) -> bool {
        self.health_after_apply
    }

    #[cfg(test)]
    pub(in super::super) fn is_ignored(&self) -> bool {
        self.events.is_none() && self.error.is_none() && self.control.is_none()
    }
}

pub(in super::super) fn push_private_ws_payload(
    state: &AppState,
    venue: &'static str,
    messages: &mut Vec<String>,
    requested: usize,
    label: &'static str,
    payload: ExchangeResult<String>,
) -> bool {
    match payload {
        Ok(message) => {
            messages.push(message);
            true
        }
        Err(error) => {
            let error = format!("{label}: {error}");
            state.private_ws_health().record_subscribe_build_failed(
                venue,
                requested,
                messages.len(),
                &error,
            );
            warn!(%venue, %label, %error, "private ws payload build failed");
            false
        }
    }
}

pub(in super::super) struct AbortOnDrop {
    handle: JoinHandle<()>,
}

impl AbortOnDrop {
    pub(in super::super) fn new(handle: JoinHandle<()>) -> Self {
        Self { handle }
    }
}

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

pub(in super::super) fn parse_failed(venue: &'static str, error: &ExchangeError) -> PrivateWsParse {
    let message = error.to_string();
    warn!(%venue, %error, "private ws parse failed");
    PrivateWsParse::failed(message)
}

pub(in super::super) fn ws_config(
    exchange: &'static str,
    url: String,
    heartbeat_interval: Duration,
    heartbeat: WsHeartbeat,
    inbound_codec: WsInboundCodec,
    server_ping: WsServerPing,
) -> WsConfig {
    WsConfig {
        url,
        exchange: format!("{exchange}-private"),
        heartbeat_interval,
        heartbeat,
        inbound_codec,
        server_ping,
        initial_reconnect_delay: Duration::from_secs(1),
        max_reconnect_delay: Duration::from_secs(60),
        circuit_breaker_threshold: 5,
    }
}
