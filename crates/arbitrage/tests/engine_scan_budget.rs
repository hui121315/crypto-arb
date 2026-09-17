use arbitrage::{ArbitrageEngineV3, MarketDataSnapshot, OpportunityMarketDataSource};
use shared_types::{
    ArbitrageConfig, ArbitrageOpportunityDto, FundingRateData, StrategyKind, TickerInfo,
};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const SYMBOLS: usize = 256;
const MAX_ROWS_PER_SYMBOL: usize = 6;
const MAX_SCAN_ROWS: usize = SYMBOLS * MAX_ROWS_PER_SYMBOL;
const MAX_SCAN_DURATION: Duration = Duration::from_secs(2);
const HOUR_MS: i64 = 3_600_000;
const VENUES: [&str; 7] = [
    "binance",
    "okx",
    "bybit",
    "bitget",
    "gate",
    "kucoin",
    "hyperliquid",
];
const NATIVE_INTERVALS: [u32; 7] = [1, 2, 4, 6, 8, 4, 8];

#[derive(Debug)]
struct StaticSource {
    snapshot: MarketDataSnapshot,
}

impl OpportunityMarketDataSource for StaticSource {
    fn market_snapshot(&self) -> MarketDataSnapshot {
        self.snapshot.clone()
    }
}

#[test]
fn scan_fixture_stays_within_candidate_budget_and_p0_scope() {
    let engine = ArbitrageEngineV3::new(
        config(),
        Arc::new(StaticSource {
            snapshot: market_snapshot(),
        }),
        100_000.0,
        0.5,
    );

    let started_at = Instant::now();
    let rows = engine.run_scan();
    let elapsed = started_at.elapsed();

    assert!(!rows.is_empty());
    assert!(
        rows.len() <= MAX_SCAN_ROWS,
        "scan rows {} exceeded budget {}",
        rows.len(),
        MAX_SCAN_ROWS
    );
    assert!(
        elapsed <= MAX_SCAN_DURATION,
        "scan took {:?}, expected <= {:?}",
        elapsed,
        MAX_SCAN_DURATION
    );
    assert!(rows
        .windows(2)
        .all(|window| profit_order(&window[0], &window[1]) != Ordering::Greater));
    assert!(rows.iter().all(|row| matches!(
        row.strategy_kind,
        Some(StrategyKind::PerpCross | StrategyKind::SpotPerp | StrategyKind::CrossSpotPerp)
    )));
}

fn profit_order(left: &ArbitrageOpportunityDto, right: &ArbitrageOpportunityDto) -> Ordering {
    right
        .execution_eligible
        .cmp(&left.execution_eligible)
        .then_with(|| verified_net(right).total_cmp(&verified_net(left)))
        .then_with(|| right.net_single_yield.total_cmp(&left.net_single_yield))
        .then_with(|| left.id.cmp(&right.id))
}

fn verified_net(row: &ArbitrageOpportunityDto) -> f64 {
    row.execution_cost
        .as_ref()
        .filter(|cost| cost.one_cycle.covers_round_trip_cost)
        .filter(|cost| {
            cost.round_trip.as_ref().is_some_and(|round_trip| {
                round_trip.profitability_evidence.is_cost_verified()
                    && round_trip.profitability_evidence.fee_evidence_ids.len() >= 2
            })
        })
        .map(|cost| cost.one_cycle.net_bps)
        .filter(|value| value.is_finite())
        .unwrap_or(f64::NEG_INFINITY)
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
    let observed_at_ms = observed_at_ms();
    MarketDataSnapshot {
        funding: funding_snapshot(observed_at_ms).into(),
        perp_tickers: perp_ticker_snapshot(observed_at_ms).into(),
        ..MarketDataSnapshot::default()
    }
}

fn observed_at_ms() -> i64 {
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(i64::MAX - HOUR_MS)
}

fn funding_snapshot(observed_at_ms: i64) -> HashMap<String, HashMap<String, FundingRateData>> {
    let next_funding_time = observed_at_ms.saturating_add(HOUR_MS);
    let mut out = HashMap::with_capacity(SYMBOLS);
    for idx in 0..SYMBOLS {
        let symbol = format!("SYM{idx:03}");
        let mut venues = HashMap::with_capacity(VENUES.len());
        for (venue_idx, venue) in VENUES.iter().enumerate() {
            venues.insert(
                (*venue).to_owned(),
                funding_row(
                    &symbol,
                    venue,
                    idx,
                    venue_idx,
                    observed_at_ms,
                    next_funding_time,
                ),
            );
        }
        out.insert(symbol, venues);
    }
    out
}

fn perp_ticker_snapshot(observed_at_ms: i64) -> Vec<TickerInfo> {
    let mut out = Vec::with_capacity(SYMBOLS * VENUES.len());
    for idx in 0..SYMBOLS {
        let symbol = format!("SYM{idx:03}");
        let price = 100.0 + idx as f64;
        for venue in VENUES {
            out.push(TickerInfo {
                symbol: symbol.clone(),
                exchange: venue.to_owned(),
                bid: price,
                ask: price,
                last: price,
                volume_24h: 250_000.0 + idx as f64 * 100.0,
                timestamp: observed_at_ms,
            });
        }
    }
    out
}

fn funding_row(
    symbol: &str,
    venue: &str,
    symbol_idx: usize,
    venue_idx: usize,
    observed_at_ms: i64,
    next_funding_time: i64,
) -> FundingRateData {
    let phase = ((symbol_idx + venue_idx) % 11) as f64 - 5.0;
    let rate = phase * 0.000_01 + venue_idx as f64 * 0.001;
    let funding_interval = NATIVE_INTERVALS[venue_idx];
    FundingRateData {
        symbol: symbol.to_owned(),
        exchange: venue.to_owned(),
        rate,
        rate_8h: rate * 8.0 / f64::from(funding_interval),
        predicted_rate: None,
        next_funding_time,
        funding_interval,
        volume_24h: 250_000.0 + symbol_idx as f64 * 100.0,
        timestamp: observed_at_ms,
        smoothed_rate: None,
        rate_std: None,
        is_outlier: false,
    }
}
