use crate::services::hedge_ticket::CachedLegOrderbook;
use crate::services::market_data::{envelope::orderbook_envelope, MarketQuality};
use shared_types::{
    MarketDataCoverage, MarketDataEnvelope, MarketDataHealth, MarketDataQuality,
    MarketDataSourceKind, OrderBookInfo,
};

pub(super) fn cached_orderbook_envelope(
    cached: CachedLegOrderbook,
    depth: u32,
    observed_at_ms: i64,
) -> MarketDataEnvelope<Option<OrderBookInfo>> {
    if cached.read.value.is_none()
        && cached.read.quality == MarketQuality::Missing
        && cached.read.retry_after_ms.is_none()
        && cached.read.last_error.is_none()
    {
        return deferred_orderbook_envelope(observed_at_ms);
    }
    orderbook_envelope(
        cached.read,
        &cached.venue,
        &cached.symbol,
        depth as usize,
        observed_at_ms,
    )
}

fn deferred_orderbook_envelope(observed_at_ms: i64) -> MarketDataEnvelope<Option<OrderBookInfo>> {
    MarketDataEnvelope {
        data: None,
        health: MarketDataHealth {
            quality: MarketDataQuality::Unverified,
            source: MarketDataSourceKind::LocalCache,
            freshness_ms: None,
            retry_after_ms: None,
            last_error: None,
            observed_at_ms,
            coverage: Some(MarketDataCoverage::new(0, 0)),
            problem: None,
        },
        retry_after_ms: None,
        row_cap: None,
        row_evidence: Vec::new(),
        fanout: Vec::new(),
    }
}
