use super::Binance;
use crate::adapter::ExchangeAdapter;
use crate::adapters::binance_ws_mark;
use crate::adapters::binance_ws_market::MarketStream as WsMarketStream;
use crate::adapters::binance_ws_spot_ticker;
use crate::adapters::binance_ws_ticker;
use shared_types::{FundingRateData, MarkIndexInfo, OrderBookInfo, SpotTick, TickerInfo};

impl Binance {
    /// 记录 REST 全量应答见到的 USDT-M 上市符号（供千倍族符号解析挑选）。
    pub(super) fn note_listed_usdm<'a>(&self, symbols: impl Iterator<Item = &'a str>) {
        for symbol in symbols {
            if crate::adapters::binance_format::is_usdm_perp(symbol)
                && !self.listed_usdm.contains_key(symbol)
            {
                self.listed_usdm.insert(symbol.to_owned(), ());
            }
        }
    }

    pub(super) fn ws_orderbook(&self, exchange_symbol: &str) -> Option<OrderBookInfo> {
        if self.config.testnet || self.config.base_url_override.is_some() {
            return None;
        }
        let stream = self.market_stream.get_or_init(WsMarketStream::new);
        stream.touch(exchange_symbol);
        stream.latest(exchange_symbol)
    }

    pub(super) fn ws_ticker(&self, exchange_symbol: &str) -> Option<TickerInfo> {
        binance_ws_ticker::latest_ticker(&self.config, exchange_symbol)
    }

    pub(super) fn ws_ticker_snapshot(&self, symbols: Option<&[String]>) -> Option<Vec<TickerInfo>> {
        let raw_symbols = symbols?;
        let exch_symbols: Vec<String> = raw_symbols
            .iter()
            .map(|s| self.to_exchange_symbol(s))
            .collect();
        binance_ws_ticker::snapshot_tickers(&self.config, Some(&exch_symbols))
    }

    pub(super) fn ws_spot_tick_snapshot(
        &self,
        symbols: Option<&[String]>,
    ) -> Option<Vec<SpotTick>> {
        binance_ws_spot_ticker::snapshot_spot_ticks(&self.config, symbols)
    }

    /// WS fast-path for `get_funding_rates(Some(&symbols))`. Returns `None`
    /// when caller is asking for the full universe or at least one watchlist
    /// symbol is missing a fresh markPrice row.
    pub(super) fn ws_funding_snapshot(
        &self,
        symbols: Option<&[String]>,
    ) -> Option<Vec<FundingRateData>> {
        let raw_symbols = symbols?;
        let exch_symbols: Vec<String> = raw_symbols
            .iter()
            .map(|s| self.to_exchange_symbol(s))
            .collect();
        binance_ws_mark::snapshot_funding(&self.config, Some(&exch_symbols), |symbol| {
            self.funding_interval_for(symbol)
        })
    }

    pub(super) fn ws_mark_index_snapshot(
        &self,
        symbols: Option<&[String]>,
    ) -> Option<Vec<MarkIndexInfo>> {
        let raw_symbols = symbols?;
        let exch_symbols: Vec<String> = raw_symbols
            .iter()
            .map(|symbol| self.to_exchange_symbol(symbol))
            .collect();
        binance_ws_mark::snapshot_mark_index(&self.config, Some(&exch_symbols))
    }
}
