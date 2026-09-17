use super::*;

#[derive(Clone, Copy)]
pub(super) enum QuoteTarget {
    Notional(f64),
    BaseQuantity(f64),
}

impl QuoteTarget {
    pub(super) fn vwap(self, book: &OrderBookInfo, side: OrderSide) -> Option<f64> {
        match self {
            Self::Notional(notional) => vwap_price_for_notional(book, side, notional),
            Self::BaseQuantity(quantity) => vwap_price_for_base_quantity(book, side, quantity),
        }
    }

    pub(super) fn required_notional(self, vwap: Option<f64>) -> Option<f64> {
        match self {
            Self::Notional(notional) => Some(notional),
            Self::BaseQuantity(quantity) => vwap
                .map(|price| quantity * price)
                .filter(|value| value.is_finite()),
        }
    }
}
