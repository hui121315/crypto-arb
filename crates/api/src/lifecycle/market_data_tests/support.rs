use super::*;
use async_trait::async_trait;
use shared_types::{FundingRateData, MarkIndexInfo, OrderBookInfo, SpotTick, TickerInfo};

#[path = "support/fallback_adapters.rs"]
mod fallback_adapters;
#[path = "support/rows.rs"]
mod rows;

pub(super) use fallback_adapters::{FailingTouchAdapter, UnsupportedTouchAdapter};
use rows::{funding, mark_index, spot_tick, ticker};

pub(super) fn test_runtime() -> MarketDataRuntime {
    MarketDataRuntime {
        aggregator: Arc::new(exchange::Aggregator::new()),
        market_data: Arc::new(MarketDataCache::default()),
        market_subscriptions: Arc::new(
            crate::services::market_subscriptions::MarketSubscriptions::load(None),
        ),
        watchlist: Arc::new(tokio::sync::RwLock::new(Vec::new())),
        watchlist_alert_store: Arc::new(realtime::WatchlistAlertStore::default()),
        hub: realtime::WsHub::default(),
    }
}

pub(super) struct TouchAdapter {
    empty_ws_snapshot: bool,
    partial_ws_snapshot: bool,
}

pub(super) struct CoordinatedTouchAdapter {
    venue: &'static str,
    barrier: Arc<tokio::sync::Barrier>,
}

impl CoordinatedTouchAdapter {
    pub(super) const fn new(venue: &'static str, barrier: Arc<tokio::sync::Barrier>) -> Self {
        Self { venue, barrier }
    }
}

#[async_trait]
impl exchange::ExchangeAdapter for CoordinatedTouchAdapter {
    fn name(&self) -> &'static str {
        self.venue
    }

    async fn public_ws_ticker_snapshot(
        &self,
        _symbols: &[String],
    ) -> exchange::ExchangeResult<exchange::PublicWsSnapshot<TickerInfo>> {
        self.barrier.wait().await;
        Ok(exchange::PublicWsSnapshot::Pending)
    }

    async fn get_funding_rate(&self, _symbol: &str) -> exchange::ExchangeResult<FundingRateData> {
        rate_limited()
    }

    async fn get_funding_rates(
        &self,
        _symbols: Option<&[String]>,
    ) -> exchange::ExchangeResult<Vec<FundingRateData>> {
        rate_limited()
    }

    async fn get_ticker(&self, _symbol: &str) -> exchange::ExchangeResult<TickerInfo> {
        rate_limited()
    }

    async fn get_tickers(
        &self,
        _symbols: Option<&[String]>,
    ) -> exchange::ExchangeResult<Vec<TickerInfo>> {
        rate_limited()
    }

    async fn get_orderbook(
        &self,
        _symbol: &str,
        _depth: u32,
    ) -> exchange::ExchangeResult<OrderBookInfo> {
        Err(exchange::ExchangeError::UnsupportedCapability("orderbook"))
    }

    fn normalize_symbol(&self, symbol: &str) -> String {
        symbol.to_ascii_uppercase()
    }

    fn to_exchange_symbol(&self, symbol: &str) -> String {
        symbol.to_ascii_uppercase()
    }
}

impl TouchAdapter {
    pub(super) const fn ready() -> Self {
        Self {
            empty_ws_snapshot: false,
            partial_ws_snapshot: false,
        }
    }

    pub(super) const fn empty() -> Self {
        Self {
            empty_ws_snapshot: true,
            partial_ws_snapshot: false,
        }
    }

    pub(super) const fn partial() -> Self {
        Self {
            empty_ws_snapshot: false,
            partial_ws_snapshot: true,
        }
    }

    fn ws_symbols<'a>(&self, symbols: &'a [String]) -> &'a [String] {
        if self.partial_ws_snapshot {
            &symbols[..symbols.len().min(1)]
        } else {
            symbols
        }
    }
}

#[async_trait]
impl exchange::ExchangeAdapter for TouchAdapter {
    fn name(&self) -> &'static str {
        "bybit"
    }

    async fn public_ws_ticker_snapshot(
        &self,
        symbols: &[String],
    ) -> exchange::ExchangeResult<exchange::PublicWsSnapshot<TickerInfo>> {
        if self.empty_ws_snapshot {
            return Ok(exchange::PublicWsSnapshot::Ready(Vec::new()));
        }
        Ok(exchange::PublicWsSnapshot::Ready(
            self.ws_symbols(symbols)
                .iter()
                .map(|symbol| ticker(symbol))
                .collect(),
        ))
    }

    async fn public_ws_funding_snapshot(
        &self,
        symbols: &[String],
    ) -> exchange::ExchangeResult<exchange::PublicWsSnapshot<FundingRateData>> {
        if self.empty_ws_snapshot {
            return Ok(exchange::PublicWsSnapshot::Ready(Vec::new()));
        }
        Ok(exchange::PublicWsSnapshot::Ready(
            self.ws_symbols(symbols)
                .iter()
                .map(|symbol| funding(symbol))
                .collect(),
        ))
    }

    async fn public_ws_mark_index_snapshot(
        &self,
        symbols: &[String],
    ) -> exchange::ExchangeResult<exchange::PublicWsSnapshot<MarkIndexInfo>> {
        if self.empty_ws_snapshot {
            return Ok(exchange::PublicWsSnapshot::Ready(Vec::new()));
        }
        Ok(exchange::PublicWsSnapshot::Ready(
            self.ws_symbols(symbols)
                .iter()
                .map(|symbol| mark_index(symbol))
                .collect(),
        ))
    }

    async fn public_ws_spot_snapshot(
        &self,
        symbols: &[String],
    ) -> exchange::ExchangeResult<exchange::PublicWsSnapshot<SpotTick>> {
        if self.empty_ws_snapshot {
            return Ok(exchange::PublicWsSnapshot::Ready(Vec::new()));
        }
        Ok(exchange::PublicWsSnapshot::Ready(
            self.ws_symbols(symbols)
                .iter()
                .map(|symbol| spot_tick(symbol))
                .collect(),
        ))
    }

    async fn get_funding_rate(&self, symbol: &str) -> exchange::ExchangeResult<FundingRateData> {
        Ok(funding(symbol))
    }

    async fn get_funding_rates(
        &self,
        symbols: Option<&[String]>,
    ) -> exchange::ExchangeResult<Vec<FundingRateData>> {
        Ok(requested_symbols(symbols)
            .iter()
            .map(|symbol| funding(symbol))
            .collect())
    }

    async fn get_ticker(&self, symbol: &str) -> exchange::ExchangeResult<TickerInfo> {
        Ok(ticker(symbol))
    }

    async fn get_tickers(
        &self,
        symbols: Option<&[String]>,
    ) -> exchange::ExchangeResult<Vec<TickerInfo>> {
        Ok(requested_symbols(symbols)
            .iter()
            .map(|symbol| ticker(symbol))
            .collect())
    }

    async fn get_spot_tickers(
        &self,
        symbols: Option<&[String]>,
    ) -> exchange::ExchangeResult<Vec<SpotTick>> {
        Ok(requested_symbols(symbols)
            .iter()
            .map(|symbol| spot_tick(symbol))
            .collect())
    }

    async fn get_orderbook(
        &self,
        _symbol: &str,
        _depth: u32,
    ) -> exchange::ExchangeResult<OrderBookInfo> {
        Err(exchange::ExchangeError::UnsupportedCapability("orderbook"))
    }

    fn normalize_symbol(&self, symbol: &str) -> String {
        symbol.to_ascii_uppercase()
    }

    fn to_exchange_symbol(&self, symbol: &str) -> String {
        symbol.to_ascii_uppercase()
    }
}

fn rate_limited<T>() -> exchange::ExchangeResult<T> {
    Err(exchange::ExchangeError::RateLimited {
        retry_after_secs: 3,
    })
}

fn requested_symbols(symbols: Option<&[String]>) -> Vec<String> {
    symbols.map_or_else(|| vec!["MU".to_owned()], <[String]>::to_vec)
}
