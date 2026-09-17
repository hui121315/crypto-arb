use super::*;

pub(in crate::lifecycle::market_data::tests) struct FailingTouchAdapter;

#[async_trait]
impl exchange::ExchangeAdapter for FailingTouchAdapter {
    fn name(&self) -> &'static str {
        "bybit"
    }

    async fn public_ws_ticker_snapshot(
        &self,
        _symbols: &[String],
    ) -> exchange::ExchangeResult<exchange::PublicWsSnapshot<TickerInfo>> {
        Ok(exchange::PublicWsSnapshot::Pending)
    }

    async fn public_ws_funding_snapshot(
        &self,
        _symbols: &[String],
    ) -> exchange::ExchangeResult<exchange::PublicWsSnapshot<FundingRateData>> {
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

pub(in crate::lifecycle::market_data::tests) struct UnsupportedTouchAdapter;

#[async_trait]
impl exchange::ExchangeAdapter for UnsupportedTouchAdapter {
    fn name(&self) -> &'static str {
        "rest-only"
    }

    async fn get_funding_rate(&self, _symbol: &str) -> exchange::ExchangeResult<FundingRateData> {
        Err(exchange::ExchangeError::UnsupportedCapability("funding"))
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

    async fn get_ticker(&self, _symbol: &str) -> exchange::ExchangeResult<TickerInfo> {
        Err(exchange::ExchangeError::UnsupportedCapability("ticker"))
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
