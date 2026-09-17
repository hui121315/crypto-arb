use super::*;

#[test]
fn funding_cycle_outcome_preserves_each_failed_substep() {
    let result = funding_cycle_outcome(
        4,
        Err(format!("{FUNDING_HISTORY_APPEND_FAILED}: disk unavailable")),
        Err(format!("{FUNDING_STREAM_SERIALIZE_FAILED}: invalid value")),
    );

    let error = result.err().unwrap_or_default();
    assert!(error.contains(FUNDING_HISTORY_APPEND_FAILED));
    assert!(error.contains(FUNDING_STREAM_SERIALIZE_FAILED));
}

#[test]
fn funding_cycle_outcome_rejects_empty_rows_even_when_sinks_succeed() {
    let error = funding_cycle_outcome(0, Ok(()), Ok(()))
        .err()
        .unwrap_or_default();

    assert!(error.contains(NO_FUNDING_ROWS));
}

#[test]
fn funding_publication_projection_keeps_fresh_ws_over_rest_discovery() {
    let cache = MarketDataCache::default();
    let now_ms = common::time::now_ms();
    let ws_row = funding_row(now_ms, 0.0002);
    let rest_row = funding_row(now_ms, 0.0009);

    cache.store_funding_rows(std::slice::from_ref(&ws_row), MarketSource::WsPush);
    cache.store_funding_rows(std::slice::from_ref(&rest_row), MarketSource::RestBaseline);
    let projection = current_funding_projection(&cache);

    assert_eq!(projection.rows.len(), 1);
    assert_eq!(projection.rows[0].rate, ws_row.rate);
    assert_eq!(projection.row_evidence.len(), 1);
    assert_eq!(
        projection.row_evidence[0].health.source,
        shared_types::MarketDataSourceKind::WsPush
    );
}

fn funding_row(timestamp: i64, rate: f64) -> shared_types::FundingRateData {
    shared_types::FundingRateData {
        symbol: "BTC".to_owned(),
        exchange: "binance".to_owned(),
        rate,
        rate_8h: rate,
        predicted_rate: None,
        next_funding_time: timestamp.saturating_add(60_000),
        funding_interval: 8,
        volume_24h: 1_000_000.0,
        timestamp,
        smoothed_rate: None,
        rate_std: None,
        is_outlier: false,
    }
}
