use rust_decimal::Decimal;
use shared_types::{FundingRateData, MarkIndexInfo, SpotTick, TickerInfo};

pub(super) fn ticker(symbol: &str) -> TickerInfo {
    TickerInfo {
        symbol: symbol.to_ascii_uppercase(),
        exchange: "bybit".to_owned(),
        bid: 100.0,
        ask: 101.0,
        last: 100.5,
        volume_24h: 1_000.0,
        timestamp: 1_000,
    }
}

pub(super) fn funding(symbol: &str) -> FundingRateData {
    FundingRateData {
        symbol: symbol.to_ascii_uppercase(),
        exchange: "bybit".to_owned(),
        rate: 0.0001,
        rate_8h: 0.0001,
        predicted_rate: None,
        next_funding_time: 28_800_000,
        funding_interval: 8,
        volume_24h: 1_000.0,
        timestamp: 1_000,
        smoothed_rate: None,
        rate_std: None,
        is_outlier: false,
    }
}

pub(super) fn mark_index(symbol: &str) -> MarkIndexInfo {
    MarkIndexInfo {
        symbol: symbol.to_ascii_uppercase(),
        exchange: "bybit".to_owned(),
        mark_price: 100.25,
        index_price: Some(100.2),
        open_interest: None,
        open_interest_value: None,
        timestamp: common::time::now_ms(),
    }
}

pub(super) fn spot_tick(symbol: &str) -> SpotTick {
    SpotTick {
        venue: "bybit".to_owned(),
        symbol: symbol.to_ascii_uppercase(),
        bid: Decimal::from(100),
        ask: Decimal::from(101),
        last: Decimal::from(100),
        bid_size: Some(Decimal::from(2)),
        ask_size: Some(Decimal::from(3)),
        volume_24h: Decimal::from(1_000),
        exchange_ts_ms: Some(1_000),
        received_at_ms: 1_001,
    }
}
