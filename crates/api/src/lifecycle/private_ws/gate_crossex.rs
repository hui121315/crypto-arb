use super::*;
use exchange::{GateCrossEx, GateCrossExConfig, GateCrossExCredentials, PrivateWsRuntimeStatus};

const STATUS_INTERVAL: Duration = Duration::from_secs(5);

pub(super) fn spawn_gate_crossex_private_ws(
    state: AppState,
    credentials: Option<(String, String)>,
) -> Option<JoinHandle<()>> {
    let (api_key, api_secret) = credentials?;
    Some(tokio::spawn(run(
        state,
        GateCrossExConfig {
            credentials: Some(GateCrossExCredentials {
                api_key,
                api_secret,
            }),
            allow_live_writes: false,
            timeout_secs: 10,
            qps: shared_types::VenueId::GateCrossEx.defaults().qps,
            ..Default::default()
        },
    )))
}

async fn run(state: AppState, config: GateCrossExConfig) {
    let venue = "gate_crossex";
    state.private_ws_health().record_task_started(venue);
    let adapter = match GateCrossEx::new(config) {
        Ok(adapter) => adapter,
        Err(error) => {
            state
                .private_ws_health()
                .record_auth_failed(venue, &error.to_string());
            return;
        }
    };
    supervise_status(&state, venue, || adapter.warm_private_ws()).await;
}

async fn supervise_status<F, Fut>(state: &AppState, venue: &str, mut warm: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = ExchangeResult<PrivateWsRuntimeStatus>>,
{
    let health = state.private_ws_health();
    let mut announced = false;
    let mut tick = tokio::time::interval(STATUS_INTERVAL);
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        tick.tick().await;
        match warm().await {
            Ok(status) => {
                health.record_connected(venue);
                if !announced {
                    health.record_subscribe_attempt(venue, status.subscriptions);
                    health.record_subscribe_sent_pending_ack(
                        venue,
                        status.subscriptions,
                        status.subscriptions,
                    );
                    announced = true;
                }
                record_samples(health, venue, status);
            }
            Err(error) => {
                health.record_disconnected(venue, &error.to_string());
            }
        }
    }
}

pub(super) fn record_samples(
    health: &crate::services::private_ws_health::PrivateWsHealthStore,
    venue: &str,
    status: PrivateWsRuntimeStatus,
) {
    health.record_stream_progress(
        venue,
        crate::services::private_ws_health::OP_PRIVATE_WS_ACCOUNT_STREAM,
        "账户流",
        status.account_samples,
        status.account_streams,
    );
    health.record_stream_progress(
        venue,
        crate::services::private_ws_health::OP_PRIVATE_WS_ORDER_STREAM,
        "订单/成交流",
        status.order_samples,
        status.order_streams,
    );
    if status.account_ready() && status.order_ready() {
        health.record_subscriptions_proven(venue, status.subscriptions);
    }
}
