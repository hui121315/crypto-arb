use super::*;

#[tokio::test]
async fn append_and_query_funding_round_trip() -> Result<(), Box<dyn std::error::Error>> {
    let store = HistoryStore::new(10);
    store.append_funding_rates(&[funding_rate()]).await?;
    let rows = store
        .query_funding(FundingQuery {
            symbol: Some("btc".into()),
            limit: 10,
            ..FundingQuery::default()
        })
        .await?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].exchange, "binance");
    Ok(())
}

#[tokio::test]
async fn append_and_query_funding_diff_round_trip() -> Result<(), Box<dyn std::error::Error>> {
    let store = HistoryStore::new(10);
    let rows = vec![
        funding_rate_for("BTC", "binance", 0.0001),
        funding_rate_for("BTC", "okx", 0.0005),
    ];
    let diffs = HistoryStore::derive_funding_diffs(&rows);
    store.append_funding_diffs(&diffs).await?;

    let rows = store
        .query_funding_diffs(FundingDiffQuery {
            symbol: Some("btc".into()),
            long_exchange: Some("binance".into()),
            short_exchange: Some("okx".into()),
            limit: 10,
            ..FundingDiffQuery::default()
        })
        .await?;

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].gross_diff_bps, 4.0);
    Ok(())
}

#[tokio::test]
async fn query_funding_diff_stats_uses_funding_cycles() -> Result<(), Box<dyn std::error::Error>> {
    let store = HistoryStore::new(10);
    let latest_ms = 72 * 3_600_000;
    store
        .append_funding_diffs(&[
            funding_diff_row(latest_ms - 24 * 3_600_000, 2.0, 8),
            funding_diff_row(latest_ms - 8 * 3_600_000, 4.0, 8),
            funding_diff_row(latest_ms, 8.0, 8),
        ])
        .await?;

    let rows = store
        .query_funding_diff_stats(FundingDiffStatsQuery {
            symbol: Some("btc".into()),
            limit: 10,
            ..FundingDiffStatsQuery::default()
        })
        .await?;

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].base_interval_hours, 8);
    assert_eq!(rows[0].windows[0].window_hours, 8);
    assert_eq!(rows[0].windows[1].window_hours, 24);
    assert_eq!(rows[0].windows[2].window_hours, 72);
    Ok(())
}

#[tokio::test]
async fn append_and_query_index_composition_round_trip() -> Result<(), Box<dyn std::error::Error>> {
    let store = HistoryStore::new(10);
    store
        .append_index_compositions(&[index_composition("binance", "BTC")])
        .await?;

    let rows = store
        .query_index_compositions(IndexCompositionQuery {
            venue: Some("binance".into()),
            symbol: Some("btc".into()),
            limit: 10,
            ..IndexCompositionQuery::default()
        })
        .await?;

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].component_count, 1);
    assert_eq!(rows[0].quality, IndexCompositionQuality::Verified);
    Ok(())
}
