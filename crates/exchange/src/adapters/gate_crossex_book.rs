//! Sequence-checked state for `CrossEx` incremental order books.

use super::gate_crossex_data::CrossExBookDelta;
use super::gate_crossex_symbols::CrossExRoute;
use rust_decimal::Decimal;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MergeOutcome {
    Applied,
    Duplicate,
    Gap,
}

#[derive(Debug, Clone)]
pub(super) struct CrossExBookState {
    route: CrossExRoute,
    bids: BTreeMap<Decimal, Decimal>,
    asks: BTreeMap<Decimal, Decimal>,
    sequence: u64,
    timestamp: i64,
}

impl CrossExBookState {
    pub(super) fn from_snapshot(snapshot: &CrossExBookDelta) -> Option<Self> {
        if !snapshot.snapshot {
            return None;
        }
        let mut state = Self {
            route: snapshot.route.clone(),
            bids: BTreeMap::new(),
            asks: BTreeMap::new(),
            sequence: snapshot.sequence_end,
            timestamp: snapshot.timestamp,
        };
        merge_levels(&mut state.bids, &snapshot.bids);
        merge_levels(&mut state.asks, &snapshot.asks);
        Some(state)
    }

    pub(super) fn apply(&mut self, delta: &CrossExBookDelta) -> MergeOutcome {
        if delta.sequence_end <= self.sequence {
            return MergeOutcome::Duplicate;
        }
        if delta.sequence_start > self.sequence.saturating_add(1) {
            return MergeOutcome::Gap;
        }
        merge_levels(&mut self.bids, &delta.bids);
        merge_levels(&mut self.asks, &delta.asks);
        self.sequence = delta.sequence_end;
        self.timestamp = delta.timestamp;
        MergeOutcome::Applied
    }

    pub(super) fn timestamp(&self) -> i64 {
        self.timestamp
    }

    pub(super) fn snapshot(
        &self,
        depth: usize,
        quantity_multiplier: Decimal,
    ) -> shared_types::OrderBookInfo {
        let bids = self
            .bids
            .iter()
            .rev()
            .take(depth.max(1))
            .filter_map(|(price, quantity)| decimal_level(*price, *quantity, quantity_multiplier))
            .collect();
        let asks = self
            .asks
            .iter()
            .take(depth.max(1))
            .filter_map(|(price, quantity)| decimal_level(*price, *quantity, quantity_multiplier))
            .collect();
        shared_types::OrderBookInfo {
            symbol: self.route.base.clone(),
            exchange: self.route.venue(),
            bids,
            asks,
            timestamp: self.timestamp,
        }
    }
}

fn merge_levels(book: &mut BTreeMap<Decimal, Decimal>, levels: &[[Decimal; 2]]) {
    for [price, quantity] in levels {
        if quantity.is_zero() {
            book.remove(price);
        } else {
            book.insert(*price, *quantity);
        }
    }
}

fn decimal_level(price: Decimal, quantity: Decimal, multiplier: Decimal) -> Option<[f64; 2]> {
    Some([
        price.to_string().parse().ok()?,
        (quantity * multiplier).to_string().parse().ok()?,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn delta(snapshot: bool, start: u64, end: u64, quantity: i64) -> CrossExBookDelta {
        CrossExBookDelta {
            route: CrossExRoute::parse("KRAKEN_FUTURE_BTC_USD").unwrap(),
            snapshot,
            sequence_start: start,
            sequence_end: end,
            bids: vec![[Decimal::new(100, 0), Decimal::new(quantity, 0)]],
            asks: vec![[Decimal::new(101, 0), Decimal::ONE]],
            timestamp: i64::try_from(end).unwrap(),
        }
    }

    #[test]
    fn requires_snapshot_then_accepts_only_contiguous_updates() {
        assert!(CrossExBookState::from_snapshot(&delta(false, 1, 1, 1)).is_none());
        let mut state = CrossExBookState::from_snapshot(&delta(true, 1, 4, 2)).unwrap();
        assert_eq!(state.apply(&delta(false, 5, 6, 3)), MergeOutcome::Applied);
        assert_eq!(state.apply(&delta(false, 5, 6, 3)), MergeOutcome::Duplicate);
        assert_eq!(state.apply(&delta(false, 8, 8, 4)), MergeOutcome::Gap);
        assert_eq!(state.snapshot(1, Decimal::ONE).bids[0], [100.0, 3.0]);
    }
}
