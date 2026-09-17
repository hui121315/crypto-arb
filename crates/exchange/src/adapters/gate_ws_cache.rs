use super::Gate;
use crate::adapter::ExchangeAdapter;
use crate::adapters::gate_market_data::{
    parse_funding_from_ticker_schedule, parse_funding_from_ws_ticker, TickerItem,
};
use crate::adapters::gate_ws_market::MarketStream as WsMarketStream;
use crate::adapters::gate_ws_ticker;
use shared_types::{FundingRateData, OrderBookInfo};

impl Gate {
    pub(super) fn ws_orderbook(&self, symbol: &str, depth: u32) -> Option<OrderBookInfo> {
        if self.config.testnet
            || self.config.base_url_override.is_some()
            || !(1..=50).contains(&depth)
        {
            return None;
        }
        let stream = self.market_stream.get_or_init(WsMarketStream::shared);
        stream.touch(symbol);
        stream.latest(symbol, depth)
    }

    /// Fast-path for `get_funding_rate(symbol)`. Reads the WS `futures.tickers`
    /// row when fresh and stitches it with the verified contract schedule.
    pub(super) async fn ws_funding(&self, exch: &str) -> Option<FundingRateData> {
        let row = gate_ws_ticker::latest_funding_row(&self.config, exch)?;
        self.parse_ws_funding_row(&row, common::time::now_ms())
    }

    /// Snapshot fast-path for `get_funding_rates(symbols)`. Returns `None`
    /// when any symbol is missing a fresh row or its WS payload has no
    /// funding data yet, so the caller falls back to the REST aggregate.
    pub(super) async fn ws_funding_snapshot(
        &self,
        symbols: Option<&[String]>,
    ) -> Option<Vec<FundingRateData>> {
        let raw_symbols = symbols?;
        let exch_symbols: Vec<String> = raw_symbols
            .iter()
            .map(|s| self.to_exchange_symbol(s))
            .collect();
        let rows = gate_ws_ticker::snapshot_funding_rows(&self.config, Some(&exch_symbols))?;
        let mut out = Vec::with_capacity(rows.len());
        let observed_at_ms = common::time::now_ms();
        for row in rows {
            out.push(self.parse_ws_funding_row(&row, observed_at_ms)?);
        }
        Some(out)
    }

    fn parse_ws_funding_row(
        &self,
        row: &TickerItem,
        observed_at_ms: i64,
    ) -> Option<FundingRateData> {
        let interval = self.contract_cache.funding_interval_hours(&row.contract)?;
        self.contract_cache
            .funding_schedule(&row.contract, observed_at_ms)
            .and_then(|(_, next_funding_time)| {
                parse_funding_from_ticker_schedule(row, interval, next_funding_time)
            })
            .or_else(|| parse_funding_from_ws_ticker(row, interval))
    }
}
