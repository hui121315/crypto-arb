//! Kraken Spot WebSocket v2 L2 book state and CRC32 validation.

use super::kraken_spot_data::{order_book, SpotBookUpdate};
use rust_decimal::Decimal;
use shared_types::OrderBookInfo;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MergeOutcome {
    Applied,
    ChecksumMismatch,
    MissingSnapshot,
}

#[derive(Debug, Clone)]
pub(super) struct SpotBookState {
    native_symbol: String,
    bids: BTreeMap<Decimal, Decimal>,
    asks: BTreeMap<Decimal, Decimal>,
    depth: usize,
    timestamp_ms: i64,
}

impl SpotBookState {
    pub(super) fn from_snapshot(update: &SpotBookUpdate, depth: usize) -> Option<Self> {
        if !update.snapshot {
            return None;
        }
        let mut state = Self {
            native_symbol: update.symbol.clone(),
            bids: update.bids.iter().copied().collect(),
            asks: update.asks.iter().copied().collect(),
            depth,
            timestamp_ms: update.timestamp_ms,
        };
        state.truncate();
        (state.checksum() == update.checksum).then_some(state)
    }

    pub(super) fn apply(&mut self, update: &SpotBookUpdate) -> MergeOutcome {
        if update.snapshot || update.symbol != self.native_symbol {
            return MergeOutcome::MissingSnapshot;
        }
        apply_side(&mut self.bids, &update.bids);
        apply_side(&mut self.asks, &update.asks);
        self.timestamp_ms = update.timestamp_ms;
        self.truncate();
        if self.checksum() == update.checksum {
            MergeOutcome::Applied
        } else {
            MergeOutcome::ChecksumMismatch
        }
    }

    pub(super) fn snapshot(&self, depth: usize) -> OrderBookInfo {
        order_book(
            &self.native_symbol,
            self.bids.iter().rev().map(|(price, qty)| (*price, *qty)),
            self.asks.iter().map(|(price, qty)| (*price, *qty)),
            self.timestamp_ms,
            depth.min(self.depth),
        )
    }

    pub(super) fn timestamp_ms(&self) -> i64 {
        self.timestamp_ms
    }

    fn truncate(&mut self) {
        while self.bids.len() > self.depth {
            let Some(key) = self.bids.first_key_value().map(|(key, _)| *key) else {
                break;
            };
            self.bids.remove(&key);
        }
        while self.asks.len() > self.depth {
            let Some(key) = self.asks.last_key_value().map(|(key, _)| *key) else {
                break;
            };
            self.asks.remove(&key);
        }
    }

    fn checksum(&self) -> u32 {
        let mut input = String::new();
        for (price, quantity) in self.asks.iter().take(10) {
            append_checksum_level(&mut input, *price, *quantity);
        }
        for (price, quantity) in self.bids.iter().rev().take(10) {
            append_checksum_level(&mut input, *price, *quantity);
        }
        crc32fast::hash(input.as_bytes())
    }
}

fn apply_side(side: &mut BTreeMap<Decimal, Decimal>, updates: &[(Decimal, Decimal)]) {
    for (price, quantity) in updates {
        if quantity.is_zero() {
            side.remove(price);
        } else {
            side.insert(*price, *quantity);
        }
    }
}

fn append_checksum_level(input: &mut String, price: Decimal, quantity: Decimal) {
    input.push_str(&checksum_component(price));
    input.push_str(&checksum_component(quantity));
}

fn checksum_component(value: Decimal) -> String {
    let digits = value.to_string().replace('.', "");
    let trimmed = digits.trim_start_matches('0');
    if trimmed.is_empty() {
        "0".to_owned()
    } else {
        trimmed.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::kraken_spot_data::parse_book_frame;

    #[test]
    fn validates_kraken_official_top_ten_checksum() {
        let fixture = include_str!("../../fixtures/kraken/spot_v2_book_btcusd.json");
        let update = parse_book_frame(fixture)
            .expect("fixture parses")
            .into_iter()
            .next()
            .expect("book row");
        let state = SpotBookState::from_snapshot(&update, 10).expect("checksum matches");
        let book = state.snapshot(10);
        assert_eq!(book.symbol, "BTC/USD");
        assert_eq!(book.bids.len(), 10);
        assert_eq!(book.asks.len(), 10);
        assert_eq!(book.best_bid(), Some(45_283.5));
        assert_eq!(book.best_ask(), Some(45_285.2));
    }

    #[test]
    fn rejects_corrupt_checksum() {
        let fixture = include_str!("../../fixtures/kraken/spot_v2_book_btcusd.json");
        let mut update = parse_book_frame(fixture).expect("fixture parses").remove(0);
        update.checksum = update.checksum.wrapping_add(1);
        assert!(SpotBookState::from_snapshot(&update, 10).is_none());
    }
}
