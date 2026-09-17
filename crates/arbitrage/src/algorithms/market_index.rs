//! Prepared symbol, venue, quote and executable-price lookups shared by one market scan.

use crate::algorithms::{
    market,
    quote_conversion::{QuoteConversion, QuoteConversionIndex},
};
use crate::interfaces::MarketDataSnapshot;
use shared_types::{FundingRateData, SpotTick, TickerInfo};
use std::collections::HashMap;

type SpotGroups<'a> = HashMap<String, Vec<IndexedSpot<'a>>>;
type PerpGroups<'a> = HashMap<String, Vec<IndexedPerp<'a>>>;
type FundingGroups<'a> = HashMap<String, Vec<IndexedFunding<'a>>>;
type QuotedPerps<'a> = HashMap<String, &'a TickerInfo>;
type QuotedFunding<'a> = HashMap<String, &'a FundingRateData>;
type PerpsByVenueBase<'a> = HashMap<String, HashMap<String, QuotedPerps<'a>>>;
type FundingByVenueBase<'a> = HashMap<String, HashMap<String, QuotedFunding<'a>>>;

#[derive(Debug)]
pub(crate) struct IndexedSpot<'a> {
    pub(crate) row: &'a SpotTick,
    pub(crate) venue_key: String,
    pub(crate) quote: String,
    pub(crate) bid: Option<f64>,
    pub(crate) ask: Option<f64>,
    pub(crate) volume_24h: f64,
}

#[derive(Debug)]
pub(crate) struct IndexedPerp<'a> {
    pub(crate) row: &'a TickerInfo,
    pub(crate) venue_key: String,
    pub(crate) quote: String,
    pub(crate) bid: Option<f64>,
    pub(crate) ask: Option<f64>,
}

#[derive(Debug)]
pub(crate) struct IndexedFunding<'a> {
    pub(crate) row: &'a FundingRateData,
    pub(crate) venue_key: String,
    pub(crate) resolved_quote: Option<String>,
}

#[derive(Debug)]
pub(crate) struct MarketScanIndex<'a> {
    spots_by_base: SpotGroups<'a>,
    perps_by_base: PerpGroups<'a>,
    funding_by_base: FundingGroups<'a>,
    perps_by_venue_base: PerpsByVenueBase<'a>,
    funding_by_venue_base: FundingByVenueBase<'a>,
    quote_conversions: QuoteConversionIndex<'a>,
}

impl<'a> MarketScanIndex<'a> {
    pub(crate) fn from_snapshot(market: &'a MarketDataSnapshot) -> Self {
        Self::build(
            &market.spot_ticks,
            &market.perp_tickers,
            market
                .funding
                .values()
                .flat_map(|by_venue| by_venue.values()),
        )
    }

    pub(crate) fn from_slices(
        spots: &'a [SpotTick],
        perps: &'a [TickerInfo],
        funding: &'a [FundingRateData],
    ) -> Self {
        Self::build(spots, perps, funding.iter())
    }

    fn build<I>(spots: &'a [SpotTick], perps: &'a [TickerInfo], funding: I) -> Self
    where
        I: IntoIterator<Item = &'a FundingRateData>,
    {
        let mut index = Self {
            spots_by_base: HashMap::with_capacity(spots.len()),
            perps_by_base: HashMap::with_capacity(perps.len()),
            funding_by_base: HashMap::new(),
            perps_by_venue_base: HashMap::new(),
            funding_by_venue_base: HashMap::new(),
            quote_conversions: QuoteConversionIndex::default(),
        };
        for spot in spots {
            index.insert_spot(spot);
        }
        for perp in perps {
            index.insert_perp(perp);
        }
        for rate in funding {
            index.insert_funding(rate);
        }
        index
    }

    pub(crate) fn spot_groups(&self) -> impl Iterator<Item = (&str, &[IndexedSpot<'a>])> + '_ {
        self.spots_by_base
            .iter()
            .map(|(base, rows)| (base.as_str(), rows.as_slice()))
    }

    pub(crate) fn perp_groups(&self) -> impl Iterator<Item = (&str, &[IndexedPerp<'a>])> + '_ {
        self.perps_by_base
            .iter()
            .map(|(base, rows)| (base.as_str(), rows.as_slice()))
    }

    pub(crate) fn funding_groups(
        &self,
    ) -> impl Iterator<Item = (&str, &[IndexedFunding<'a>])> + '_ {
        self.funding_by_base
            .iter()
            .map(|(base, rows)| (base.as_str(), rows.as_slice()))
    }

    pub(crate) fn spots(&self, base: &str) -> &[IndexedSpot<'a>] {
        self.spots_by_base.get(base).map_or(&[], Vec::as_slice)
    }

    pub(crate) fn perps_by_key(&self, venue_key: &str, base: &str) -> Option<&QuotedPerps<'a>> {
        self.perps_by_venue_base.get(venue_key)?.get(base)
    }

    pub(crate) fn perp_by_key(
        &self,
        venue_key: &str,
        base: &str,
        quote: &str,
    ) -> Option<&'a TickerInfo> {
        self.perps_by_key(venue_key, base)?.get(quote).copied()
    }

    pub(crate) fn funding_by_key(
        &self,
        venue_key: &str,
        base: &str,
        quote: &str,
    ) -> Option<&'a FundingRateData> {
        self.funding_by_venue_base
            .get(venue_key)?
            .get(base)?
            .get(quote)
            .copied()
    }

    pub(crate) fn conversion_by_key(
        &self,
        venue_key: &str,
        from_quote: &str,
        to_quote: &str,
    ) -> Option<QuoteConversion<'a>> {
        self.quote_conversions
            .find_on_venue(venue_key, from_quote, to_quote)
    }

    fn insert_spot(&mut self, spot: &'a SpotTick) {
        let Some(quote) = market::canonical_quote_symbol(&spot.symbol) else {
            return;
        };
        let base = market::canonical_base_symbol(&spot.symbol);
        let venue_key = market::venue_identity_key(&spot.venue);
        self.quote_conversions
            .insert_normalized_tick(&venue_key, &base, &quote, spot);
        self.spots_by_base
            .entry(base)
            .or_default()
            .push(IndexedSpot {
                row: spot,
                venue_key,
                quote,
                bid: market::spot_bid(spot),
                ask: market::spot_price(spot),
                volume_24h: market::spot_volume_usd(spot),
            });
    }

    fn insert_perp(&mut self, perp: &'a TickerInfo) {
        let Some(quote) = market::canonical_perp_quote_symbol(&perp.exchange, &perp.symbol) else {
            return;
        };
        let base = market::canonical_base_symbol(&perp.symbol);
        let venue_key = market::venue_identity_key(&perp.exchange);
        self.perps_by_base
            .entry(base.clone())
            .or_default()
            .push(IndexedPerp {
                row: perp,
                venue_key: venue_key.clone(),
                quote: quote.clone(),
                bid: market::ticker_bid(perp),
                ask: market::ticker_ask(perp),
            });
        let quoted = self
            .perps_by_venue_base
            .entry(venue_key)
            .or_default()
            .entry(base)
            .or_default();
        if quoted
            .get(&quote)
            .is_none_or(|current| perp.timestamp > current.timestamp)
        {
            quoted.insert(quote, perp);
        }
    }

    fn insert_funding(&mut self, rate: &'a FundingRateData) {
        let base = market::canonical_base_symbol(&rate.symbol);
        let venue_key = market::venue_identity_key(&rate.exchange);
        let resolved_quote = market::canonical_perp_quote_symbol(&rate.exchange, &rate.symbol);
        self.funding_by_base
            .entry(base.clone())
            .or_default()
            .push(IndexedFunding {
                row: rate,
                venue_key: venue_key.clone(),
                resolved_quote: resolved_quote.clone(),
            });
        let Some(quote) = resolved_quote else {
            return;
        };
        let quoted = self
            .funding_by_venue_base
            .entry(venue_key)
            .or_default()
            .entry(base)
            .or_default();
        if quoted
            .get(&quote)
            .is_none_or(|current| rate.timestamp > current.timestamp)
        {
            quoted.insert(quote, rate);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_once_and_selects_the_latest_exact_contract_rows() {
        let spots = [spot("binance", "btc/usdt")];
        let perps = [
            perp("binance-live", "BTCUSDT", 1),
            perp("BINANCE", "BTCUSDT", 2),
        ];
        let funding = [
            rate("binance", "BTCUSDT", 1),
            rate("BINANCE-LIVE", "BTCUSDT", 2),
        ];
        let index = MarketScanIndex::from_slices(&spots, &perps, &funding);

        assert_eq!(index.spots("BTC").len(), 1);
        assert_eq!(
            index
                .perps_by_key("binance", "BTC")
                .and_then(|rows| rows.get("USDT"))
                .map(|row| row.timestamp),
            Some(2)
        );
        assert_eq!(
            index
                .funding_by_key("binance", "BTC", "USDT")
                .map(|row| row.timestamp),
            Some(2)
        );
    }

    #[test]
    fn preserves_hyperliquid_builder_boundaries() {
        let perps = [
            perp("hyperliquid:xyz", "BTC", 1),
            perp("hyperliquid:km", "BTC", 2),
        ];
        let index = MarketScanIndex::from_slices(&[], &perps, &[]);

        assert_eq!(
            index
                .perps_by_key("hyperliquid:xyz", "BTC")
                .and_then(|rows| rows.get("USDC"))
                .map(|row| row.timestamp),
            Some(1)
        );
        assert_eq!(
            index
                .perps_by_key("hyperliquid:km", "BTC")
                .and_then(|rows| rows.get("USDC"))
                .map(|row| row.timestamp),
            Some(2)
        );
    }

    fn spot(venue: &str, symbol: &str) -> SpotTick {
        SpotTick {
            venue: venue.to_owned(),
            symbol: symbol.to_owned(),
            bid: rust_decimal::Decimal::ZERO,
            ask: rust_decimal::Decimal::ZERO,
            last: rust_decimal::Decimal::ZERO,
            bid_size: None,
            ask_size: None,
            volume_24h: rust_decimal::Decimal::ZERO,
            exchange_ts_ms: Some(1),
            received_at_ms: 1,
        }
    }

    fn perp(exchange: &str, symbol: &str, timestamp: i64) -> TickerInfo {
        TickerInfo {
            exchange: exchange.to_owned(),
            symbol: symbol.to_owned(),
            bid: 0.0,
            ask: 0.0,
            last: 0.0,
            volume_24h: 0.0,
            timestamp,
        }
    }

    fn rate(exchange: &str, symbol: &str, timestamp: i64) -> FundingRateData {
        FundingRateData {
            exchange: exchange.to_owned(),
            symbol: symbol.to_owned(),
            rate: 0.0,
            rate_8h: 0.0,
            predicted_rate: None,
            next_funding_time: 0,
            funding_interval: 8,
            volume_24h: 0.0,
            timestamp,
            smoothed_rate: None,
            rate_std: None,
            is_outlier: false,
        }
    }
}
