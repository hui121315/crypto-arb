use async_trait::async_trait;
use exchange::{
    Aggregator, ExchangeError, ExchangeResult, FanoutReport, FanoutVenueResult, PublicWsSnapshot,
    VenueId,
};
use shared_types::{
    IndexCompositionSnapshot, OrderBookInfo, SpotTick, TickerInfo, OP_REST_PERP_TICKERS,
    OP_REST_SPOT_TICKS,
};
use std::future::Future;
use std::time::Duration;
use tokio::time::timeout;

use super::source::MarketDataSource;
use crate::services::market_subscriptions::{MarketSubscriptionFeed, MarketSubscriptions};

pub(crate) struct SubscribedMarketDataSource<'a> {
    aggregator: &'a Aggregator,
    subscriptions: &'a MarketSubscriptions,
}

impl<'a> SubscribedMarketDataSource<'a> {
    pub(crate) const fn new(
        aggregator: &'a Aggregator,
        subscriptions: &'a MarketSubscriptions,
    ) -> Self {
        Self {
            aggregator,
            subscriptions,
        }
    }

    fn enabled_venues(&self, feed: MarketSubscriptionFeed) -> Vec<String> {
        self.aggregator
            .names()
            .into_iter()
            .filter(|venue| self.subscriptions.enabled(venue, feed))
            .collect()
    }

    fn filter_perp_report(&self, mut report: FanoutReport<TickerInfo>) -> FanoutReport<TickerInfo> {
        report.rows.retain(|row| {
            self.subscriptions
                .enabled(&row.exchange, MarketSubscriptionFeed::Perp)
        });
        report.venues.retain(|row| {
            self.subscriptions
                .enabled(&row.venue, MarketSubscriptionFeed::Perp)
        });
        report
    }

    fn filter_spot_report(&self, mut report: FanoutReport<SpotTick>) -> FanoutReport<SpotTick> {
        report.rows.retain(|row| {
            self.subscriptions
                .enabled(&row.venue, MarketSubscriptionFeed::Spot)
        });
        report.venues.retain(|row| {
            self.subscriptions
                .enabled(&row.venue, MarketSubscriptionFeed::Spot)
        });
        report
    }
}

#[async_trait]
impl MarketDataSource for SubscribedMarketDataSource<'_> {
    #[cfg(test)]
    async fn fetch_perp_tickers_report(&self) -> FanoutReport<TickerInfo> {
        let venues = self.enabled_venues(MarketSubscriptionFeed::Perp);
        let report = self.aggregator.fetch_tickers_report_for(&venues).await;
        self.filter_perp_report(report)
    }

    async fn fetch_spot_ticks_report(&self) -> FanoutReport<SpotTick> {
        let venues = self.enabled_venues(MarketSubscriptionFeed::Spot);
        let report = self.aggregator.fetch_spot_ticks_report_for(&venues).await;
        self.filter_spot_report(report)
    }

    async fn fetch_perp_tickers_for_venue_report(&self, venue: &str) -> FanoutReport<TickerInfo> {
        let report = self
            .aggregator
            .fetch_perp_tickers_for_venue_report(venue)
            .await;
        self.filter_perp_report(report)
    }

    async fn fetch_spot_ticks_for_venue_report(&self, venue: &str) -> FanoutReport<SpotTick> {
        let report = self
            .aggregator
            .fetch_spot_ticks_for_venue_report(venue)
            .await;
        self.filter_spot_report(report)
    }
}

#[async_trait]
impl MarketDataSource for Aggregator {
    #[cfg(test)]
    async fn fetch_perp_tickers_report(&self) -> FanoutReport<TickerInfo> {
        self.fetch_all_tickers_report().await
    }

    async fn fetch_spot_ticks_report(&self) -> FanoutReport<SpotTick> {
        self.fetch_all_spot_ticks_report().await
    }

    async fn fetch_perp_tickers_for_venue_report(&self, venue: &str) -> FanoutReport<TickerInfo> {
        let Some(adapter) = self.get(venue) else {
            return unavailable_venue_report(
                venue,
                OP_REST_PERP_TICKERS,
                "perp ticker adapter not registered",
            );
        };
        single_venue_report(venue, OP_REST_PERP_TICKERS, adapter.get_tickers(None)).await
    }

    async fn fetch_spot_ticks_for_venue_report(&self, venue: &str) -> FanoutReport<SpotTick> {
        let Some(adapter) = self.get(venue) else {
            return unavailable_venue_report(
                venue,
                OP_REST_SPOT_TICKS,
                "spot ticker adapter not registered",
            );
        };
        single_venue_report(venue, OP_REST_SPOT_TICKS, adapter.get_spot_tickers(None)).await
    }

    async fn fetch_ws_orderbook(
        &self,
        venue: &str,
        symbol: &str,
        depth: u32,
    ) -> ExchangeResult<PublicWsSnapshot<OrderBookInfo>> {
        let adapter = self.get(venue).ok_or_else(|| {
            ExchangeError::UnsupportedCapability("orderbook adapter not registered")
        })?;
        adapter.public_ws_orderbook_snapshot(symbol, depth).await
    }

    async fn fetch_ws_spot_orderbook(
        &self,
        venue: &str,
        symbol: &str,
        depth: u32,
    ) -> ExchangeResult<PublicWsSnapshot<OrderBookInfo>> {
        let adapter = self.get(venue).ok_or_else(|| {
            ExchangeError::UnsupportedCapability("spot orderbook adapter not registered")
        })?;
        adapter
            .public_ws_spot_orderbook_snapshot(symbol, depth)
            .await
    }

    async fn fetch_index_composition(
        &self,
        venue: &str,
        symbol: &str,
    ) -> ExchangeResult<IndexCompositionSnapshot> {
        let adapter = self.get(venue).ok_or_else(|| {
            ExchangeError::UnsupportedCapability("index composition adapter not registered")
        })?;
        adapter.get_index_composition(symbol).await
    }
}

async fn single_venue_report<T>(
    venue: &str,
    operation: &'static str,
    request: impl Future<Output = ExchangeResult<Vec<T>>>,
) -> FanoutReport<T> {
    let timeout_limit = venue_timeout(venue);
    let started = tokio::time::Instant::now();
    match timeout(timeout_limit, request).await {
        Ok(Ok(rows)) => successful_venue_report(venue, operation, rows, started.elapsed()),
        Ok(Err(error)) => failed_venue_report(venue, operation, error, started.elapsed()),
        Err(_) => failed_venue_report(
            venue,
            operation,
            ExchangeError::Timeout {
                seconds: timeout_limit.as_secs(),
            },
            started.elapsed(),
        ),
    }
}

fn venue_timeout(venue: &str) -> Duration {
    let seconds = VenueId::from_exchange_name(venue)
        .map(|venue| venue.defaults().fanout_timeout_secs)
        .unwrap_or(10);
    Duration::from_secs(seconds)
}

fn successful_venue_report<T>(
    venue: &str,
    operation: &'static str,
    rows: Vec<T>,
    elapsed: Duration,
) -> FanoutReport<T> {
    let row_count = rows.len();
    FanoutReport {
        rows,
        venues: vec![FanoutVenueResult {
            venue: venue.to_owned(),
            operation,
            rows: row_count,
            latency_ms: elapsed.as_millis() as u64,
            error: None,
            problem: None,
        }],
    }
}

fn failed_venue_report<T>(
    venue: &str,
    operation: &'static str,
    error: ExchangeError,
    elapsed: Duration,
) -> FanoutReport<T> {
    let latency_ms = elapsed.as_millis() as u64;
    let problem = error
        .to_problem(venue, operation)
        .with_latency_ms(Some(latency_ms));
    FanoutReport {
        rows: Vec::new(),
        venues: vec![FanoutVenueResult {
            venue: venue.to_owned(),
            operation,
            rows: 0,
            latency_ms,
            error: Some(error),
            problem: Some(problem),
        }],
    }
}

fn unavailable_venue_report<T>(
    venue: &str,
    operation: &'static str,
    message: &'static str,
) -> FanoutReport<T> {
    failed_venue_report(
        venue,
        operation,
        ExchangeError::UnsupportedCapability(message),
        Duration::ZERO,
    )
}
