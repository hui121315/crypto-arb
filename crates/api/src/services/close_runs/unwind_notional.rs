use super::*;

#[derive(Debug, Clone, PartialEq)]
pub(super) struct UnwindNotionalEvidence {
    pub(super) amount_usd: f64,
    pub(super) quality: ExecutionLedgerQuality,
    pub(super) source: String,
    pub(super) missing_fields: Vec<String>,
}

pub(super) fn leg_confirmed_price(leg: &CloseLeg) -> Option<f64> {
    let price = leg.order.as_ref()?.filled_price?;
    (price.is_finite() && price > 0.0).then_some(price)
}

pub(super) fn unwind_leg_notional(
    leg: &CloseLeg,
    confirmed_quantity: Option<f64>,
    confirmed_price: Option<f64>,
) -> UnwindNotionalEvidence {
    match (confirmed_quantity, confirmed_price) {
        (Some(quantity), Some(price)) => notional_evidence(
            quantity.abs() * price,
            ExecutionLedgerQuality::Actual,
            "filled_quantity_x_filled_price",
            Vec::new(),
        ),
        (Some(quantity), None) => notional_from_known_quantity(leg, quantity),
        (None, Some(price)) => notional_from_known_price(leg, price),
        (None, None) => notional_from_estimates(leg),
    }
}

fn notional_from_known_quantity(leg: &CloseLeg, quantity: f64) -> UnwindNotionalEvidence {
    if let Some(mark_price) = positive_finite(leg.mark_price) {
        return notional_evidence(
            quantity.abs() * mark_price,
            ExecutionLedgerQuality::Estimated,
            "filled_quantity_x_mark_price",
            vec!["filled_price".to_owned()],
        );
    }
    fallback_notional(
        leg,
        vec!["filled_price".to_owned(), "mark_price".to_owned()],
    )
}

fn notional_from_known_price(leg: &CloseLeg, price: f64) -> UnwindNotionalEvidence {
    if let Some(quantity) = positive_finite(leg.quantity.abs()) {
        return notional_evidence(
            quantity * price,
            ExecutionLedgerQuality::Estimated,
            "target_quantity_x_filled_price",
            vec!["filled_quantity".to_owned()],
        );
    }
    fallback_notional(
        leg,
        vec!["filled_quantity".to_owned(), "target_quantity".to_owned()],
    )
}

fn notional_from_estimates(leg: &CloseLeg) -> UnwindNotionalEvidence {
    let target_quantity = positive_finite(leg.quantity.abs());
    let mark_price = positive_finite(leg.mark_price);
    if let (Some(quantity), Some(price)) = (target_quantity, mark_price) {
        return notional_evidence(
            quantity * price,
            ExecutionLedgerQuality::Estimated,
            "target_quantity_x_mark_price",
            vec!["filled_quantity".to_owned(), "filled_price".to_owned()],
        );
    }

    let mut missing = vec!["filled_quantity".to_owned(), "filled_price".to_owned()];
    if target_quantity.is_none() {
        missing.push("target_quantity".to_owned());
    }
    if mark_price.is_none() {
        missing.push("mark_price".to_owned());
    }
    fallback_notional(leg, missing)
}

fn fallback_notional(leg: &CloseLeg, mut missing_fields: Vec<String>) -> UnwindNotionalEvidence {
    if let Some(notional) = positive_finite(leg.notional_usd) {
        return notional_evidence(
            notional,
            ExecutionLedgerQuality::Estimated,
            "leg_notional_usd_fallback",
            missing_fields,
        );
    }
    missing_fields.push("notional_usd".to_owned());
    notional_evidence(
        0.0,
        ExecutionLedgerQuality::Missing,
        "missing_notional",
        missing_fields,
    )
}

fn notional_evidence(
    amount_usd: f64,
    quality: ExecutionLedgerQuality,
    source: &str,
    missing_fields: Vec<String>,
) -> UnwindNotionalEvidence {
    UnwindNotionalEvidence {
        amount_usd,
        quality,
        source: source.to_owned(),
        missing_fields,
    }
}

fn positive_finite(value: f64) -> Option<f64> {
    (value.is_finite() && value > 0.0).then_some(value)
}
