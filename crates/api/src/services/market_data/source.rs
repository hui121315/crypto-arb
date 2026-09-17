use async_trait::async_trait;
use exchange::{ExchangeError, ExchangeResult, FanoutReport, PublicWsSnapshot};
use shared_types::{IndexCompositionSnapshot, OrderBookInfo, SpotTick, TickerInfo};

#[async_trait]
pub(crate) trait MarketDataSource: Send + Sync {
    #[cfg(test)]
    async fn fetch_perp_tickers_report(&self) -> FanoutReport<TickerInfo>;
    async fn fetch_spot_ticks_report(&self) -> FanoutReport<SpotTick>;
    async fn fetch_perp_tickers_for_venue_report(&self, venue: &str) -> FanoutReport<TickerInfo>;
    async fn fetch_spot_ticks_for_venue_report(&self, venue: &str) -> FanoutReport<SpotTick>;

    async fn fetch_ws_orderbook(
        &self,
        _venue: &str,
        _symbol: &str,
        _depth: u32,
    ) -> ExchangeResult<PublicWsSnapshot<OrderBookInfo>> {
        Ok(PublicWsSnapshot::Unsupported)
    }

    async fn fetch_ws_spot_orderbook(
        &self,
        _venue: &str,
        _symbol: &str,
        _depth: u32,
    ) -> ExchangeResult<PublicWsSnapshot<OrderBookInfo>> {
        Ok(PublicWsSnapshot::Unsupported)
    }

    async fn fetch_index_composition(
        &self,
        _venue: &str,
        _symbol: &str,
    ) -> ExchangeResult<IndexCompositionSnapshot> {
        Err(ExchangeError::UnsupportedCapability("index_composition"))
    }
}
