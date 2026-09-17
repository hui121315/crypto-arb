use super::tasks::{tick_or_shutdown, BackgroundTasks, ShutdownToken};
use crate::services::system_health;
use crate::state::AppState;
use tokio::time::MissedTickBehavior;
use tracing::{info, warn};

const UPDATER_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);
const DETAIL_REFRESH_MS: i64 = 30_000;

pub(super) fn spawn_updater(state: &AppState, tasks: &mut BackgroundTasks) {
    let shutdown = tasks.shutdown_token();
    let state = state.clone();
    tasks.supervise(
        "system_health",
        UPDATER_INTERVAL.as_millis() as i64,
        move || {
            let state = state.clone();
            let shutdown = shutdown.clone();
            async move { run_updater(state, shutdown).await }
        },
    );

    info!(
        period_secs = UPDATER_INTERVAL.as_secs(),
        "system health updater started"
    );
}

async fn run_updater(state: AppState, shutdown: ShutdownToken) {
    let registry = state.task_registry().clone();
    let mut last_fingerprint = None;
    let started_at_ms = common::time::now_ms();
    let result = publish_once(&state, &mut last_fingerprint).await;
    registry.record_result_timed("system_health", started_at_ms, result);

    let mut tick = tokio::time::interval(UPDATER_INTERVAL);
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    tick.tick().await;
    while tick_or_shutdown(&mut tick, &shutdown).await {
        let started_at_ms = common::time::now_ms();
        let result = publish_once(&state, &mut last_fingerprint).await;
        registry.record_result_timed("system_health", started_at_ms, result);
    }
}

/// 计算并发布一次系统健康快照。`Ok(())` 表示成功（含 fingerprint 无变化的 dedup no-op）；
/// `Err` 仅表示序列化失败；上游交易所错误必须进入 degraded snapshot，不能打穿发布链路。
async fn publish_once(
    state: &AppState,
    last_fingerprint: &mut Option<String>,
) -> Result<(), String> {
    let health = system_health::snapshot(state).await;
    state.cache_system_health(health.clone());
    let fingerprint = system_fingerprint(&health);
    if last_fingerprint.as_deref() == Some(fingerprint.as_str()) {
        return Ok(());
    }
    // 零订阅者时跳过 WS 序列化；fingerprint 不落盘，订阅者接入后首个变化仍会推送
    //（订阅时的全量 snapshot 由 ws_replay 提供）。
    if state.ws_hub().subscriber_count(realtime::channels::SYSTEM) == 0 {
        return Ok(());
    }
    let message = realtime::WsMessage::json(&health).map_err(|error| {
        warn!(%error, "system health serialization failed");
        error.to_string()
    })?;
    *last_fingerprint = Some(fingerprint);
    state
        .ws_hub()
        .publish_throttled(realtime::channels::SYSTEM, message);
    Ok(())
}

fn system_fingerprint(health: &shared_types::SystemHealth) -> String {
    serde_json::json!({
        "api": health.api,
        "ws": health.ws,
        "orderElapsedMs": health.order_elapsed_ms,
        "risk": health.risk,
        "netDeltaUsd": health.net_delta_usd,
        "netDeltaPctOfNav": health.net_delta_pct_of_nav,
        "nextFunding": health.next_funding,
        "degraded": health.degraded,
        "detailWindow": health.updated_at_ms.div_euclid(DETAIL_REFRESH_MS),
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::config::AppConfig;

    #[tokio::test]
    async fn publish_once_records_success_on_fresh_state() {
        let mut config = AppConfig::default();
        config.history.enabled = false;
        let state = match AppState::new(config).await {
            Ok(state) => state,
            Err(error) => fail(&format!("state init failed: {error:?}")),
        };
        let mut fingerprint = None;

        // 零订阅者：跳过 WS 序列化且不落 fingerprint（订阅后首个变化仍会推送）。
        assert!(publish_once(&state, &mut fingerprint).await.is_ok());
        assert!(fingerprint.is_none());

        // 有订阅者时：快照成功并发布，记录 fingerprint（成功语义，非仅心跳）。
        let _rx = state.ws_hub().subscribe(realtime::channels::SYSTEM);
        assert!(publish_once(&state, &mut fingerprint).await.is_ok());
        assert!(fingerprint.is_some());

        // 再次无变化：dedup no-op 仍视为成功，绝不能误记为失败。
        assert!(publish_once(&state, &mut fingerprint).await.is_ok());
    }

    #[tokio::test]
    async fn publish_once_caches_snapshot_for_status_hot_path() {
        let mut config = AppConfig::default();
        config.history.enabled = false;
        let state = match AppState::new(config).await {
            Ok(state) => state,
            Err(error) => fail(&format!("state init failed: {error:?}")),
        };
        assert!(state.system_health_snapshot().value_now().is_none());

        let mut fingerprint = None;
        assert!(publish_once(&state, &mut fingerprint).await.is_ok());

        assert!(
            state.system_health_snapshot().value_now().is_some(),
            "updater must cache the snapshot so /api/system/health never recomputes"
        );
    }

    #[test]
    fn fingerprint_ignores_problem_timestamp_and_message_churn() {
        let problem = shared_types::RuntimeProblem {
            scope: "market_data".to_owned(),
            operation: "ws_spot_snapshot".to_owned(),
            code: "MARKET_DATA_MISSING".to_owned(),
            message: "first-event deadline 10000ms".to_owned(),
            venue: Some("kraken".to_owned()),
            retry_after_ms: Some(5_000),
            problem: None,
            observed_at_ms: 1_000,
        };
        let mut first = health_with_problem(problem);
        let mut second = first.clone();
        second.updated_at_ms = 2_000;
        second.problems[0].message = "first-event deadline 15000ms".to_owned();
        second.problems[0].retry_after_ms = Some(10_000);
        second.problems[0].observed_at_ms = 2_000;

        assert_eq!(system_fingerprint(&first), system_fingerprint(&second));
        first.problems[0].code = "UPSTREAM_NETWORK".to_owned();
        assert_eq!(system_fingerprint(&first), system_fingerprint(&second));
        first.risk = shared_types::RiskStatusSlot::Warn;
        assert_ne!(system_fingerprint(&first), system_fingerprint(&second));
        first.risk = shared_types::RiskStatusSlot::Ok;
        second.updated_at_ms = DETAIL_REFRESH_MS + 1;
        assert_ne!(system_fingerprint(&first), system_fingerprint(&second));
    }

    fn health_with_problem(problem: shared_types::RuntimeProblem) -> shared_types::SystemHealth {
        shared_types::SystemHealth {
            api_version: "test".to_owned(),
            api: shared_types::ApiHealthSlot {
                healthy: 0,
                total: 0,
                failed_venues: Vec::new(),
            },
            ws: shared_types::WsHealthSlot {
                channels: 0,
                disconnected: Vec::new(),
            },
            order_elapsed_ms: None,
            risk: shared_types::RiskStatusSlot::Ok,
            net_delta_usd: 0.0,
            net_delta_pct_of_nav: 0.0,
            next_funding: None,
            updated_at_ms: 1_000,
            degraded: true,
            problems: vec![problem],
        }
    }

    #[allow(clippy::panic)]
    fn fail(message: &str) -> ! {
        panic!("{message}");
    }
}
