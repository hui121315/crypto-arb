use super::*;
use shared_types::{
    FundingRateData, IndexComponent, IndexCompositionQuality, IndexCompositionSnapshot,
};

mod funding;
mod health;
mod migration;
mod observability;

fn funding_rate() -> FundingRateData {
    funding_rate_for("BTC", "binance", 0.0001)
}

fn api_health_sample(
    exchange: &str,
    endpoint: &str,
    outcome: &str,
    occurred_at_ms: i64,
) -> ApiHealthSampleRow {
    let rate_limited = outcome == "rate_limited";
    ApiHealthSampleRow {
        occurred_at_ms,
        exchange: exchange.into(),
        endpoint: endpoint.into(),
        method: Some("GET".into()),
        outcome: outcome.into(),
        status_code: Some(if rate_limited { 429 } else { 200 }),
        latency_ms: Some(42.5),
        retry_after_ms: rate_limited.then_some(750),
        circuit_state: rate_limited.then(|| "half_open".to_owned()),
        error_code: (outcome == "error").then(|| "UPSTREAM_5XX".to_owned()),
        payload: serde_json::json!({ "endpoint": endpoint }),
    }
}

fn ledger_event(
    event_id: &str,
    category: &str,
    action: &str,
    occurred_at_ms: i64,
) -> LedgerEventRow {
    LedgerEventRow {
        occurred_at_ms,
        event_id: event_id.into(),
        category: category.into(),
        action: action.into(),
        actor: Some("operator".into()),
        resource: Some("order:BTCUSDT".into()),
        outcome: "accepted".into(),
        severity: Some("high".into()),
        request_id: Some(format!("req-{event_id}")),
        run_id: Some(format!("run-{event_id}")),
        ticket_id: Some(format!("ticket-{event_id}")),
        client_order_id: Some(format!("client-{event_id}")),
        exchange_order_id: Some(format!("exch-{event_id}")),
        payload: serde_json::json!({ "action": action }),
    }
}

fn index_composition(venue: &str, symbol: &str) -> IndexCompositionSnapshot {
    IndexCompositionSnapshot {
        venue: venue.into(),
        symbol: symbol.into(),
        index_id: format!("{symbol}USDT"),
        components: vec![IndexComponent {
            symbol: format!("{symbol}/USDT"),
            name: venue.into(),
            weight: 1.0,
            price: Some(100.0),
        }],
        quality: IndexCompositionQuality::Verified,
        source: "test".into(),
        received_at_ms: 1,
        freshness_ms: Some(0),
        error: None,
        retry_after_ms: None,
        source_url: None,
        payload_sha256: None,
        schema_version: None,
    }
}

fn funding_rate_for(symbol: &str, exchange: &str, rate_8h: f64) -> FundingRateData {
    FundingRateData {
        symbol: symbol.into(),
        exchange: exchange.into(),
        rate: rate_8h,
        rate_8h,
        predicted_rate: None,
        next_funding_time: 100,
        funding_interval: 8,
        volume_24h: 1_000_000.0,
        timestamp: 1,
        smoothed_rate: None,
        rate_std: None,
        is_outlier: false,
    }
}

fn funding_diff_row(
    occurred_at_ms: i64,
    gross_diff_bps: f64,
    interval_hours: u32,
) -> FundingDiffRow {
    FundingDiffRow {
        occurred_at_ms,
        symbol: "BTC".into(),
        long_exchange: "binance".into(),
        short_exchange: "okx".into(),
        long_rate_8h: 0.0,
        short_rate_8h: 0.0,
        gross_diff_bps,
        long_next_funding_ms: 0,
        short_next_funding_ms: 0,
        window_alignment_minutes: 0,
        long_interval_hours: interval_hours,
        short_interval_hours: interval_hours,
        min_volume_24h: 1_000_000.0,
    }
}
