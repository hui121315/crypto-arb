use super::*;
use crate::trading_service::{private_ws_events::*, AdapterCredentials};
use axum::{extract::WebSocketUpgrade, routing::get, Router};

#[tokio::test]
async fn real_private_ws_rejects_old_connection_after_account_rotation() -> anyhow::Result<()> {
    let runtime = tempfile::tempdir()?;
    let mut config = common::config::AppConfig::default();
    config.storage.data_dir = runtime.path().display().to_string();
    config.storage.portfolio_nav_path = None;
    config.storage.execution_ledger_path = None;
    config.storage.execution_run_ledger_path = None;
    config.storage.order_snapshot_path = None;
    config.storage.close_run_ledger_path = None;
    config.storage.watchlist_alerts_path = None;
    config.storage.postgres_url = None;
    config.security.audit_log_path = None;
    let state = AppState::new(config).await?;
    let first = credentials("a");
    let second = credentials("b");
    state
        .trading_service()
        .initialize_account_reader(first.clone())?;
    assert!(state.trading_service().private_ws_credentials_match(&first));
    assert!(!state
        .trading_service()
        .private_ws_credentials_match(&second));

    let (accepted, mut connections) = tokio::sync::mpsc::unbounded_channel();
    let app = Router::new().route(
        "/ws",
        get(move |upgrade: WebSocketUpgrade| {
            let accepted = accepted.clone();
            async move {
                upgrade.on_upgrade(move |mut socket| async move {
                    let (send, mut messages) = tokio::sync::mpsc::unbounded_channel::<String>();
                    if accepted.send(send).is_err() {
                        return;
                    }
                    while let Some(message) = messages.recv().await {
                        if socket
                            .send(axum::extract::ws::Message::Text(message.into()))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                })
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let endpoint = format!("ws://{}/ws", listener.local_addr()?);
    let _server = AbortOnDrop::new(tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    }));
    let parsed = Arc::new(AtomicUsize::new(0));
    let mut old = start(&state, &endpoint, parsed.clone());
    let old_messages = tokio::time::timeout(Duration::from_secs(2), connections.recv())
        .await?
        .unwrap();
    old_messages.send("41".into())?;
    wait_for_balance(&state, &first, 41.0).await?;

    // Refreshing unchanged keys reuses the physical connection, including after paper/live changes.
    {
        let _config = state.trading_runtime_config_mutation_lock().lock().await;
        state
            .trading_service()
            .refresh_account_reader(first.clone())?;
    }
    old_messages.send("42".into())?;
    wait_for_balance(&state, &first, 42.0).await?;
    let new_task = {
        let _config = state.trading_runtime_config_mutation_lock().lock().await;
        state
            .trading_service()
            .refresh_account_reader(second.clone())?;
        start(&state, &endpoint, parsed.clone())
    };
    let _new_task = AbortOnDrop::new(new_task);
    let new_messages = tokio::time::timeout(Duration::from_secs(2), connections.recv())
        .await?
        .unwrap();
    new_messages.send("55".into())?;
    wait_for_balance(&state, &second, 55.0).await?;
    let before = health(&state);
    old_messages.send("999".into())?;
    // Completion proves the stale task consumed/rejected the queued frame; a sleep would not.
    tokio::time::timeout(Duration::from_secs(2), &mut old).await??;
    assert_eq!(parsed.load(Ordering::SeqCst), 3);
    assert_eq!(
        health(&state),
        before,
        "old shutdown must not mark the replacement unhealthy"
    );
    wait_for_balance(&state, &second, 55.0).await?;
    assert!(state
        .trading_service()
        .unresolved_private_account_scope(
            "binance",
            PrivateAccountScope::All,
            common::time::now_ms()
        )
        .is_none());
    new_messages.send("56".into())?;
    wait_for_balance(&state, &second, 56.0).await?;
    assert_eq!(parsed.load(Ordering::SeqCst), 4);
    Ok(())
}

fn credentials(key: &str) -> AdapterCredentials {
    AdapterCredentials {
        binance_live: Some((format!("isolated-{key}"), "fake-secret".into())),
        ..Default::default()
    }
}

fn start(state: &AppState, endpoint: &str, parsed: Arc<AtomicUsize>) -> JoinHandle<()> {
    spawn_plain_private_ws(
        state.clone(),
        "binance",
        super::super::super::protocol::ws_config(
            "binance",
            endpoint.into(),
            Duration::from_secs(60),
            WsHeartbeat::PingFrame,
            WsInboundCodec::Plain,
            WsServerPing::None,
        ),
        || Ok(Vec::new()),
        move |message| {
            parsed.fetch_add(1, Ordering::SeqCst);
            let total = message.parse::<f64>().unwrap();
            PrivateWsParse::events(vec![
                PrivateWsEvent::Balances(Box::new(PrivateBalancesSnapshot {
                    venue: "binance".into(),
                    rows: vec![shared_types::VenueBalanceInfo {
                        venue: "binance".into(),
                        currency: "USDT".into(),
                        total,
                        available: total,
                        frozen: 0.0,
                        unrealized_pnl: 0.0,
                    }],
                })),
                PrivateWsEvent::Positions(PrivatePositionsSnapshot {
                    venue: "binance".into(),
                    rows: vec![],
                }),
                PrivateWsEvent::OpenOrders(PrivateOpenOrdersSnapshot {
                    venue: "binance".into(),
                    rows: vec![],
                }),
            ])
        },
    )
}

async fn wait_for_balance(
    state: &AppState,
    credentials: &AdapterCredentials,
    total: f64,
) -> anyhow::Result<()> {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            // Only read a proven fresh cache; never fall through to fake-key remote requests.
            if state
                .trading_service()
                .unresolved_private_account_scope(
                    "binance",
                    PrivateAccountScope::Balances,
                    common::time::now_ms(),
                )
                .is_none()
            {
                let rows = state
                    .trading_service()
                    .list_configured_balances(credentials.clone())
                    .await
                    .unwrap();
                if rows.len() == 1 && rows[0].total == total {
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    Ok(())
}

fn health(state: &AppState) -> Vec<crate::services::private_ws_health::PrivateWsRuntimeHealth> {
    let mut rows = state.private_ws_health().snapshot(0);
    for row in &mut rows {
        row.freshness_ms = None;
    }
    rows.sort_by_key(|row| row.operation);
    rows
}
