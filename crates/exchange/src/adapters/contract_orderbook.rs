//! Normalize derivative order-book quantities into base-asset units.

use crate::error::{ExchangeError, ExchangeResult};
use shared_types::OrderBookInfo;

pub(super) fn normalize_contract_book(
    mut book: OrderBookInfo,
    contract_size: f64,
) -> ExchangeResult<OrderBookInfo> {
    if !valid_positive(contract_size) {
        return Err(ExchangeError::Parse(format!(
            "{} orderbook has invalid contract size {contract_size}",
            book.exchange
        )));
    }
    normalize_levels(&mut book.bids, contract_size, &book.exchange)?;
    normalize_levels(&mut book.asks, contract_size, &book.exchange)?;
    Ok(book)
}

fn normalize_levels(
    levels: &mut [[f64; 2]],
    contract_size: f64,
    exchange: &str,
) -> ExchangeResult<()> {
    for level in levels {
        let base_quantity = level[1] * contract_size;
        if !valid_positive(base_quantity) {
            return Err(ExchangeError::Parse(format!(
                "{exchange} orderbook quantity {} cannot be normalized with contract size {contract_size}",
                level[1]
            )));
        }
        level[1] = base_quantity;
    }
    Ok(())
}

fn valid_positive(value: f64) -> bool {
    value.is_finite() && value > 0.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract_counts_are_exposed_as_base_asset_quantity() {
        let book = OrderBookInfo {
            symbol: "BTC".into(),
            exchange: "venue".into(),
            bids: vec![[100_000.0, 25.0]],
            asks: vec![[100_001.0, 40.0]],
            timestamp: 1,
        };

        let normalized = normalize_contract_book(book, 0.001).expect("valid contract size");

        assert_eq!(normalized.bids, vec![[100_000.0, 0.025]]);
        assert_eq!(normalized.asks, vec![[100_001.0, 0.04]]);
    }

    #[test]
    fn invalid_contract_size_fails_closed() {
        let book = OrderBookInfo {
            symbol: "BTC".into(),
            exchange: "venue".into(),
            bids: vec![[100_000.0, 25.0]],
            asks: vec![[100_001.0, 40.0]],
            timestamp: 1,
        };

        assert!(normalize_contract_book(book, 0.0).is_err());
    }
}
