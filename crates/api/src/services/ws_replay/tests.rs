use super::*;
use shared_types::{
    ExecutionMode, ExecutionRun, ExecutionRunLeg, ExecutionRunState, HedgeLegRole, MarginMode,
    OrderIntent, OrderSide, OrderSource, OrderType, TimeInForce,
};

#[path = "tests/replay_sources.rs"]
mod replay_sources;
#[path = "tests/watchlist_alerts.rs"]
mod watchlist_alerts;

#[tokio::test]
async fn execution_channel_replays_recent_runs() -> Result<(), String> {
    let state = test_state().await?;
    execution_runs::record(&state, run("old", 1));
    execution_runs::record(&state, run("new", 2));

    let payloads = payloads_for_channel(channels::EXECUTION, &state)
        .await
        .map_err(|error| format!("replay payload failed: {error}"))?;

    let payload = payloads
        .iter()
        .find(|payload| payload.payload["executionRun"]["runId"] == "new")
        .ok_or_else(|| "new replay run missing".to_owned())?;
    assert_eq!(payload.channel, channels::EXECUTION);
    assert_eq!(payload.payload["event"], "execution_run_updated");
    Ok(())
}

#[tokio::test]
async fn arbitrage_channel_replays_warming_opportunity_envelope() -> Result<(), String> {
    let state = test_state().await?;

    let payloads = payloads_for_channel(channels::ARBITRAGE, &state)
        .await
        .map_err(|error| format!("replay payload failed: {error}"))?;

    let payload = payloads
        .first()
        .ok_or_else(|| "arbitrage replay payload missing".to_owned())?;
    assert_eq!(payload.channel, channels::ARBITRAGE);
    assert_eq!(payload.payload["status"], "warming");
    assert_eq!(payload.payload["scope"], "main_p0");
    assert_eq!(payload.payload["source"], "warming");
    assert!(payload.payload["error"]["code"].as_str().is_some());
    Ok(())
}

#[tokio::test]
async fn arbitrage_replay_uses_the_published_snapshot_identity() -> Result<(), String> {
    let state = test_state().await?;
    state.opportunity_index().publish_report(
        "authoritative-snapshot".to_owned(),
        chrono::Utc::now(),
        shared_types::OpportunityScanReport::default(),
    );

    let payloads = payloads_for_channel(channels::ARBITRAGE, &state)
        .await
        .map_err(|error| format!("replay payload failed: {error}"))?;
    let payload = payloads
        .first()
        .ok_or_else(|| "arbitrage replay payload missing".to_owned())?;

    assert_eq!(payload.payload["snapshotId"], "authoritative-snapshot");
    Ok(())
}

#[tokio::test]
async fn orders_channel_replays_recent_order_records() -> Result<(), String> {
    let state = test_state().await?;
    state
        .trading_service()
        .submit(order_intent("older", 1))
        .await
        .map_err(|error| format!("older order failed: {error}"))?;
    state
        .trading_service()
        .submit(order_intent("newer", 2))
        .await
        .map_err(|error| format!("newer order failed: {error}"))?;

    let payloads = payloads_for_channel(channels::ORDERS, &state)
        .await
        .map_err(|error| format!("replay payload failed: {error}"))?;

    let payload = payloads
        .first()
        .ok_or_else(|| "order replay payload missing".to_owned())?;
    assert_eq!(payload.channel, channels::ORDERS);
    assert_eq!(payload.payload["event"], "order_snapshot_replay");
    assert_eq!(payload.payload["record"]["intent"]["id"], "newer");
    Ok(())
}

#[tokio::test]
async fn portfolio_channel_replays_latest_error_envelope() -> Result<(), String> {
    let state = test_state().await?;
    state.cache_portfolio_snapshot_envelope(shared_types::PortfolioSnapshotEnvelope {
        status: shared_types::PortfolioSnapshotStatus::Error,
        source: "portfolio_lifecycle".to_owned(),
        observed_at_ms: 1_700_000_000_000,
        snapshot: None,
        problem: Some(shared_types::ApiProblem::new(
            shared_types::problem::codes::PORTFOLIO_SNAPSHOT_UNAVAILABLE,
            "portfolio snapshot unavailable",
        )),
        problems: Vec::new(),
        operation_health: Vec::new(),
        retry_after_ms: Some(2_000),
    });

    let payloads = payloads_for_channel(channels::PORTFOLIO, &state)
        .await
        .map_err(|error| format!("replay payload failed: {error}"))?;

    let payload = payloads
        .first()
        .ok_or_else(|| "portfolio replay payload missing".to_owned())?;
    assert_eq!(payload.channel, channels::PORTFOLIO);
    assert_eq!(payload.payload["status"], "error");
    assert!(payload.payload["snapshot"].is_null());
    assert_eq!(
        payload.payload["problem"]["code"],
        shared_types::problem::codes::PORTFOLIO_SNAPSHOT_UNAVAILABLE
    );
    Ok(())
}

#[tokio::test]
async fn system_channel_replays_current_health_snapshot() -> Result<(), String> {
    let state = test_state().await?;
    let health = crate::services::system_health::snapshot(&state).await;
    state.cache_system_health(health);

    let payloads = payloads_for_channel(channels::SYSTEM, &state)
        .await
        .map_err(|error| format!("replay payload failed: {error}"))?;

    let payload = payloads
        .first()
        .ok_or_else(|| "system replay payload missing".to_owned())?;
    assert_eq!(payload.channel, channels::SYSTEM);
    assert!(payload.payload["updatedAtMs"].as_i64().is_some());
    assert!(payload.payload["api"]["total"].as_u64().is_some());
    Ok(())
}

#[tokio::test]
async fn system_channel_waits_for_the_lifecycle_snapshot() -> Result<(), String> {
    let state = test_state().await?;

    let payloads = payloads_for_channel(channels::SYSTEM, &state)
        .await
        .map_err(|error| format!("replay payload failed: {error}"))?;

    assert!(payloads.is_empty());
    assert!(state.system_health_snapshot().value_now().is_none());
    Ok(())
}

#[tokio::test]
async fn risk_alerts_channel_replays_current_risk_snapshot() -> Result<(), String> {
    let state = test_state().await?;
    state
        .trading_service()
        .update_risk_config(|config| config.kill_switch_active = true);

    let payloads = payloads_for_channel(channels::RISK_ALERTS, &state)
        .await
        .map_err(|error| format!("replay payload failed: {error}"))?;

    let payload = payloads
        .first()
        .ok_or_else(|| "risk alert replay payload missing".to_owned())?;
    assert_eq!(payload.channel, channels::RISK_ALERTS);
    assert_eq!(payload.payload["event"], "risk_snapshot_replay");
    assert_eq!(payload.payload["risk"]["killSwitchActive"], true);
    assert!(payload.payload.get("executionRun").is_none());
    Ok(())
}

async fn test_state() -> Result<AppState, String> {
    AppState::new(common::config::AppConfig::default())
        .await
        .map_err(|error| format!("test state failed: {error}"))
}

fn run(id: &str, updated_at_ms: i64) -> ExecutionRun {
    ExecutionRun {
        run_id: id.to_owned(),
        ticket_id: format!("ticket-{id}"),
        opportunity_id: format!("opp-{id}"),
        state: ExecutionRunState::SecondLegSubmitted,
        long_leg: leg(HedgeLegRole::Long),
        short_leg: leg(HedgeLegRole::Short),
        net_exposure_usd: 0.0,
        cost_reconciliation: None,
        valuation_problem: None,
        unwind_problem: None,
        finality_problem: None,
        finality_checked_at_ms: None,
        evidence: Default::default(),
        recovery_action: None,
        status_reason: "replay".to_owned(),
        created_at_ms: updated_at_ms,
        updated_at_ms,
    }
}

fn leg(role: HedgeLegRole) -> ExecutionRunLeg {
    ExecutionRunLeg {
        role,
        exchange: "paper".to_owned(),
        symbol: "BTC-USDT".to_owned(),
        order_ids: Vec::new(),
        identity: None,
        finality_source: None,
        confirmed_filled_at_ms: None,
        state: shared_types::LiveOrderState::Accepted,
        target_quantity: 1.0,
        filled_quantity: None,
        target_notional_usd: 1.0,
        filled_notional_usd: None,
        filled_fee: None,
    }
}

fn order_intent(id: &str, at_ms: i64) -> OrderIntent {
    OrderIntent {
        id: id.to_owned(),
        source: OrderSource::Manual,
        strategy: None,
        mode: ExecutionMode::DryRun,
        exchange: "mock".to_owned(),
        symbol: "BTCUSDT".to_owned(),
        side: OrderSide::Buy,
        order_type: OrderType::Limit,
        quantity: 1.0,
        price: Some(10.0),
        slippage_tolerance_bps: None,
        reduce_only: false,
        time_in_force: TimeInForce::Gtc,
        post_only: false,
        margin_mode: MarginMode::Cross,
        leverage: 1.0,
        client_order_id: format!("client-{id}"),
        client_order_id_policy: None,
        created_at_ms: at_ms,
    }
}
