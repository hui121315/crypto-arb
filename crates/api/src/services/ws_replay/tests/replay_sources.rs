use super::*;

#[tokio::test]
async fn funding_rates_channel_replays_latest_cached_envelope() -> Result<(), String> {
    let state = test_state().await?;
    state.market_data().store_funding_rows(
        &[funding_rate("BTC-USDT", 0.0001)],
        market_data::MarketSource::WsPush,
    );

    let payloads = payloads_for_channel(channels::FUNDING_RATES, &state)
        .await
        .map_err(|error| error.to_string())?;

    assert_eq!(payloads.len(), 1);
    assert_eq!(payloads[0].channel, channels::FUNDING_RATES);
    assert_eq!(payloads[0].payload["data"][0]["symbol"], "BTC-USDT");
    assert_eq!(payloads[0].payload["data"][0]["exchange"], "binance");
    assert_eq!(payloads[0].payload["health"]["source"], "local_cache");
    assert_eq!(
        payloads[0].payload["rowEvidence"][0]["health"]["source"],
        "ws_push"
    );
    Ok(())
}

#[tokio::test]
async fn funding_replay_does_not_mutate_an_empty_market_cache() -> Result<(), String> {
    let state = test_state().await?;

    let payloads = payloads_for_channel(channels::FUNDING_RATES, &state)
        .await
        .map_err(|error| error.to_string())?;

    assert_eq!(payloads.len(), 1);
    assert!(payloads[0].payload["data"]
        .as_array()
        .is_some_and(Vec::is_empty));
    assert!(state.market_data().funding_rows_snapshot().is_empty());
    Ok(())
}

#[tokio::test]
async fn portfolio_replay_requires_lifecycle_envelope() -> Result<(), String> {
    let state = test_state().await?;
    state.cache_portfolio_snapshot(portfolio_snapshot("pos-replay", 1_700_000_000_000));

    let payloads = payloads_for_channel(channels::PORTFOLIO, &state)
        .await
        .map_err(|error| format!("replay payload failed: {error}"))?;

    assert!(payloads.is_empty());
    Ok(())
}

fn funding_rate(symbol: &str, rate: f64) -> shared_types::FundingRateData {
    shared_types::FundingRateData {
        symbol: symbol.to_owned(),
        exchange: "binance".to_owned(),
        rate,
        rate_8h: rate,
        predicted_rate: None,
        next_funding_time: common::time::now_ms() + 60_000,
        funding_interval: 8,
        volume_24h: 1_000_000.0,
        timestamp: common::time::now_ms(),
        smoothed_rate: None,
        rate_std: None,
        is_outlier: false,
    }
}

fn portfolio_snapshot(version: &str, now_ms: i64) -> shared_types::PortfolioSnapshot {
    shared_types::PortfolioSnapshot {
        summary: shared_types::PortfolioSummary {
            total_nav_usd: 1000.0,
            nav_evidence: shared_types::PortfolioNavEvidence::default(),
            nav_change_24h_pct: Some(0.0),
            net_delta_usd: 0.0,
            net_delta_pct_of_nav: 0.0,
            naked_exposure_usd: 0.0,
            naked_position_count: 0,
            realized_pnl_today_usd: 0.0,
            pnl_breakdown: shared_types::PnlBreakdown::default(),
            updated_at_ms: now_ms,
        },
        positions: Vec::new(),
        balances: Vec::new(),
        risk: shared_types::RiskSnapshot {
            var_99_1d_usd: 0.0,
            var_pct_of_nav: 0.0,
            var_sample_size: 0,
            funding_clustering: Vec::new(),
            delta_concentration: Vec::new(),
            margin_utilization: Vec::new(),
            hard_limits: shared_types::HardLimitsUsage::default(),
            updated_at_ms: now_ms,
        },
        server_now_ms: now_ms,
        snapshot_version: version.to_owned(),
        degraded: false,
        problems: Vec::new(),
        operation_health: Vec::new(),
        account_state: shared_types::AccountStateSnapshot::default(),
        recent_close_runs: Vec::new(),
    }
}

#[tokio::test]
async fn every_registered_channel_has_an_explicit_replay_source() -> Result<(), String> {
    let state = test_state().await?;

    for spec in realtime::channels::WS_CHANNEL_SPECS {
        payloads_for_channel(spec.name, &state)
            .await
            .map_err(|error| format!("{} replay failed: {error}", spec.name))?;
    }
    Ok(())
}

/// 订阅 `webhook` 即拿到完整运行态，前端首包不再需要一次 REST 读取。
#[tokio::test]
async fn webhook_channel_replays_current_runtime_status() -> Result<(), String> {
    let state = test_state().await?;

    let payloads = payloads_for_channel(channels::WEBHOOK, &state)
        .await
        .map_err(|error| error.to_string())?;

    assert_eq!(payloads.len(), 1);
    assert_eq!(payloads[0].channel, channels::WEBHOOK);
    assert_eq!(payloads[0].payload["queueDepth"], 0);
    assert_eq!(payloads[0].payload["config"]["enabled"], false);
    assert!(
        payloads[0].payload["recentDeliveries"]
            .as_array()
            .is_some_and(Vec::is_empty),
        "a fresh runtime has no delivery records"
    );
    assert!(
        payloads[0].payload.get("secret").is_none(),
        "the signing secret must never reach the stream"
    );
    Ok(())
}

#[tokio::test]
async fn review_channel_replays_the_lifecycle_snapshot() -> Result<(), String> {
    let state = test_state().await?;
    state.cache_review_snapshot(review_snapshot(42));

    let payloads = payloads_for_channel(channels::REVIEW, &state)
        .await
        .map_err(|error| error.to_string())?;

    assert_eq!(payloads.len(), 1);
    assert_eq!(payloads[0].channel, channels::REVIEW);
    assert_eq!(payloads[0].payload["generatedAtMs"], 42);
    assert_eq!(payloads[0].payload["executed"]["rowCount"], 0);
    assert_eq!(payloads[0].payload["strategyPerformance"]["rowCount"], 0);
    Ok(())
}

fn review_snapshot(generated_at_ms: i64) -> shared_types::ReviewRuntimeSnapshot {
    shared_types::ReviewRuntimeSnapshot {
        executed: shared_types::ReviewEnvelope::new(
            Vec::<shared_types::ExecutedTrade>::new(),
            generated_at_ms,
            30,
            shared_types::ReviewDataSource::ExecutionLedger,
            Some(shared_types::ReviewLedgerStatus::NoLedgerEvents),
            Vec::new(),
        ),
        strategy_performance: shared_types::ReviewEnvelope::new(
            Vec::<shared_types::StrategyPerformance>::new(),
            generated_at_ms,
            30,
            shared_types::ReviewDataSource::ExecutionLedger,
            Some(shared_types::ReviewLedgerStatus::NoLedgerEvents),
            Vec::new(),
        ),
        generated_at_ms,
    }
}
