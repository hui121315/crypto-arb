use super::*;
use crate::market::{
    MarketDataFanoutOutcome, MarketDataHealth, MarketDataQuality, MarketDataRowEvidence,
    MarketDataSnapshotOperation, MarketDataSourceKind,
};
use serde_json::json;

#[test]
fn funding_rates_envelope_uses_shared_camel_case_contract() {
    let envelope = FundingRatesEnvelope {
        data: vec![FundingRateData {
            symbol: "BTCUSDT".into(),
            exchange: "binance".into(),
            rate: 0.0001,
            rate_8h: 0.0003,
            predicted_rate: Some(0.0002),
            next_funding_time: 1_700_000_000_000,
            funding_interval: 8,
            volume_24h: 42_000_000.0,
            timestamp: 1_699_999_000_000,
            smoothed_rate: Some(0.00015),
            rate_std: Some(0.00001),
            is_outlier: true,
        }],
        health: health(MarketDataQuality::Fresh),
        retry_after_ms: Some(1_000),
        row_cap: None,
        row_evidence: vec![MarketDataRowEvidence {
            venue: "binance".into(),
            symbol: "BTCUSDT".into(),
            operation: MarketDataSnapshotOperation::FundingRates,
            health: health(MarketDataQuality::Fresh),
        }],
        fanout: vec![MarketDataFanoutOutcome {
            venue: "okx".into(),
            operation: MarketDataSnapshotOperation::FundingRates,
            health: health(MarketDataQuality::RateLimited),
        }],
    };

    let value = serde_json::to_value(envelope).expect("serialize funding rates envelope");

    assert_eq!(value["data"][0]["rate8h"], json!(0.0003));
    assert_eq!(value["data"][0]["predictedRate"], json!(0.0002));
    assert_eq!(
        value["data"][0]["nextFundingTime"],
        json!(1_700_000_000_000_i64)
    );
    assert_eq!(value["data"][0]["fundingInterval"], json!(8));
    assert_eq!(value["data"][0]["volume24h"], json!(42_000_000.0));
    assert_eq!(value["data"][0]["smoothedRate"], json!(0.00015));
    assert_eq!(value["data"][0]["rateStd"], json!(0.00001));
    assert_eq!(value["data"][0]["isOutlier"], json!(true));
    assert_eq!(value["retryAfterMs"], json!(1_000));
    assert_eq!(value["health"]["retryAfterMs"], json!(1_000));
    assert_eq!(value["rowEvidence"][0]["operation"], json!("funding_rates"));
    assert_eq!(value["fanout"][0]["operation"], json!("funding_rates"));
}

#[test]
fn funding_rates_envelope_accepts_minimal_legacy_payload() {
    let decoded: FundingRatesEnvelope = serde_json::from_value(json!({
        "data": [{
            "symbol": "ETHUSDT",
            "exchange": "okx",
            "rate": 0.0002,
            "rate8h": 0.0002,
            "nextFundingTime": 1700000000000_i64,
            "fundingInterval": 8,
            "volume24h": 12000000.0,
            "timestamp": 1699999000000_i64
        }],
        "health": {
            "quality": "fresh",
            "source": "local_cache",
            "observedAtMs": 1700000000100_i64
        }
    }))
    .expect("deserialize minimal funding rates envelope");

    assert_eq!(decoded.data.len(), 1);
    assert_eq!(decoded.data[0].symbol, "ETHUSDT");
    assert_eq!(decoded.data[0].predicted_rate, None);
    assert!(!decoded.data[0].is_outlier);
    assert!(decoded.row_evidence.is_empty());
    assert!(decoded.fanout.is_empty());
    assert_eq!(decoded.retry_after_ms, None);
    assert_eq!(decoded.health.quality, MarketDataQuality::Fresh);
}

#[test]
fn funding_payment_ingest_report_uses_shared_camel_case_contract() {
    let mut report = FundingPaymentIngestReport {
        observed_at_ms: 1_700,
        window_start_ms: Some(1_000),
        window_end_ms: Some(2_000),
        fetched: 3,
        mapped: 2,
        ledger_events: 1,
        skipped: 1,
        invalid: 1,
        duplicate_or_already_recorded: 1,
        route_failures: 1,
        route_failure_details: vec![FundingPaymentIngestRouteFailure {
            venue: "gate".into(),
            operation: "funding_payments".into(),
            message: "timeout".into(),
        }],
        ..FundingPaymentIngestReport::default()
    };
    report.refresh_skip_reasons();

    let value = serde_json::to_value(report).expect("serialize funding ingest report");

    assert_eq!(value["observedAtMs"], json!(1_700));
    assert_eq!(value["duplicateOrAlreadyRecorded"], json!(1));
    assert_eq!(value["skipReasons"][0]["reason"], json!("invalid_row"));
    assert_eq!(
        value["skipReasons"][1]["reason"],
        json!("duplicate_or_already_recorded")
    );
    assert_eq!(value["routeFailureDetails"][0]["venue"], json!("gate"));
}

fn health(quality: MarketDataQuality) -> MarketDataHealth {
    MarketDataHealth {
        quality,
        source: MarketDataSourceKind::RestBaseline,
        freshness_ms: Some(250),
        retry_after_ms: Some(1_000),
        last_error: None,
        observed_at_ms: 1_700_000_000_100,
        coverage: None,
        problem: Some(ApiProblem::new("FUNDING_RATES_TEST", "test funding health")),
    }
}
