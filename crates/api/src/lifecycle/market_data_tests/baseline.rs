use super::*;

#[test]
fn rotating_baseline_covers_all_venue_feeds_inside_discovery_window() {
    let venues = WS_PREWARM_VENUES
        .iter()
        .map(|venue| venue.as_str().to_owned())
        .collect::<Vec<_>>();
    let slot_count = venues.len() * BaselineFeed::COUNT;
    let shards = (0..slot_count)
        .filter_map(|cursor| baseline_shard_at(&venues, cursor))
        .collect::<Vec<_>>();

    assert_eq!(shards.len(), slot_count);
    assert_eq!(advance_baseline_cursor(slot_count - 1, venues.len()), 0);
    for venue in &venues {
        assert!(shards.contains(&BaselineShard {
            venue: venue.clone(),
            feed: BaselineFeed::PerpTickers,
        }));
        assert!(shards.contains(&BaselineShard {
            venue: venue.clone(),
            feed: BaselineFeed::SpotTicks,
        }));
    }

    let baseline_cycle_ms = WATCHLIST_PREWARM_INTERVAL
        .as_millis()
        .saturating_mul(slot_count as u128);
    assert!(baseline_cycle_ms > TICKER_FRESH_MS as u128);
    assert!(baseline_cycle_ms.saturating_mul(2) <= TICKER_DISCOVERY_MAX_AGE_MS as u128);
}

#[tokio::test]
async fn disabled_venue_is_excluded_from_rest_and_ws_prewarm() {
    let runtime = test_runtime();
    runtime.aggregator.register(Arc::new(TouchAdapter::ready()));
    runtime
        .market_subscriptions
        .update(&shared_types::MarketSubscriptionPatch {
            venue: "bybit".to_owned(),
            spot_enabled: Some(false),
            perp_enabled: Some(false),
            funding_enabled: Some(false),
        })
        .expect("disable bybit feeds");
    let requests = BTreeMap::from([("bybit".to_owned(), vec!["BTC".to_owned()])]);

    let outcome = prewarm_market_data(
        &runtime,
        MarketSource::RestBaseline,
        &requests,
        &BaselineRefresh::Shard(BaselineShard {
            venue: "bybit".to_owned(),
            feed: BaselineFeed::SpotTicks,
        }),
    )
    .await;
    let snapshot = runtime.market_data.market_snapshot_cached();

    assert_eq!(outcome.stats.perp_tickers, 0);
    assert_eq!(outcome.stats.spot_ticks, 0);
    assert_eq!(outcome.ws_touched, 0);
    assert!(snapshot.perp_tickers.is_empty());
    assert!(snapshot.spot_ticks.is_empty());
}
