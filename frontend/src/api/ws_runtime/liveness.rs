use super::*;
use futures::future::{select, Either};
use gloo_timers::callback::Interval;

const CONNECT_TIMEOUT_MS: u32 = 15_000;
const SILENCE_TIMEOUT_MS: u64 = PING_INTERVAL_MS as u64 * 2 + 5_000;

pub(super) async fn read_ticket(client: &ApiClient) -> Result<shared_types::WsTicketResponse, ApiError> {
    let request = client.ws_ticket();
    let timeout = TimeoutFuture::new(CONNECT_TIMEOUT_MS);
    futures::pin_mut!(request, timeout);
    match select(request, timeout).await {
        Either::Left((result, _)) => result,
        Either::Right(_) => Err(ApiError::client(
            "WS_AUTH_TICKET_TIMEOUT", "获取实时连接许可超过 15 秒，将自动重试",
        )),
    }
}

impl WsRuntime {
    pub(super) fn watch_connection(&self, token: u64) {
        let started_at_ms = clock_ms();
        let inner = Rc::downgrade(&self.inner);
        *self.inner.watchdog.borrow_mut() = Some(Interval::new(1_000, move || {
            let Some(inner) = inner.upgrade() else { return; };
            let runtime = WsRuntime { inner };
            if !runtime.generation_matches(token) { return; }
            let now = clock_ms();
            let problem = if runtime.inner.frame_sender.borrow().is_none() {
                (now.saturating_sub(started_at_ms) >= u64::from(CONNECT_TIMEOUT_MS)).then(||
                    ws_reconnect_problem("runtime", "WS_OPEN_TIMEOUT", "实时连接超过 15 秒未建立，将自动重连"))
            } else {
                let expired = runtime.inner.pending_subscribe_batches.borrow().iter()
                    .find(|pending| now.saturating_sub(pending.sent_at_ms) >= u64::from(CONNECT_TIMEOUT_MS)
                        && pending.channels.iter().any(|channel| runtime.channel_is_active(channel)))
                    .map(|pending| pending.request_id.clone());
                expired.map(|request_id| ws_reconnect_problem(
                    "runtime", "WS_SUBSCRIBE_TIMEOUT", "后台超过 15 秒未确认订阅，将重新连接",
                ).with_request_id(Some(request_id))).or_else(||
                    (now.saturating_sub(runtime.inner.last_received_at_ms.get()) >= SILENCE_TIMEOUT_MS).then(||
                        ws_reconnect_problem("runtime", "WS_HEARTBEAT_TIMEOUT", "实时连接持续无回应，将自动重连")))
            };
            if let Some(problem) = problem {
                // Dispose socket callbacks and this timer after their current JS callback returns.
                spawn_local(async move {
                    TimeoutFuture::new(0).await;
                    if runtime.generation_matches(token) {
                        runtime.broadcast_problem(&problem);
                        runtime.socket_closed(token);
                    }
                });
            }
        }));
    }
}

pub(super) fn clock_ms() -> u64 {
    #[cfg(target_arch = "wasm32")]
    {
        web_sys::window().and_then(|window| window.performance())
            .map(|performance| performance.now() as u64)
            .unwrap_or_else(|| js_sys::Date::now() as u64)
    }
    #[cfg(not(target_arch = "wasm32"))]
    { chrono::Utc::now().timestamp_millis().max(0) as u64 }
}
