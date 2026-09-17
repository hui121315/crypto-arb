use super::*;

pub(super) fn filled_notional(snapshot: &FillLedgerSnapshot) -> Option<f64> {
    if snapshot.quality == shared_types::ExecutionLedgerQuality::Missing {
        return None;
    }
    if is_positive(snapshot.quote_value) {
        return Some(snapshot.quote_value);
    }
    if is_positive(snapshot.quantity) && is_positive(snapshot.average_price) {
        return Some(snapshot.quantity * snapshot.average_price);
    }
    None
}

pub(super) fn fee_amount(snapshot: &FeeLedgerSnapshot) -> Option<f64> {
    (snapshot.quality != shared_types::ExecutionLedgerQuality::Missing
        && snapshot.amount.is_finite())
    .then_some(snapshot.amount)
}

pub(super) fn funding_amount(payment: &FundingPaymentLedgerRecord) -> Option<f64> {
    (payment.quality != shared_types::ExecutionLedgerQuality::Missing && payment.amount.is_finite())
        .then_some(payment.amount)
}

pub(super) fn durable_slippage_amount(record: &SlippageLedgerRecord) -> Option<f64> {
    (record.quality != shared_types::ExecutionLedgerQuality::Missing
        && record.amount_usd.is_finite())
    .then_some(record.amount_usd)
}

fn fill_quantity(snapshot: &FillLedgerSnapshot) -> Option<f64> {
    if is_positive(snapshot.quantity) {
        return Some(snapshot.quantity);
    }
    if is_positive(snapshot.quote_value) && is_positive(snapshot.average_price) {
        return Some(snapshot.quote_value / snapshot.average_price);
    }
    None
}

fn fill_price(snapshot: &FillLedgerSnapshot) -> Option<f64> {
    if is_positive(snapshot.average_price) {
        return Some(snapshot.average_price);
    }
    if is_positive(snapshot.quote_value) && is_positive(snapshot.quantity) {
        return Some(snapshot.quote_value / snapshot.quantity);
    }
    None
}

pub(super) fn positive_price(price: Option<f64>) -> Option<f64> {
    price.filter(|value| is_positive(*value))
}

pub(super) fn slippage_usd(
    side: OrderSide,
    reference_price: f64,
    fill: &FillLedgerSnapshot,
) -> Option<f64> {
    let quantity = fill_quantity(fill)?;
    let fill_price = fill_price(fill)?;
    let delta = match side {
        OrderSide::Buy => fill_price - reference_price,
        OrderSide::Sell => reference_price - fill_price,
    };
    (delta.is_finite() && quantity.is_finite()).then_some(delta * quantity)
}

fn is_positive(value: f64) -> bool {
    value.is_finite() && value > 0.0
}

pub(super) fn hedge_group_id(id: &str) -> String {
    id.strip_suffix("-long")
        .or_else(|| id.strip_suffix("-short"))
        .or_else(|| id.strip_suffix("-unwind"))
        .unwrap_or(id)
        .to_owned()
}

pub(super) fn day_start_ms(ts: i64) -> i64 {
    ts - ts.rem_euclid(DAY_MS)
}

pub(super) fn holding_minutes(opened_at_ms: i64, closed_at_ms: i64) -> Option<u32> {
    let elapsed = closed_at_ms.checked_sub(opened_at_ms)?;
    u32::try_from(elapsed / 60_000).ok()
}
