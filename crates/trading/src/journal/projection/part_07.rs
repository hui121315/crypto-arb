fn apply_ack_fill_fields(record: &mut OrderRecord, ack: &OrderAck) {
    if is_positive_option(ack.filled_quantity) {
        record.filled_quantity = ack.filled_quantity;
    }
    if is_positive_option(ack.filled_price) {
        record.filled_price = ack.filled_price;
    }
    if ack
        .filled_fee
        .is_some_and(|fee| fee.is_finite() && fee.abs() > f64::EPSILON)
    {
        record.filled_fee = ack.filled_fee;
    }
}

fn ack_has_fill_evidence(ack: &OrderAck) -> bool {
    is_positive_option(ack.filled_quantity) && is_positive_option(ack.filled_price)
}

fn is_positive_option(value: Option<f64>) -> bool {
    value.is_some_and(|value| value.is_finite() && value > 0.0)
}

fn positive_option(value: Option<f64>) -> Option<f64> {
    value.filter(|candidate| candidate.is_finite() && *candidate > 0.0)
}

fn positive_value(value: f64) -> Option<f64> {
    (value.is_finite() && value > 0.0).then_some(value)
}

fn is_open_state(state: LiveOrderState) -> bool {
    matches!(
        state,
        LiveOrderState::Created
            | LiveOrderState::RiskChecked
            | LiveOrderState::Submitted
            | LiveOrderState::Accepted
            | LiveOrderState::PartiallyFilled
            | LiveOrderState::CancelRequested
            | LiveOrderState::Unknown
    )
}

fn live_state_from_order_status(status: OrderStatus) -> LiveOrderState {
    match status {
        OrderStatus::Pending | OrderStatus::Open => LiveOrderState::Accepted,
        OrderStatus::PartiallyFilled => LiveOrderState::PartiallyFilled,
        OrderStatus::Filled => LiveOrderState::Filled,
        OrderStatus::Canceled => LiveOrderState::Cancelled,
        OrderStatus::Rejected => LiveOrderState::Rejected,
        OrderStatus::Expired => LiveOrderState::Failed,
    }
}

#[cfg(test)]
#[path = "../funding_tests.rs"]
mod funding_tests;
#[cfg(test)]
#[path = "../funding_replay_tests.rs"]
mod funding_replay_tests;
#[cfg(test)]
#[path = "../orderbook_tests.rs"]
mod orderbook_tests;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
