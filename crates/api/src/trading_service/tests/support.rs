use super::*;

mod orders;
mod services;

pub(super) use orders::*;
pub(super) use services::*;

pub(super) fn seed_accepted_order(
    service: &TradingService,
    internal_id: &str,
    exchange_order_id: &str,
    quantity: f64,
) {
    let mut intent = limit_intent(internal_id);
    intent.quantity = quantity;
    service.journal.insert_created(intent.clone(), 1);
    assert!(service
        .journal
        .mark_risk_checked(&intent.id, RiskDecision::allow(quantity * 50_000.0), 2)
        .is_some());
    assert!(service.journal.mark_submitted(&intent.id, 3).is_some());
    assert!(service
        .journal
        .apply_ack(&OrderAck {
            internal_order_id: intent.id,
            exchange_order_id: Some(exchange_order_id.into()),
            client_order_id: intent.client_order_id,
            identity_update: Default::default(),
            state: LiveOrderState::Accepted,
            accepted_at_ms: 4,
            message: None,
            filled_quantity: None,
            filled_price: None,
            filled_fee: None,
        })
        .is_some());
}

pub(super) fn seed_accepted_order_with_venue_client_id(
    service: &TradingService,
    internal_id: &str,
    exchange_order_id: &str,
    venue_client_order_id: &str,
    quantity: f64,
) {
    let mut intent = limit_intent(internal_id);
    intent.quantity = quantity;
    let public_client_order_id = intent.client_order_id.clone();
    service.journal.insert_created(intent.clone(), 1);
    assert!(service
        .journal
        .mark_risk_checked(&intent.id, RiskDecision::allow(quantity * 50_000.0), 2)
        .is_some());
    assert!(service.journal.mark_submitted(&intent.id, 3).is_some());
    assert!(service
        .journal
        .apply_ack(&OrderAck {
            internal_order_id: intent.id,
            exchange_order_id: Some(exchange_order_id.into()),
            client_order_id: venue_client_order_id.to_owned(),
            identity_update: shared_types::VenueOrderIdentityUpdate::from_ids(
                public_client_order_id,
                venue_client_order_id,
                Some(exchange_order_id.to_owned()),
            ),
            state: LiveOrderState::Accepted,
            accepted_at_ms: 4,
            message: None,
            filled_quantity: None,
            filled_price: None,
            filled_fee: None,
        })
        .is_some());
}

pub(super) fn seed_filled_order(
    service: &TradingService,
    internal_id: &str,
    exchange_order_id: &str,
) {
    let intent = limit_intent(internal_id);
    service.journal.insert_created(intent.clone(), 1);
    assert!(service
        .journal
        .mark_risk_checked(
            &intent.id,
            RiskDecision::allow(intent.quantity * 50_000.0),
            2
        )
        .is_some());
    assert!(service.journal.mark_submitted(&intent.id, 3).is_some());
    assert!(service
        .journal
        .apply_ack(&OrderAck {
            internal_order_id: intent.id,
            exchange_order_id: Some(exchange_order_id.into()),
            client_order_id: intent.client_order_id,
            identity_update: Default::default(),
            state: LiveOrderState::Filled,
            accepted_at_ms: 4,
            message: None,
            filled_quantity: Some(intent.quantity),
            filled_price: intent.price,
            filled_fee: Some(0.1),
        })
        .is_some());
}

pub(super) fn seed_unknown_order(service: &TradingService, internal_id: &str) {
    seed_unknown_order_on(service, internal_id, "mock", 1);
}

pub(super) fn seed_unknown_order_on(
    service: &TradingService,
    internal_id: &str,
    exchange: &str,
    created_at_ms: i64,
) {
    let mut intent = limit_intent(internal_id);
    intent.exchange = exchange.to_owned();
    intent.created_at_ms = created_at_ms;
    service.journal.insert_created(intent.clone(), 1);
    assert!(service
        .journal
        .mark_risk_checked(
            &intent.id,
            RiskDecision::allow(intent.quantity * 50_000.0),
            2,
        )
        .is_some());
    assert!(service.journal.mark_submitted(&intent.id, 3).is_some());
    assert!(service
        .journal
        .update_state(
            &intent.id,
            LiveOrderState::Unknown,
            Some("timeout after 10s".to_owned()),
            4,
        )
        .is_some());
}
