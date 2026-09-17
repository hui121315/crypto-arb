use super::super::snapshot::response_parts;
use super::*;

/// 从单次序列化的 handler 响应中取回类型化 payload。
async fn response_payload<T: serde::de::DeserializeOwned>(
    response: axum::response::Response,
) -> anyhow::Result<T> {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

#[tokio::test]
async fn fresh_read_reuses_current_lifecycle_snapshot() -> anyhow::Result<()> {
    let state = AppState::new(common::config::AppConfig::default()).await?;
    state.cache_arbitrage_report(OpportunityScanReport {
        opportunities: vec![test_opp()],
        meta: shared_types::OpportunityScanMeta::default(),
    });

    let parts = response_parts(&state, true, false);

    assert_eq!(parts.source, "snapshot");
    assert_eq!(parts.status, OpportunityEnvelopeStatus::Fresh);
    assert_eq!(parts.entry.value.opportunities.len(), 1);
    assert!(parts.error.is_none());
    Ok(())
}

#[tokio::test]
async fn fresh_read_without_snapshot_does_not_start_a_scan() -> anyhow::Result<()> {
    let state = AppState::new(common::config::AppConfig::default()).await?;

    let parts = response_parts(&state, true, false);

    assert_eq!(parts.source, "warming");
    assert_eq!(parts.status, OpportunityEnvelopeStatus::Warming);
    assert!(parts.entry.value.opportunities.is_empty());
    assert!(parts.retry_after_ms.is_some());
    assert!(parts.error.is_some());
    assert!(state.arbitrage_scan_lock().try_lock().is_ok());
    Ok(())
}

#[tokio::test]
async fn opportunity_rest_handlers_record_payload_metrics() -> anyhow::Result<()> {
    let state = AppState::new(common::config::AppConfig::default()).await?;
    state.cache_arbitrage_report(OpportunityScanReport {
        opportunities: vec![test_opp_with_kind(StrategyKind::PerpCross)],
        meta: shared_types::OpportunityScanMeta::default(),
    });

    let list: shared_types::OpportunityListEnvelope = response_payload(
        opportunities_list(
            State(state.clone()),
            Query(OpportunityListParams {
                page_size: Some(1),
                limit: None,
                cursor: None,
                sort_key: None,
                min_yield: None,
                strategy: None,
                symbol: None,
                fresh: false,
                fast: true,
            }),
        )
        .await?,
    )
    .await?;
    let detail: shared_types::ArbitrageOpportunityDto =
        response_payload(opportunity_detail(State(state.clone()), Path("test".to_owned())).await?)
            .await?;

    let snap = state.metrics().snapshot();
    assert_eq!(list.rows.len(), 1);
    assert_eq!(detail.id, "test");
    assert_eq!(snap.rest_opportunity_list_rows, 1);
    assert!(snap.rest_opportunity_list_payload_bytes > 0);
    assert!(snap.rest_opportunity_detail_seed_payload_bytes > 0);
    Ok(())
}

#[tokio::test]
async fn funding_rates_snapshot_reads_the_canonical_market_cache() -> anyhow::Result<()> {
    let state = AppState::new(common::config::AppConfig::default()).await?;
    state
        .market_data()
        .store_funding_rows(&[funding("BTC", "cache")], MarketSource::RestBaseline);

    let (snapshot, source) = funding_rates_snapshot(&state);

    assert_eq!(snapshot.rows.len(), 1);
    assert_eq!(snapshot.rows[0].exchange, "cache");
    assert_eq!(snapshot.row_evidence.len(), 1);
    assert_eq!(source, MarketSource::LocalCache);
    Ok(())
}

#[tokio::test]
async fn funding_rates_snapshot_does_not_mutate_an_empty_cache() -> anyhow::Result<()> {
    let state = AppState::new(common::config::AppConfig::default()).await?;

    let (snapshot, source) = funding_rates_snapshot(&state);

    assert!(snapshot.rows.is_empty());
    assert!(snapshot.row_evidence.is_empty());
    assert_eq!(source, MarketSource::LocalCache);
    assert!(state.market_data().funding_rows_snapshot().is_empty());
    Ok(())
}
