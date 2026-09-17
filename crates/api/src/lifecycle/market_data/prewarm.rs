use super::ws_touch::prewarm_ws_tickers;
use super::*;
use crate::services::market_data::aggregator_source::SubscribedMarketDataSource;
use crate::services::market_subscriptions::MarketSubscriptionFeed;

pub(super) async fn prewarm_market_data(
    runtime: &MarketDataRuntime,
    source: MarketSource,
    ticker_requests: &BTreeMap<String, Vec<String>>,
    baseline_refresh: &BaselineRefresh,
) -> PrewarmOutcome {
    match baseline_refresh {
        BaselineRefresh::Shard(shard) => {
            prewarm_baseline_shard(runtime, source, ticker_requests, shard).await
        }
        BaselineRefresh::None => prewarm_watchlist_only(runtime, ticker_requests).await,
    }
}

async fn prewarm_baseline_shard(
    runtime: &MarketDataRuntime,
    source: MarketSource,
    ticker_requests: &BTreeMap<String, Vec<String>>,
    shard: &BaselineShard,
) -> PrewarmOutcome {
    let refresh_shard = async {
        let feed = match shard.feed {
            BaselineFeed::PerpTickers => MarketSubscriptionFeed::Perp,
            BaselineFeed::SpotTicks => MarketSubscriptionFeed::Spot,
        };
        if !runtime.market_subscriptions.enabled(&shard.venue, feed) {
            return (0, 0);
        }
        let producer = SubscribedMarketDataSource::new(
            runtime.aggregator.as_ref(),
            runtime.market_subscriptions.as_ref(),
        );
        match shard.feed {
            BaselineFeed::PerpTickers => {
                let count = runtime
                    .market_data
                    .prewarm_public_perp_tickers_for_venue(&producer, source, &shard.venue)
                    .await;
                (count, 0)
            }
            BaselineFeed::SpotTicks => {
                let count = runtime
                    .market_data
                    .prewarm_public_spot_ticks_for_venue(&producer, source, &shard.venue)
                    .await;
                (0, count)
            }
        }
    };
    let ((perp_tickers, spot_ticks), ws_touched) =
        tokio::join!(refresh_shard, prewarm_ws_tickers(runtime, ticker_requests));
    PrewarmOutcome {
        stats: PublicBaselineStats {
            perp_tickers,
            spot_ticks,
        },
        ws_touched,
    }
}

async fn prewarm_watchlist_only(
    runtime: &MarketDataRuntime,
    ticker_requests: &BTreeMap<String, Vec<String>>,
) -> PrewarmOutcome {
    let ws_touched = prewarm_ws_tickers(runtime, ticker_requests).await;
    let stats = PublicBaselineStats {
        perp_tickers: 0,
        spot_ticks: 0,
    };
    PrewarmOutcome { stats, ws_touched }
}

#[derive(Clone, Copy)]
pub(super) struct PrewarmOutcome {
    pub(super) stats: PublicBaselineStats,
    pub(super) ws_touched: usize,
}

impl PrewarmOutcome {
    pub(super) const fn returned_rows(self) -> usize {
        self.stats
            .perp_tickers
            .saturating_add(self.stats.spot_ticks)
            .saturating_add(self.ws_touched)
    }
}

pub(super) fn prewarm_request_count(ticker_requests: &BTreeMap<String, Vec<String>>) -> usize {
    ticker_requests
        .values()
        .map(Vec::len)
        .fold(0usize, usize::saturating_add)
}

pub(super) fn prewarm_task_outcome(
    outcome: &PrewarmOutcome,
    baseline_refresh: &BaselineRefresh,
    requested: usize,
) -> Result<(), String> {
    if outcome.returned_rows() > 0
        || matches!(baseline_refresh, BaselineRefresh::Shard(_))
        || (matches!(baseline_refresh, BaselineRefresh::None) && requested == 0)
    {
        return Ok(());
    }
    match baseline_refresh {
        BaselineRefresh::None => Err(format!(
            "public market watchlist prewarm returned no rows for {requested} requests"
        )),
        BaselineRefresh::Shard(_) => Ok(()),
    }
}
