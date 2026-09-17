use super::*;
use std::time::Duration;

struct WsOrderbookFailure {
    quality: MarketQuality,
    backoff_ms: i64,
    error: String,
}

impl MarketDataCache {
    pub(crate) async fn refresh_orderbook_from_ws(
        &self,
        producer: &dyn MarketDataSource,
        venue: &str,
        symbol: &str,
        depth: u32,
        now_ms: i64,
    ) -> MarketRead<OrderBookInfo> {
        let key = MarketKey::new(venue, symbol);
        if let Some(read) = self.ready_ws_orderbook(&key, now_ms) {
            return read;
        }

        let guard_symbol = format!("{venue}:{symbol}");
        let guard_key = MarketKey::new("ws-perp-orderbook", &guard_symbol);
        let _guard = self.rest_baseline.orderbook_guard(&guard_key).await;
        let refresh_ms = common::time::now_ms();
        if let Some(read) = self.ready_ws_orderbook(&key, refresh_ms) {
            return read;
        }

        self.record_cache_miss_for_feed(FEED_ORDERBOOK);
        let book = match fetch_first_ws_orderbook(producer, venue, symbol, depth).await {
            Ok(book) => book,
            Err(failure) => {
                let failed_at_ms = common::time::now_ms();
                self.record_orderbook_error(
                    &key,
                    failed_at_ms,
                    failure.quality,
                    failure.backoff_ms,
                    &failure.error,
                );
                return self.read_orderbook_after_blocked_fetch(
                    &key,
                    failed_at_ms,
                    Some(failure.backoff_ms),
                    failure.quality,
                    Some(failure.error),
                );
            }
        };
        let refresh_ms = common::time::now_ms();
        self.orderbook_fetch_backoff.remove(&key);
        self.orderbooks.insert(
            key,
            CachedEntry::new(book.clone(), refresh_ms, MarketSource::WsPush),
        );
        let read = MarketRead::fresh(book, 0, MarketSource::WsPush);
        self.record_cache_access(
            FEED_ORDERBOOK,
            CACHE_OUTCOME_REFRESH,
            read.source,
            read.quality,
        );
        read
    }

    fn ready_ws_orderbook(
        &self,
        key: &MarketKey,
        now_ms: i64,
    ) -> Option<MarketRead<OrderBookInfo>> {
        self.reusable_ws_orderbook(key, now_ms)
            .or_else(|| self.blocked_ws_orderbook(key, now_ms))
    }

    fn reusable_ws_orderbook(
        &self,
        key: &MarketKey,
        now_ms: i64,
    ) -> Option<MarketRead<OrderBookInfo>> {
        let read = read_entry(&self.orderbooks, key, now_ms, ORDERBOOK_WS_REUSE_MS)?;
        if read.source != MarketSource::WsPush {
            return None;
        }
        self.record_cache_hit_for_read(FEED_ORDERBOOK, &read);
        Some(read)
    }

    fn blocked_ws_orderbook(
        &self,
        key: &MarketKey,
        now_ms: i64,
    ) -> Option<MarketRead<OrderBookInfo>> {
        let backoff = fetch_backoff_wait(&self.orderbook_fetch_backoff, key, now_ms)?;
        Some(self.read_orderbook_after_blocked_fetch(
            key,
            now_ms,
            Some(backoff.wait_ms),
            backoff.quality,
            backoff.last_error,
        ))
    }
}

async fn fetch_first_ws_orderbook(
    producer: &dyn MarketDataSource,
    venue: &str,
    symbol: &str,
    depth: u32,
) -> Result<OrderBookInfo, WsOrderbookFailure> {
    for attempt in 0..WS_ORDERBOOK_ATTEMPTS {
        match producer.fetch_ws_orderbook(venue, symbol, depth).await {
            Ok(exchange::PublicWsSnapshot::Ready(mut books)) => {
                if let Some(book) = books.pop() {
                    return Ok(book);
                }
            }
            Ok(exchange::PublicWsSnapshot::Pending) => {}
            Ok(exchange::PublicWsSnapshot::Unsupported) => {
                return Err(WsOrderbookFailure {
                    quality: MarketQuality::Unsupported,
                    backoff_ms: ORDERBOOK_UNSUPPORTED_CACHE_MS,
                    error: format!("{venue} {symbol} does not expose perpetual WS depth"),
                });
            }
            Err(error) => {
                tracing::debug!(%venue, %symbol, error = %error, "perpetual orderbook WS snapshot unavailable");
                return Err(WsOrderbookFailure {
                    quality: error_quality(&error),
                    backoff_ms: error_backoff_ms(&error),
                    error: error.to_string(),
                });
            }
        }
        if attempt + 1 < WS_ORDERBOOK_ATTEMPTS {
            tokio::time::sleep(Duration::from_millis(WS_ORDERBOOK_RETRY_MS)).await;
        }
    }
    Err(WsOrderbookFailure {
        quality: MarketQuality::Warming,
        backoff_ms: ORDERBOOK_NEGATIVE_CACHE_MS,
        error: format!(
            "{venue} {symbol} perpetual WS depth produced no first frame within {}ms",
            WS_ORDERBOOK_ATTEMPTS as u64 * WS_ORDERBOOK_RETRY_MS
        ),
    })
}
