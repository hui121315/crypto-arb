//! 行情与订单簿模型。

use crate::{list::RowCapEvidence, problem::ApiProblem};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarketDataQuality {
    Fresh,
    StaleAllowed,
    StaleBlocked,
    Missing,
    RateLimited,
    CircuitOpen,
    Unsupported,
    Unverified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarketDataSourceKind {
    WsPush,
    RestColdStart,
    RestBaseline,
    RestFallback,
    LocalCache,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketDataCoverage {
    pub requested: u64,
    pub received: u64,
    pub coverage_pct: f64,
}

impl MarketDataCoverage {
    pub fn new(requested: u64, received: u64) -> Self {
        let coverage_pct = if requested == 0 {
            0.0
        } else {
            (received as f64 / requested as f64).clamp(0.0, 1.0)
        };
        Self {
            requested,
            received,
            coverage_pct,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketDataHealth {
    pub quality: MarketDataQuality,
    pub source: MarketDataSourceKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freshness_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    pub observed_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coverage: Option<MarketDataCoverage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<ApiProblem>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketDataFanoutOutcome {
    pub venue: String,
    pub operation: MarketDataSnapshotOperation,
    pub health: MarketDataHealth,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketDataRowEvidence {
    pub venue: String,
    pub symbol: String,
    pub operation: MarketDataSnapshotOperation,
    pub health: MarketDataHealth,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketDataEnvelope<T> {
    pub data: T,
    pub health: MarketDataHealth,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row_cap: Option<RowCapEvidence>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub row_evidence: Vec<MarketDataRowEvidence>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fanout: Vec<MarketDataFanoutOutcome>,
}

pub type FeedSnapshot<T> = MarketDataEnvelope<T>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarketDataSnapshotOperation {
    FundingRates,
    PerpTickers,
    SpotTicks,
    Orderbooks,
    IndexCompositions,
    Metadata,
    FeeSchedule,
    WsFunding,
    WsTicker,
    WsSpotTicks,
}

impl MarketDataSnapshotOperation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FundingRates => "funding_rates",
            Self::PerpTickers => "perp_tickers",
            Self::SpotTicks => "spot_ticks",
            Self::Orderbooks => "orderbooks",
            Self::IndexCompositions => "index_compositions",
            Self::Metadata => "metadata",
            Self::FeeSchedule => "fee_schedule",
            Self::WsFunding => "ws_funding",
            Self::WsTicker => "ws_ticker",
            Self::WsSpotTicks => "ws_spot_ticks",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketDataSnapshotStatusRow {
    pub venue: String,
    pub operation: MarketDataSnapshotOperation,
    pub health: MarketDataHealth,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketDataSnapshotStatus {
    pub observed_at_ms: i64,
    #[serde(default)]
    pub rows: Vec<MarketDataSnapshotStatusRow>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketDataCacheCounters {
    pub hit_total: u64,
    pub miss_total: u64,
    pub stale_total: u64,
    pub hit_ratio: f64,
    pub perp_ticker_snapshot_served_stale_total: u64,
    pub spot_tick_snapshot_served_stale_total: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestBaselineDiagnostics {
    pub orderbook_guard_keys: u64,
    pub orderbook_in_flight: u64,
    pub orderbook_wait_count_total: u64,
    pub orderbook_wait_ms_total: u64,
    pub orderbook_guard_evicted_total: u64,
    pub orderbook_guard_oldest_idle_ms: u64,
    pub snapshot_feed_keys: u64,
    pub snapshot_feed_in_flight: u64,
    pub snapshot_wait_count_total: u64,
    pub snapshot_wait_ms_total: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketCacheAccessRow {
    pub feed: String,
    pub outcome: String,
    pub source: MarketDataSourceKind,
    pub quality: MarketDataQuality,
    pub count: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketDataDiagnosticsSnapshot {
    pub generated_at_ms: i64,
    pub cache: MarketDataCacheCounters,
    pub rest_baseline: RestBaselineDiagnostics,
    pub status: MarketDataSnapshotStatus,
    #[serde(default)]
    pub access_rows: Vec<MarketCacheAccessRow>,
    pub access_total: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TickerInfo {
    pub symbol: String,
    pub exchange: String,
    pub bid: f64,
    pub ask: f64,
    pub last: f64,
    pub volume_24h: f64,
    /// 时间戳（毫秒）。
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarkIndexInfo {
    pub symbol: String,
    pub exchange: String,
    pub mark_price: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index_price: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_interest: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_interest_value: Option<f64>,
    /// 时间戳（毫秒）。
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderBookInfo {
    pub symbol: String,
    pub exchange: String,
    /// 买盘 `[price, base_asset_qty]`。衍生品适配器必须先应用官方合约乘数。
    pub bids: Vec<[f64; 2]>,
    /// 卖盘 `[price, base_asset_qty]`。衍生品适配器必须先应用官方合约乘数。
    pub asks: Vec<[f64; 2]>,
    /// 时间戳（毫秒）。
    pub timestamp: i64,
}

impl OrderBookInfo {
    pub fn best_bid(&self) -> Option<f64> {
        self.bids.first().map(|p| p[0])
    }

    pub fn best_ask(&self) -> Option<f64> {
        self.asks.first().map(|p| p[0])
    }

    pub fn spread(&self) -> Option<f64> {
        Some(self.best_ask()? - self.best_bid()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn market_data_coverage_calculates_ratio() {
        assert_eq!(MarketDataCoverage::new(8, 3).coverage_pct, 0.375);
        assert_eq!(MarketDataCoverage::new(0, 3).coverage_pct, 0.0);
        assert_eq!(MarketDataCoverage::new(2, 5).coverage_pct, 1.0);
    }

    #[test]
    fn market_data_envelope_uses_camel_case_health_fields() {
        let envelope = MarketDataEnvelope {
            data: Vec::<TickerInfo>::new(),
            health: MarketDataHealth {
                quality: MarketDataQuality::RateLimited,
                source: MarketDataSourceKind::RestBaseline,
                freshness_ms: None,
                retry_after_ms: Some(2_000),
                last_error: Some("rate limited".into()),
                observed_at_ms: 1_000,
                coverage: Some(MarketDataCoverage::new(8, 3)),
                problem: Some(
                    ApiProblem::new("MARKET_DATA_RATE_LIMITED", "rate limited")
                        .with_retry_after_ms(Some(2_000)),
                ),
            },
            retry_after_ms: Some(2_000),
            row_cap: Some(RowCapEvidence::exact(8, 3, 3, "market-data-test")),
            row_evidence: vec![MarketDataRowEvidence {
                venue: "binance".into(),
                symbol: "BTCUSDT".into(),
                operation: MarketDataSnapshotOperation::FundingRates,
                health: MarketDataHealth {
                    quality: MarketDataQuality::Fresh,
                    source: MarketDataSourceKind::RestBaseline,
                    freshness_ms: Some(100),
                    retry_after_ms: None,
                    last_error: None,
                    observed_at_ms: 1_000,
                    coverage: Some(MarketDataCoverage::new(1, 1)),
                    problem: None,
                },
            }],
            fanout: vec![MarketDataFanoutOutcome {
                venue: "okx".into(),
                operation: MarketDataSnapshotOperation::FundingRates,
                health: MarketDataHealth {
                    quality: MarketDataQuality::Missing,
                    source: MarketDataSourceKind::RestBaseline,
                    freshness_ms: None,
                    retry_after_ms: None,
                    last_error: Some("empty".into()),
                    observed_at_ms: 1_000,
                    coverage: Some(MarketDataCoverage::new(1, 0)),
                    problem: Some(ApiProblem::new("MARKET_DATA_MISSING", "empty")),
                },
            }],
        };

        let text = serde_json::to_string(&envelope).expect("serialize market envelope");

        let value: serde_json::Value = serde_json::from_str(&text).expect("parse market envelope");

        assert_eq!(value["retryAfterMs"], 2_000);
        assert!(text.contains("\"observedAtMs\":1000"));
        assert!(text.contains("\"coveragePct\":0.375"));
        assert!(text.contains("\"rowCap\""));
        assert!(text.contains("\"maxRows\":8"));
        assert!(text.contains("\"rowEvidence\""));
        assert!(text.contains("\"symbol\":\"BTCUSDT\""));
        assert!(text.contains("\"fanout\""));
        assert!(text.contains("\"operation\":\"funding_rates\""));
    }

    #[test]
    fn snapshot_status_uses_camel_case_rows() {
        let status = MarketDataSnapshotStatus {
            observed_at_ms: 1_000,
            rows: vec![MarketDataSnapshotStatusRow {
                venue: "all".into(),
                operation: MarketDataSnapshotOperation::FundingRates,
                health: MarketDataHealth {
                    quality: MarketDataQuality::Missing,
                    source: MarketDataSourceKind::LocalCache,
                    freshness_ms: None,
                    retry_after_ms: None,
                    last_error: Some("empty".into()),
                    observed_at_ms: 1_000,
                    coverage: Some(MarketDataCoverage::new(0, 0)),
                    problem: Some(ApiProblem::new("MARKET_DATA_MISSING", "empty")),
                },
            }],
        };

        let text = serde_json::to_string(&status).expect("serialize market status");

        assert!(text.contains("\"observedAtMs\":1000"));
        assert!(text.contains("\"operation\":\"funding_rates\""));
        assert!(text.contains("\"lastError\":\"empty\""));
    }

    #[test]
    fn market_data_diagnostics_uses_camel_case_fields() {
        let snapshot = MarketDataDiagnosticsSnapshot {
            generated_at_ms: 1_000,
            cache: MarketDataCacheCounters {
                hit_total: 7,
                miss_total: 3,
                stale_total: 2,
                hit_ratio: 0.7,
                perp_ticker_snapshot_served_stale_total: 1,
                spot_tick_snapshot_served_stale_total: 4,
            },
            rest_baseline: RestBaselineDiagnostics {
                orderbook_guard_keys: 2,
                orderbook_in_flight: 1,
                orderbook_wait_count_total: 5,
                orderbook_wait_ms_total: 60,
                orderbook_guard_evicted_total: 8,
                orderbook_guard_oldest_idle_ms: 9,
                snapshot_feed_keys: 2,
                snapshot_feed_in_flight: 0,
                snapshot_wait_count_total: 3,
                snapshot_wait_ms_total: 40,
            },
            status: MarketDataSnapshotStatus::default(),
            access_rows: vec![MarketCacheAccessRow {
                feed: "orderbook".into(),
                outcome: "hit".into(),
                source: MarketDataSourceKind::RestBaseline,
                quality: MarketDataQuality::Fresh,
                count: 11,
            }],
            access_total: 11,
        };

        let text = serde_json::to_string(&snapshot).expect("serialize diagnostics");

        assert!(text.contains("\"generatedAtMs\":1000"));
        assert!(text.contains("\"orderbookGuardKeys\":2"));
        assert!(text.contains("\"accessRows\""));
        assert!(text.contains("\"accessTotal\":11"));
    }
}
