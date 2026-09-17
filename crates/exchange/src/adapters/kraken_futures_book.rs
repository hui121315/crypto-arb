//! Sequence-checked Kraken Derivatives order-book state.

use super::kraken_futures_data::{order_book, BookSide, FuturesBookDelta, FuturesBookSnapshot};
use rust_decimal::Decimal;
use shared_types::OrderBookInfo;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MergeOutcome {
    Applied,
    Duplicate,
    Gap,
}

#[derive(Debug, Clone)]
pub(super) struct FuturesBookState {
    native_symbol: String,
    bids: BTreeMap<Decimal, Decimal>,
    asks: BTreeMap<Decimal, Decimal>,
    sequence: u64,
    timestamp_ms: i64,
}

impl FuturesBookState {
    pub(super) fn from_snapshot(snapshot: FuturesBookSnapshot) -> Self {
        Self {
            native_symbol: snapshot.product_id,
            bids: snapshot.bids.into_iter().collect(),
            asks: snapshot.asks.into_iter().collect(),
            sequence: snapshot.sequence,
            timestamp_ms: snapshot.timestamp_ms,
        }
    }

    pub(super) fn apply(&mut self, delta: &FuturesBookDelta) -> MergeOutcome {
        if delta.sequence <= self.sequence {
            return MergeOutcome::Duplicate;
        }
        if delta.sequence != self.sequence + 1 || delta.product_id != self.native_symbol {
            return MergeOutcome::Gap;
        }
        let side = match delta.side {
            BookSide::Buy => &mut self.bids,
            BookSide::Sell => &mut self.asks,
        };
        if delta.quantity.is_zero() {
            side.remove(&delta.price);
        } else {
            side.insert(delta.price, delta.quantity);
        }
        self.sequence = delta.sequence;
        self.timestamp_ms = delta.timestamp_ms;
        MergeOutcome::Applied
    }

    pub(super) fn snapshot(&self, depth: usize, contract_multiplier: Decimal) -> OrderBookInfo {
        order_book(
            &self.native_symbol,
            self.bids.iter().rev().map(|(price, qty)| (*price, *qty)),
            self.asks.iter().map(|(price, qty)| (*price, *qty)),
            self.timestamp_ms,
            depth,
            contract_multiplier,
        )
    }

    pub(super) fn timestamp_ms(&self) -> i64 {
        self.timestamp_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::kraken_futures_data::{parse_book_frame, FuturesBookFrame};
    use serde_json::Value;

    fn fixture_frames() -> (FuturesBookSnapshot, FuturesBookDelta) {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../fixtures/kraken/futures_book_pf_xbtusd.json"
        ))
        .expect("fixture json");
        let FuturesBookFrame::Snapshot(snapshot) =
            parse_book_frame(&fixture["snapshot"].to_string()).expect("snapshot")
        else {
            panic!("expected snapshot");
        };
        let FuturesBookFrame::Delta(delta) =
            parse_book_frame(&fixture["delta"].to_string()).expect("delta")
        else {
            panic!("expected delta");
        };
        (snapshot, delta)
    }

    #[test]
    fn applies_contiguous_delta_and_rejects_gap() {
        let (snapshot, delta) = fixture_frames();
        let mut state = FuturesBookState::from_snapshot(snapshot);
        assert_eq!(state.apply(&delta), MergeOutcome::Applied);
        assert_eq!(state.apply(&delta), MergeOutcome::Duplicate);

        let mut gap = delta;
        gap.sequence += 2;
        assert_eq!(state.apply(&gap), MergeOutcome::Gap);
    }
}
