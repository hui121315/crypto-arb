use super::Hyperliquid;
use crate::adapters::hyperliquid_ws_active_ctx::ActiveAssetCtxStream as WsActiveAssetCtxStream;
use crate::adapters::hyperliquid_ws_all_mids::AllMidsStream as WsAllMidsStream;
use crate::adapters::hyperliquid_ws_market::MarketStream as WsMarketStream;
use shared_types::{FundingRateData, MarkIndexInfo, OrderBookInfo, TickerInfo};
use std::sync::Arc;

impl Hyperliquid {
    pub(super) fn ws_all_mids_snapshot(&self, coins: &[String]) -> Option<Vec<(String, String)>> {
        if self.config.base_url_override.is_some() {
            return None;
        }
        self.all_mids_stream
            .get_or_init(|| WsAllMidsStream::new(self.adapter_name()))
            .snapshot(coins)
    }

    /// Lazily-initialised WS funding cache. Returns `None` when the adapter
    /// is configured with a `base_url_override` (testnet / mock); callers
    /// fall back to REST in that case.
    fn active_ctx_stream(&self) -> Option<&Arc<WsActiveAssetCtxStream>> {
        if self.config.base_url_override.is_some() {
            return None;
        }
        Some(
            self.active_ctx_stream
                .get_or_init(|| WsActiveAssetCtxStream::new(self.adapter_name())),
        )
    }

    /// WS fast-path for `get_funding_rate(symbol)`; returns the row when
    /// the `activeAssetCtx` cache has a fresh frame for the coin.
    pub(super) fn ws_funding(&self, coin: &str) -> Option<FundingRateData> {
        self.active_ctx_stream()?
            .latest_funding(&self.api_coin(coin))
    }

    /// WS fast-path for `get_funding_rates(Some(symbols))`. Returns `None`
    /// when caller wants the full universe or any coin is missing a fresh frame.
    pub(super) fn ws_funding_snapshot(
        &self,
        symbols: Option<&[String]>,
    ) -> Option<Vec<FundingRateData>> {
        let raw_symbols = symbols?;
        let coins = self.ws_perp_coins(raw_symbols);
        self.active_ctx_stream()?.snapshot_funding(&coins)
    }

    /// WS fast-path for `get_ticker(symbol)`. Reads the `activeAssetCtx`
    /// cache and builds a [`TickerInfo`] when a fresh row is available.
    pub(super) fn ws_ticker(&self, coin: &str) -> Option<TickerInfo> {
        self.active_ctx_stream()?
            .latest_ticker(&self.api_coin(coin))
    }

    /// WS snapshot fast-path for `get_tickers(Some(symbols))`. Returns
    /// `None` for the full-scan case and atomically when any coin is missing.
    pub(super) fn ws_ticker_snapshot(&self, symbols: Option<&[String]>) -> Option<Vec<TickerInfo>> {
        let raw_symbols = symbols?;
        let coins = self.ws_perp_coins(raw_symbols);
        self.active_ctx_stream()?.snapshot_tickers(&coins)
    }

    pub(super) fn ws_mark_index_snapshot(
        &self,
        symbols: Option<&[String]>,
    ) -> Option<Vec<MarkIndexInfo>> {
        let raw_symbols = symbols?;
        let coins = self.ws_perp_coins(raw_symbols);
        self.active_ctx_stream()?.snapshot_mark_index(&coins)
    }

    pub(super) fn ws_perp_coins(&self, symbols: &[String]) -> Vec<String> {
        symbols.iter().map(|symbol| self.api_coin(symbol)).collect()
    }

    pub(super) fn ws_orderbook(&self, coin: &str, depth: u32) -> Option<OrderBookInfo> {
        if self.config.base_url_override.is_some() || depth > 20 {
            return None;
        }
        let stream = self
            .market_stream
            .get_or_init(|| WsMarketStream::new(self.adapter_name()));
        stream.touch(coin);
        stream.latest(coin).map(|mut book| {
            if depth > 0 {
                let cap = depth as usize;
                book.bids.truncate(cap);
                book.asks.truncate(cap);
            }
            book
        })
    }
}
