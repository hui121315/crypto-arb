use super::*;

#[tokio::test]
async fn stale_snapshot_rebinds_the_same_current_opportunity() -> anyhow::Result<()> {
    let state = AppState::new(common::config::AppConfig::default()).await?;
    let opportunity = test_opportunity("opp-1")?;
    state.opportunity_index().publish(
        "current-snapshot".into(),
        20,
        std::slice::from_ref(&opportunity),
    );

    let result = find_opportunity(&state, "opp-1", Some("stale-snapshot")).await;

    let (snapshot_id, rebound) = result?;
    assert_eq!(snapshot_id, "current-snapshot");
    assert_eq!(rebound.id, opportunity.id);
    Ok(())
}

#[tokio::test]
async fn stale_snapshot_fails_closed_when_opportunity_disappeared() -> anyhow::Result<()> {
    let state = AppState::new(common::config::AppConfig::default()).await?;
    state
        .opportunity_index()
        .publish("current-snapshot".into(), 20, &[]);

    let result = find_opportunity(&state, "opp-1", Some("stale-snapshot")).await;
    let Err(error) = result else {
        anyhow::bail!("missing opportunity did not expire")
    };

    assert_eq!(error.status(), StatusCode::NOT_FOUND);
    assert_eq!(error.code(), codes::OPPORTUNITY_EXPIRED);
    Ok(())
}

fn test_opportunity(id: &str) -> serde_json::Result<ArbitrageOpportunityDto> {
    serde_json::from_value(serde_json::json!({
        "id": id,
        "symbol": "BTC",
        "type": "cross_exchange",
        "typeLabel": "test",
        "longExchange": "binance",
        "shortExchange": "okx",
        "spread8h": 0.0,
        "longRate8h": 0.0,
        "shortRate8h": 0.0,
        "longRate": 0.0,
        "shortRate": 0.0,
        "singleYield": 0.0,
        "netSingleYield": 0.0,
        "rawSingleYield": 0.0,
        "settlementInterval": 8,
        "riskAdjustedYield": 0.0,
        "tradingCostRate": 0.0,
        "minHoldingPeriods": 1,
        "riskLevel": "low",
        "volatility": 0.0,
        "sharpeRatio": 0.0,
        "score": 0.0,
        "recommendation": "hold",
        "optimalPosition": 0.0,
        "maxPosition": 0.0,
        "liquidityScore": 0.0,
        "volume24h": 0.0,
        "dataSource": "test",
        "confidence": 0.0,
        "updatedAt": "2026-08-01T00:00:00Z",
        "longFundingInterval": 8,
        "shortFundingInterval": 8
    }))
}
