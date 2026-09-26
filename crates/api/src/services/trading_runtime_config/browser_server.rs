//! Isolated actual settings routes and durable files, with no market/account/trading workers.

use crate::{middleware::audit, state::AppState};
use common::config::AppConfig;
use std::{path::PathBuf, time::Duration};

#[test]
#[ignore = "local settings persistence E2E server, not a unit check"]
fn serve_settings_browser() -> anyhow::Result<()> {
    let directory = PathBuf::from(std::env::var("CROSSLINE_SETTINGS_BROWSER_DIR")?).canonicalize()?;
    anyhow::ensure!(directory.starts_with(std::env::temp_dir().canonicalize()?));
    anyhow::ensure!(directory.join("isolated-settings-fixture").is_file());
    std::thread::Builder::new()
        .name("settings-browser-server".into())
        .stack_size(16 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?
                .block_on(serve(directory))
        })?
        .join()
        .map_err(|_| anyhow::anyhow!("settings browser server panicked"))?
}

async fn serve(directory: PathBuf) -> anyhow::Result<()> {
    // Only this marked temporary directory may provide synthetic settings, never project .env.
    anyhow::ensure!(std::env::current_dir()?.canonicalize()? == directory);
    anyhow::ensure!(directory.join(".env").is_file());
    let mut config = match AppConfig::load() {
        Ok(config) => config,
        Err(error) => {
            anyhow::ensure!(std::env::var_os("CROSSLINE_STARTUP_PROBE").is_none(), "partial environment import detected");
            return Err(error.into());
        }
    };
    config.storage.data_dir = directory.display().to_string();
    config.storage.execution_ledger_path = Some(directory.join("execution-events.jsonl").display().to_string());
    config.storage.execution_run_ledger_path = Some(directory.join("execution-runs.jsonl").display().to_string());
    config.storage.order_snapshot_path = Some(directory.join("order-snapshots.jsonl").display().to_string());
    config.storage.close_run_ledger_path = Some(directory.join("close-runs.jsonl").display().to_string());
    config.storage.portfolio_nav_path = None;
    config.storage.watchlist_alerts_path = None;
    config.storage.webhook_outbox_path = Some(directory.join("webhook-outbox.sqlite").display().to_string());
    config.storage.market_subscriptions_path = Some(directory.join("market-subscriptions.json").display().to_string());
    config.history.enabled = false;
    config.security.auth_token = Some("isolated-settings-browser".into());
    config.security.allowed_origins = vec!["http://127.0.0.1:18080".into()];
    config.security.audit_log_path = Some(directory.join("audit.jsonl").display().to_string());
    audit::init(config.security.audit_log_path.as_deref());
    let state = AppState::new(config).await?;
    if crate::services::webhook::restore_config(&state).is_err() {
        anyhow::ensure!(state.webhook().configuration_problem().is_some());
        let depth = state.webhook().status(1).await.queue_depth;
        anyhow::ensure!(state.webhook().process_next(Duration::ZERO).await.is_err());
        anyhow::ensure!(state.webhook().status(2).await.queue_depth == depth);
    }
    if state.market_subscriptions().ensure_restored().is_err() {
        use crate::services::market_subscriptions::MarketSubscriptionFeed;
        for venue in ["binance", "bitget", "gate", "bybit", "kraken", "hyperliquid:xyz", "gate_crossex:gate", "kucoin", "okx"] {
            for feed in [MarketSubscriptionFeed::Spot, MarketSubscriptionFeed::Perp, MarketSubscriptionFeed::Funding] {
                anyhow::ensure!(!state.market_subscriptions().enabled(venue, feed));
            }
        }
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:18000").await?;
    // Test-only fingerprints observe the same credential accessor used by providers.
    // This server can only load its marked temporary directory and never runs workers.
    let router = crate::app::build_router(state).route("/__test/settings/credential-fingerprints", axum::routing::get(|| async {
        let rows: std::collections::BTreeMap<_, _> = ["JUPITER_API_KEY", "ZEROX_API_KEY", "OKX_DEX_API_KEY",
            "OKX_DEX_SECRET_KEY", "OKX_DEX_PASSPHRASE", "CROSSLINE_STARTUP_PROBE", "CROSSLINE_LITERAL_PROBE"]
            .into_iter().map(|key| (key, crate::services::venue_credentials::secret(key)
                .map(|value| common::signing::hmac_sha256_hex(b"isolated-settings-fingerprint", value.as_bytes())))).collect();
        axum::Json(rows)
    }));
    let result = axum::serve(listener, router)
        .with_graceful_shutdown(async {
            tokio::select! {
                () = crate::lifecycle::shutdown_signal() => {},
                () = tokio::time::sleep(Duration::from_secs(180)) => {},
            }
        }).await;
    audit::shutdown(Duration::from_secs(5)).map_err(anyhow::Error::msg)?;
    result?;
    Ok(())
}
