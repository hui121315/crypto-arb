use super::*;

#[test]
fn watchlist_ticker_requests_group_by_ws_capable_venue() {
    let rows = vec![
        item("BTC", Some("bybit"), Some("gate")),
        item("ETH", Some("bitget"), Some("binance")),
        item("sol", Some("BYBIT"), Some("hyperliquid:xyz")),
        disabled_item("DOGE", Some("bybit"), None),
    ];

    let requests = watchlist_ticker_requests(&rows);

    assert_eq!(
        requests.get("bybit"),
        Some(&vec!["BTC".to_owned(), "SOL".to_owned()])
    );
    assert_eq!(requests.get("gate"), Some(&vec!["BTC".to_owned()]));
    assert_eq!(requests.get("bitget"), Some(&vec!["ETH".to_owned()]));
    assert_eq!(requests.get("binance"), Some(&vec!["ETH".to_owned()]));
    assert_eq!(
        requests.get("hyperliquid:xyz"),
        Some(&vec!["SOL".to_owned()])
    );
    assert!(requests
        .values()
        .all(|symbols| !symbols.iter().any(|s| s == "DOGE")));
}

#[tokio::test]
async fn watchlist_runtime_plan_is_bounded_visible_and_private_ws_free() -> Result<(), String> {
    let runtime = test_runtime();
    let rows = (0..WATCHLIST_TICKER_SYMBOLS_PER_VENUE as u32 + 2)
        .map(|index| {
            let mut row = item(&format!("ASSET{index}"), Some("bybit"), None);
            row.id = i64::from(index) + 1;
            row
        })
        .collect::<Vec<_>>();
    runtime.watchlist.write().await.extend(rows.clone());
    let ticker_requests = watchlist_ticker_requests(&rows);

    let envelope = refresh_watchlist_runtime(&runtime, &rows, &ticker_requests, 42)
        .await
        .ok_or_else(|| "runtime envelope missing".to_owned())?;

    assert_eq!(
        ticker_requests.get("bybit").map(Vec::len),
        Some(WATCHLIST_TICKER_SYMBOLS_PER_VENUE)
    );
    assert_eq!(envelope.runtime.private_ws_symbols_from_watchlist, 0);
    assert_eq!(
        envelope.items[0].runtime.status,
        shared_types::WatchlistPrewarmStatus::Planned
    );
    let last = envelope.items.last().ok_or("items empty")?;
    assert_eq!(
        last.runtime.status,
        shared_types::WatchlistPrewarmStatus::Capped
    );
    assert_eq!(last.runtime.capped_legs, 1);
    assert_eq!(
        last.runtime
            .problem
            .as_ref()
            .map(|problem| problem.code.as_str()),
        Some("WATCHLIST_PREWARM_CAPPED")
    );
    Ok(())
}

#[tokio::test]
async fn watchlist_runtime_surfaces_correlated_prewarm_problem() -> Result<(), String> {
    let runtime = test_runtime();
    let mut row = item("BTC", Some("bybit"), None);
    row.id = 7;
    let rows = vec![row];
    runtime.watchlist.write().await.extend(rows.clone());
    runtime.market_data.record_runtime_error(
        "bybit",
        MARKET_OP_WS_TICKER_SNAPSHOT,
        MarketSource::WsPush,
        1,
        &exchange::ExchangeError::RateLimited {
            retry_after_secs: 2,
        },
    );
    let ticker_requests = watchlist_ticker_requests(&rows);

    let envelope = refresh_watchlist_runtime(&runtime, &rows, &ticker_requests, 42)
        .await
        .ok_or_else(|| "runtime envelope missing".to_owned())?;

    assert_eq!(
        envelope.items[0].runtime.status,
        shared_types::WatchlistPrewarmStatus::Degraded
    );
    let problem = envelope.items[0]
        .runtime
        .problem
        .as_ref()
        .ok_or_else(|| "prewarm problem missing".to_owned())?;
    assert_eq!(problem.code, "WATCHLIST_PREWARM_DEGRADED");
    assert_eq!(problem.retry_after_ms, Some(2_000));
    Ok(())
}

#[tokio::test]
async fn ticker_plan_prevents_false_full_leg_cap() -> Result<(), String> {
    let runtime = test_runtime();
    let mut row = item("BTC", Some("bybit"), None);
    row.id = 7;
    row.created_at_ms = 10;
    let rows = vec![row];
    runtime.watchlist.write().await.extend(rows.clone());
    let ticker_requests = BTreeMap::from([("bybit".to_owned(), vec!["BTC".to_owned()])]);

    let envelope = refresh_watchlist_runtime(&runtime, &rows, &ticker_requests, 42)
        .await
        .ok_or_else(|| "runtime envelope missing".to_owned())?;

    assert_eq!(envelope.items[0].runtime.planned_ticker_legs, 1);
    assert_eq!(envelope.items[0].runtime.capped_legs, 0);
    assert_eq!(
        envelope.items[0].runtime.status,
        shared_types::WatchlistPrewarmStatus::Planned
    );
    Ok(())
}

#[tokio::test]
async fn stale_prewarm_snapshot_cannot_update_recreated_watchlist_id() -> Result<(), String> {
    let runtime = test_runtime();
    let mut planned = item("BTC", Some("bybit"), None);
    planned.id = 7;
    planned.created_at_ms = 10;
    let mut replacement = item("ETH", Some("bybit"), None);
    replacement.id = 7;
    replacement.created_at_ms = 11;
    runtime.watchlist.write().await.push(replacement);
    let ticker_requests = watchlist_ticker_requests(std::slice::from_ref(&planned));

    let envelope = refresh_watchlist_runtime(&runtime, &[planned], &ticker_requests, 42)
        .await
        .ok_or_else(|| "runtime envelope missing".to_owned())?;

    assert_eq!(envelope.items[0].symbol, "ETH");
    assert_eq!(envelope.items[0].runtime.last_prewarm_at_ms, None);
    Ok(())
}
