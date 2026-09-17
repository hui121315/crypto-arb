use super::*;
use shared_types::FundingPaymentData;

pub(super) fn non_empty_text(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

pub(super) fn valid_fill_value(value: f64) -> bool {
    value.is_finite() && value > 0.0
}

pub(super) fn order_side_from_text(value: &str) -> Option<OrderSide> {
    match value.trim().to_ascii_lowercase().as_str() {
        "buy" | "b" => Some(OrderSide::Buy),
        "sell" | "s" | "a" => Some(OrderSide::Sell),
        _ => None,
    }
}

pub(crate) fn funding_payment_delta(row: FundingPaymentData) -> Option<PrivateFundingDelta> {
    let symbol = row.symbol.trim();
    let currency = row.currency.trim();
    if symbol.is_empty()
        || currency.is_empty()
        || !row.amount.is_finite()
        || row.amount == 0.0
        || row.funding_time_ms <= 0
        || row.venue_event_id.trim().is_empty()
    {
        return None;
    }
    Some(PrivateFundingDelta {
        venue: row.venue,
        venue_event_id: row.venue_event_id,
        coin: symbol.to_owned(),
        amount: row.amount,
        currency: currency.to_owned(),
        occurred_at_ms: row.funding_time_ms,
    })
}
