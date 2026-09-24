//! Real HTTP/WS routes over the existing paper account, used only by the browser E2E.

use super::paper_e2e_support::paper_runtime_config;
use super::paper_fixture::{automation_opportunity, seed_books, seed_instruments};
use crate::lifecycle::{drain_runtime, BackgroundTasks};
use crate::state::AppState;
use std::time::Duration;

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
    let state = AppState::new(config).await?;
    state.trading_service().select_mock_adapter();
    seed(&state)?;
    let mut tasks = BackgroundTasks::new(state.task_registry().clone());
    crate::lifecycle::portfolio::spawn_updater(&state, &mut tasks);
    crate::lifecycle::ledger_projection::spawn_worker(&state, &mut tasks);
    crate::lifecycle::review_projection::spawn_updater(&state, &mut tasks);
    let feed_state = state.clone();
    let feed = tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        loop {
            tick.tick().await;
            seed(&feed_state)?;
        }
        #[allow(unreachable_code)]
        Ok::<(), anyhow::Error>(())
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:18000").await?;
    let result = axum::serve(listener, crate::app::build_router(state.clone()))
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

fn seed(state: &AppState) -> anyhow::Result<()> {
    let now = common::time::now_ms();
    seed_instruments(state, now)?;
    seed_books(state, 100.0, 100.05, now);
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
