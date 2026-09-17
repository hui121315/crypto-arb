use super::*;

#[test]
fn wide_envelope_omits_global_funding_row_evidence() {
    let cached_at = Utc::now();
    let rows = vec![dto("perp", StrategyKind::PerpCross, true)];
    let evidence = shared_types::MarketDataRowEvidence {
        venue: "binance".into(),
        symbol: "BTCUSDT".into(),
        operation: shared_types::MarketDataSnapshotOperation::FundingRates,
        health: shared_types::MarketDataHealth {
            quality: shared_types::MarketDataQuality::Fresh,
            source: shared_types::MarketDataSourceKind::RestBaseline,
            freshness_ms: Some(1),
            retry_after_ms: None,
            last_error: None,
            observed_at_ms: 1,
            coverage: Some(shared_types::MarketDataCoverage::new(1, 1)),
            problem: None,
        },
    };

    let env = envelope(OpportunityEnvelopeInput {
        opportunities: rows.clone(),
        source_rows: &rows,
        filtered_rows: &rows,
        meta: OpportunityScanMeta {
            funding_row_evidence: vec![evidence],
            ..OpportunityScanMeta::default()
        },
        cached_at,
        source: "snapshot",
        status: OpportunityEnvelopeStatus::Fresh,
        scope: OpportunityEnvelopeScope::MainP0,
        query_key: "test".into(),
        retry_after_ms: None,
        error: None,
        query_problems: Vec::new(),
    });

    assert!(env.meta.funding_row_evidence.is_empty());
}
