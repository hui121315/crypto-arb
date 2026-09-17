use arbitrage::{
    algorithms::cross_spot_perp::{self, CrossSpotPerpConfig},
    ArbitrageEngineV3, MarketDataSnapshot, OpportunityMarketDataSource,
};
use criterion::{criterion_group, criterion_main, Criterion};
use rust_decimal_macros::dec;
use shared_types::{ArbitrageConfig, FundingRateData, SpotTick, TickerInfo};
use std::collections::HashMap;
use std::sync::Arc;

const SYMBOLS: usize = 256;
const VENUES: [&str; 7] = [
    "binance",
    "okx",
    "bybit",
    "bitget",
    "gate",
    "kucoin",
    "hyperliquid",
];

#[derive(Debug)]
struct StaticSource {
    snapshot: MarketDataSnapshot,
}

impl OpportunityMarketDataSource for StaticSource {
    fn market_snapshot(&self) -> MarketDataSnapshot {
        self.snapshot.clone()
    }
}

fn bench_engine_v3_scan(c: &mut Criterion) {
    let snapshot = market_snapshot();
    let funding = snapshot
        .funding
        .values()
        .flat_map(|by_venue| by_venue.values().cloned())
        .collect::<Vec<_>>();
    let engine = ArbitrageEngineV3::new(
        config(),
        Arc::new(StaticSource {
            snapshot: snapshot.clone(),
        }),
        100_000.0,
        0.5,
    );
    let full_count = engine.run_scan().len();
    let raw_count = cross_spot_perp::scan(
        &snapshot.spot_ticks,
        &snapshot.perp_tickers,
        &funding,
        CrossSpotPerpConfig::default(),
    )
    .len();
    let full_name = format!("engine_v3_full_market_256_symbols_8_venues_{full_count}_rows");
    let raw_name = format!("cross_spot_perp_raw_256_symbols_8_venues_{raw_count}_rows");

    c.bench_function(&full_name, |b| b.iter(|| engine.run_scan()));
    c.bench_function(&raw_name, |b| {
        b.iter(|| {
            cross_spot_perp::scan(
                &snapshot.spot_ticks,
                &snapshot.perp_tickers,
                &funding,
                CrossSpotPerpConfig::default(),
            )
        })
    });
}

fn config() -> ArbitrageConfig {
    ArbitrageConfig {
        min_spread: 0.000_01,
        min_volume_24h: 50_000.0,
        min_net_yield: 0.0,
        ..ArbitrageConfig::default()
    }
}

fn market_snapshot() -> MarketDataSnapshot {
    MarketDataSnapshot {
        funding: funding_snapshot().into(),
        perp_tickers: perp_snapshot().into(),
        spot_ticks: spot_snapshot().into(),
        ..MarketDataSnapshot::default()
    }
}

fn perp_snapshot() -> Vec<TickerInfo> {
    let mut rows = Vec::with_capacity(SYMBOLS * VENUES.len());
    for idx in 0..SYMBOLS {
        let symbol = format!("SYM{idx:03}USDT");
        for (venue_idx, venue) in VENUES.iter().enumerate() {
            let bid = 100.40 + venue_idx as f64 * 0.03 + (idx % 5) as f64 * 0.001;
            rows.push(TickerInfo {
                symbol: symbol.clone(),
                exchange: (*venue).to_owned(),
                bid,
                ask: bid + 0.02,
                last: bid + 0.01,
                volume_24h: 2_000_000.0,
                timestamp: 1_700_000_000_000,
            });
        }
    }
    rows
}

fn spot_snapshot() -> Vec<SpotTick> {
    let mut rows = Vec::with_capacity(SYMBOLS * VENUES.len());
    for idx in 0..SYMBOLS {
        let symbol = format!("SYM{idx:03}/USDT");
        for (venue_idx, venue) in VENUES.iter().enumerate() {
            let ask = dec!(100) + rust_decimal::Decimal::new(venue_idx as i64 * 2, 2);
            rows.push(SpotTick {
                venue: (*venue).to_owned(),
                symbol: symbol.clone(),
                bid: ask - dec!(0.01),
                ask,
                last: ask,
                bid_size: Some(dec!(100)),
                ask_size: Some(dec!(100)),
                volume_24h: dec!(2000000),
                exchange_ts_ms: Some(1_700_000_000_000),
                received_at_ms: 1_700_000_000_000,
            });
        }
    }
    rows
}

fn funding_snapshot() -> HashMap<String, HashMap<String, FundingRateData>> {
    let mut out = HashMap::with_capacity(SYMBOLS);
    for idx in 0..SYMBOLS {
        let symbol = format!("SYM{idx:03}");
        let mut venues = HashMap::with_capacity(VENUES.len());
        for (venue_idx, venue) in VENUES.iter().enumerate() {
            venues.insert(
                (*venue).to_owned(),
                funding_row(&symbol, venue, idx, venue_idx),
            );
        }
        out.insert(symbol, venues);
    }
    out
}

fn funding_row(symbol: &str, venue: &str, symbol_idx: usize, venue_idx: usize) -> FundingRateData {
    let phase = ((symbol_idx + venue_idx) % 11) as f64 - 5.0;
    let rate = phase * 0.000_01 + venue_idx as f64 * 0.000_003;
    FundingRateData {
        symbol: symbol.to_owned(),
        exchange: venue.to_owned(),
        rate,
        rate_8h: rate,
        predicted_rate: None,
        next_funding_time: 1_700_028_800_000,
        funding_interval: 8,
        volume_24h: 250_000.0 + symbol_idx as f64 * 100.0,
        timestamp: 1_700_000_000_000,
        smoothed_rate: None,
        rate_std: None,
        is_outlier: false,
    }
}

criterion_group!(benches, bench_engine_v3_scan);
criterion_main!(benches);
