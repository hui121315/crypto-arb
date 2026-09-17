#![allow(clippy::panic)]

use super::super::*;
use async_trait::async_trait;
use exchange::{ExchangeAdapter, ExchangeResult, PublicWsSnapshot};
use rust_decimal::Decimal;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

pub(super) fn ticker(symbol: &str) -> TickerInfo {
    TickerInfo {
        symbol: symbol.to_owned(),
        exchange: "mock".to_owned(),
        bid: 1.0,
        ask: 1.1,
        last: 1.05,
        volume_24h: 1_000.0,
        timestamp: 1_000,
    }
}
pub(super) fn spot_tick(symbol: &str) -> SpotTick {
    SpotTick {
        venue: "mock".to_owned(),
        symbol: symbol.to_owned(),
        bid: Decimal::new(100, 0),
        ask: Decimal::new(101, 0),
        last: Decimal::new(1005, 1),
        bid_size: Some(Decimal::new(1, 0)),
        ask_size: Some(Decimal::new(1, 0)),
        volume_24h: Decimal::new(1_000, 0),
        exchange_ts_ms: Some(1_000),
        received_at_ms: 1_000,
    }
}
pub(super) fn funding(symbol: &str) -> FundingRateData {
    FundingRateData {
        symbol: symbol.to_owned(),
        exchange: "mock".to_owned(),
        rate: 0.0001,
        rate_8h: 0.0001,
        predicted_rate: None,
        next_funding_time: 0,
        funding_interval: 8,
        volume_24h: 1_000.0,
        timestamp: 1_000,
        smoothed_rate: None,
        rate_std: None,
        is_outlier: false,
    }
}
pub(super) fn index_composition(symbol: &str) -> IndexCompositionSnapshot {
    IndexCompositionSnapshot {
        venue: "mock".to_owned(),
        symbol: symbol.to_owned(),
        index_id: format!("{symbol}-INDEX"),
        components: vec![shared_types::IndexComponent {
            symbol: symbol.to_owned(),
            name: symbol.to_owned(),
            weight: 1.0,
            price: Some(100.0),
        }],
        quality: shared_types::IndexCompositionQuality::Verified,
        source: "test".to_owned(),
        received_at_ms: common::time::now_ms(),
        freshness_ms: Some(0),
        error: None,
        retry_after_ms: None,
        source_url: None,
        payload_sha256: None,
        schema_version: None,
    }
}
pub(super) fn orderbook(symbol: &str) -> OrderBookInfo {
    OrderBookInfo {
        symbol: symbol.to_owned(),
        exchange: "mock".to_owned(),
        bids: vec![[100.0, 1.0]],
        asks: vec![[101.0, 1.0]],
        timestamp: common::time::now_ms(),
    }
}
pub(super) fn find_runtime_health<'a>(
    rows: &'a [MarketRuntimeHealth],
    venue: &str,
    operation: &str,
) -> &'a MarketRuntimeHealth {
    rows.iter()
        .find(|row| row.venue == venue && row.operation == operation)
        .unwrap_or_else(|| panic!("missing runtime health {venue}/{operation}"))
}
pub(super) fn find_status_row(
    rows: &[MarketDataSnapshotStatusRow],
    operation: MarketDataSnapshotOperation,
) -> &MarketDataSnapshotStatusRow {
    rows.iter()
        .find(|row| row.venue == MARKET_AGGREGATE_VENUE && row.operation == operation)
        .unwrap_or_else(|| panic!("missing snapshot status {}", operation.as_str()))
}
pub(super) fn find_cache_access<'a>(
    rows: &'a [MarketCacheAccessMetric],
    feed: &str,
    outcome: &str,
    source: MarketSource,
    quality: MarketQuality,
) -> &'a MarketCacheAccessMetric {
    rows.iter()
        .find(|row| {
            row.key.feed == feed
                && row.key.outcome == outcome
                && row.key.source == source
                && row.key.quality == quality
        })
        .unwrap_or_else(|| panic!("missing cache access {feed}/{outcome}"))
}
#[derive(Default)]
pub(super) struct OrderbookAdapter {
    pub(super) orderbook_calls: AtomicUsize,
    pub(super) perp_ws_calls: AtomicUsize,
    pub(super) spot_orderbook_calls: AtomicUsize,
    pub(super) spot_ws_calls: AtomicUsize,
    pub(super) ticker_calls: AtomicUsize,
    pub(super) spot_calls: AtomicUsize,
    pub(super) index_composition_calls: AtomicUsize,
    pub(super) fail_index_composition: bool,
    pub(super) fail_orderbook: bool,
    pub(super) perp_ws_ready: bool,
    pub(super) perp_ws_ready_after_calls: usize,
    pub(super) spot_ws_ready: bool,
    pub(super) spot_ws_ready_after_calls: usize,
    pub(super) orderbook_error: Option<OrderbookFailure>,
}
#[derive(Clone, Copy)]
pub(super) enum OrderbookFailure {
    Network,
    UnsupportedSymbol,
}

#[async_trait]
impl ExchangeAdapter for OrderbookAdapter {
    fn name(&self) -> &'static str {
        "mock"
    }

    async fn get_funding_rate(&self, symbol: &str) -> ExchangeResult<FundingRateData> {
        Err(exchange::ExchangeError::UnsupportedSymbol(
            symbol.to_owned(),
        ))
    }

    async fn get_funding_rates(
        &self,
        _symbols: Option<&[String]>,
    ) -> ExchangeResult<Vec<FundingRateData>> {
        Ok(Vec::new())
    }

    async fn get_ticker(&self, symbol: &str) -> ExchangeResult<TickerInfo> {
        Err(exchange::ExchangeError::UnsupportedSymbol(
            symbol.to_owned(),
        ))
    }

    async fn get_tickers(&self, _symbols: Option<&[String]>) -> ExchangeResult<Vec<TickerInfo>> {
        self.ticker_calls.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(20)).await;
        Ok(vec![ticker("BTC")])
    }

    async fn get_spot_tickers(&self, _symbols: Option<&[String]>) -> ExchangeResult<Vec<SpotTick>> {
        self.spot_calls.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(20)).await;
        Ok(vec![spot_tick("BTC")])
    }

    async fn get_orderbook(&self, symbol: &str, _depth: u32) -> ExchangeResult<OrderBookInfo> {
        self.orderbook_calls.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(20)).await;
        if let Some(error) = self.orderbook_error {
            Err(error.to_exchange_error())
        } else if self.fail_orderbook {
            Err(exchange::ExchangeError::RateLimited {
                retry_after_secs: 2,
            })
        } else {
            Ok(orderbook(symbol))
        }
    }

    async fn public_ws_orderbook_snapshot(
        &self,
        symbol: &str,
        _depth: u32,
    ) -> ExchangeResult<PublicWsSnapshot<OrderBookInfo>> {
        let calls = self.perp_ws_calls.fetch_add(1, Ordering::SeqCst) + 1;
        if let Some(error) = self.orderbook_error {
            return Err(error.to_exchange_error());
        }
        if self.fail_orderbook {
            return Err(exchange::ExchangeError::RateLimited {
                retry_after_secs: 2,
            });
        }
        if self.perp_ws_ready
            || (self.perp_ws_ready_after_calls > 0 && calls >= self.perp_ws_ready_after_calls)
        {
            Ok(PublicWsSnapshot::Ready(vec![orderbook(symbol)]))
        } else {
            Ok(PublicWsSnapshot::Pending)
        }
    }

    async fn get_spot_orderbook(&self, symbol: &str, _depth: u32) -> ExchangeResult<OrderBookInfo> {
        self.spot_orderbook_calls.fetch_add(1, Ordering::SeqCst);
        Ok(orderbook(symbol))
    }

    async fn public_ws_spot_orderbook_snapshot(
        &self,
        symbol: &str,
        _depth: u32,
    ) -> ExchangeResult<PublicWsSnapshot<OrderBookInfo>> {
        let calls = self.spot_ws_calls.fetch_add(1, Ordering::SeqCst) + 1;
        if self.spot_ws_ready
            || (self.spot_ws_ready_after_calls > 0 && calls >= self.spot_ws_ready_after_calls)
        {
            Ok(PublicWsSnapshot::Ready(vec![orderbook(symbol)]))
        } else {
            Ok(PublicWsSnapshot::Pending)
        }
    }

    async fn get_index_composition(
        &self,
        symbol: &str,
    ) -> ExchangeResult<IndexCompositionSnapshot> {
        self.index_composition_calls.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(20)).await;
        if self.fail_index_composition {
            Err(exchange::ExchangeError::RateLimited {
                retry_after_secs: 2,
            })
        } else {
            Ok(index_composition(symbol))
        }
    }

    fn normalize_symbol(&self, symbol: &str) -> String {
        symbol.to_owned()
    }

    fn to_exchange_symbol(&self, symbol: &str) -> String {
        symbol.to_owned()
    }

    async fn get_balance(
        &self,
        _currency: Option<&str>,
    ) -> ExchangeResult<HashMap<String, shared_types::BalanceInfo>> {
        Ok(HashMap::new())
    }
}
impl OrderbookFailure {
    pub(super) fn to_exchange_error(self) -> exchange::ExchangeError {
        match self {
            Self::Network => exchange::ExchangeError::Network("reset".to_owned()),
            Self::UnsupportedSymbol => exchange::ExchangeError::Api {
                exchange: "mock".to_owned(),
                code: "25100".to_owned(),
                message: "Trading pair BTCUSDT does not exist".to_owned(),
            },
        }
    }
}
