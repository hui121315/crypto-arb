//! Real HTTP/WS routes over the existing paper account, used only by the browser E2E.

use super::paper_e2e_support::paper_runtime_config;
use super::paper_fixture::{automation_opportunity, seed_books, seed_instruments};
use crate::lifecycle::{drain_runtime, BackgroundTasks};
use crate::state::AppState;
use axum::{
    http::{HeaderMap, StatusCode},
    routing::post,
    Json, Router,
};
use std::sync::{
    atomic::{AtomicU8, Ordering},
    Arc,
};
use std::time::Duration;

#[path = "browser_server/settlements.rs"]
mod settlements;
use settlements::seed_settlement_reviews;

#[path = "browser_server/stocks.rs"]
mod stocks;

#[path = "browser_server/liquidation.rs"]
mod liquidation;

#[path = "browser_server/compensation.rs"]
mod compensation;

#[derive(Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum MarketPhase {
    Baseline,
    TakeProfit,
    StopLoss,
    LiquidationSafe,
    LiquidationNear,
}

#[test]
#[ignore = "local server for test/e2e/paper-cycle.config.ts, not a unit check"]
fn serve_paper_browser() -> anyhow::Result<()> {
    anyhow::ensure!(std::env::var("CROSSLINE_PAPER_BROWSER").as_deref() == Ok("1"));
    std::thread::Builder::new()
        .name("paper-browser-server".into())
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?
                .block_on(serve())
        })?
        .join()
        .map_err(|_| anyhow::anyhow!("paper browser server panicked"))?
}

async fn serve() -> anyhow::Result<()> {
    let runtime = tempfile::tempdir()?;
    // Do not call AppConfig::load or init_services: no .env, saved accounts or venue adapters.
    let mut config = paper_runtime_config(runtime.path());
    config.storage.data_dir = runtime.path().display().to_string();
    config.history.enabled = false;
    config.security.auth_token = Some("isolated-paper-browser".into());
    config.security.allowed_origins = vec!["http://127.0.0.1:18080".into()];
    let mut state = AppState::new(config).await?;
    // Queue-only webhook verification; no delivery worker, DNS or external request.
    state.webhook().update_config(shared_types::WebhookConfigPatch {
        url: Some("https://example.com/isolated-webhook".into()),
        secret: Some("isolated-webhook-signature".into()),
        ..Default::default()
    })?;
    let stock_sources = stocks::Fixture::start().await?;
    state.use_stock_fixture(&stock_sources.root)?;
    state.trading_service().select_mock_adapter();
    seed_settlement_reviews(&state)?;
    if std::env::var("CROSSLINE_PAPER_COMPENSATION").as_deref() == Ok("1") {
        compensation::seed(&state)?;
    }
    let phase = Arc::new(AtomicU8::new(MarketPhase::Baseline as u8));
    seed(&state, phase.load(Ordering::Relaxed))?;
    let mut tasks = BackgroundTasks::new(state.task_registry().clone());
    if std::env::var("CROSSLINE_PAPER_LIQUIDATION").as_deref() == Ok("1") {
        liquidation::spawn_updater(&state, &mut tasks, phase.clone());
    } else {
        crate::lifecycle::portfolio::spawn_updater(&state, &mut tasks);
    }
    crate::lifecycle::ledger_projection::spawn_worker(&state, &mut tasks);
    crate::lifecycle::review_projection::spawn_updater(&state, &mut tasks);
    crate::lifecycle::automated_arbitrage::spawn_worker(&state, &mut tasks);
    crate::lifecycle::profit_exit::spawn_worker(&state, &mut tasks);
    let feed_state = state.clone();
    let feed_phase = phase.clone();
    let feed = tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_millis(100));
        loop {
            tick.tick().await;
            seed(&feed_state, feed_phase.load(Ordering::Relaxed))?;
        }
        #[allow(unreachable_code)]
        Ok::<(), anyhow::Error>(())
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:18000").await?;
    // Only this ignored test server exposes synthetic market control, never the product router.
    let control_state = state.clone();
    let notification_state = state.clone();
    let market_control = Router::new().route(
        "/__paper/market",
        post(move |headers: HeaderMap, Json(next): Json<MarketPhase>| {
            let phase = phase.clone();
            let state = control_state.clone();
            async move {
                if headers
                    .get("authorization")
                    .and_then(|header| header.to_str().ok())
                    != Some("Bearer isolated-paper-browser")
                {
                    return StatusCode::UNAUTHORIZED;
                }
                phase.store(next as u8, Ordering::Relaxed);
                if seed(&state, next as u8).is_err() {
                    return StatusCode::INTERNAL_SERVER_ERROR;
                }
                StatusCode::NO_CONTENT
            }
        }),
    ).route("/__paper/webhook-preview", post(move |headers: HeaderMap, Json(request): Json<shared_types::ExecutionArtifactBuildRequest>| {
        let state = notification_state.clone();
        async move {
            if headers.get("authorization").and_then(|header| header.to_str().ok())
                != Some("Bearer isolated-paper-browser") {
                return Err(StatusCode::UNAUTHORIZED);
            }
            let artifact = crate::services::execution_artifact::build(&state, &request)
                .map_err(|_| StatusCode::BAD_REQUEST)?;
            let now = common::time::now_ms();
            let event = shared_types::WebhookEvent {
                id: format!("opportunity-{}", artifact.artifact_id),
                version: shared_types::WEBHOOK_EVENT_VERSION.into(),
                kind: shared_types::WebhookEventKind::Opportunity,
                occurred_at_ms: now,
                payload: crate::lifecycle::webhook_events::opportunity_artifact_payload(&artifact, None, now),
            };
            // Run the actual provider encoder, never DNS, delivery, credentials or notification workers.
            let bytes = webhook::dispatcher::delivery_body(shared_types::WebhookProvider::Bark, &event)
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            let body: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            Ok::<_, StatusCode>(Json(serde_json::json!({ "event": event, "body": body })))
        }
    }));
    let result = axum::serve(
        listener,
        crate::app::build_router(state.clone()).merge(market_control).merge(stock_sources.controls()),
    )
    .with_graceful_shutdown(async {
        tokio::select! {
            () = crate::lifecycle::shutdown_signal() => {},
            () = tokio::time::sleep(Duration::from_secs(180)) => {},
        }
    })
    .await;
    feed.abort();
    let _ = feed.await;
    drain_runtime(&state, &mut tasks).await.ensure_clean()?;
    result?;
    Ok(())
}

fn seed(state: &AppState, phase: u8) -> anyhow::Result<()> {
    let now = common::time::now_ms();
    seed_instruments(state, now)?;
    let (long, short) = match phase {
        value if value == MarketPhase::TakeProfit as u8 => (110.0, 90.0),
        value if value == MarketPhase::StopLoss as u8 => (90.0, 110.0),
        _ => (100.0, 100.05),
    };
    seed_books(state, long, short, now);
    state.cache_arbitrage_report(shared_types::OpportunityScanReport {
        opportunities: vec![automation_opportunity(now)],
        meta: shared_types::OpportunityScanMeta {
            scan_started_at: Some(chrono::Utc::now()),
            candidate_count: 1,
            emitted_count: 1,
            ..Default::default()
        },
    });
    Ok(())
}
