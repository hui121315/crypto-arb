use shared_types::{ExecutionRun, ExecutionRunLeg, OrderRecord, VenueOrderIdentity};
use std::collections::BTreeSet;

pub(crate) fn leg_contains_order(leg: &ExecutionRunLeg, order: &OrderRecord) -> bool {
    leg.exchange == order.intent.exchange
        && leg.symbol == order.intent.symbol
        && (leg
            .identity
            .as_ref()
            .is_some_and(|identity| identity.internal_order_id == order.intent.id)
            || leg.order_ids.contains(&order.intent.id))
        && !leg.identity.as_ref().is_some_and(|identity| {
            identity.internal_order_id != order.intent.id && is_alias(identity, &order.intent.id)
        })
}

fn is_alias(identity: &VenueOrderIdentity, id: &str) -> bool {
    !id.is_empty()
        && (identity.public_client_order_id == id
            || identity.venue_client_order_id.as_deref() == Some(id)
            || identity.exchange_order_id.as_deref() == Some(id))
}

pub(crate) fn leg_order_ids(leg: &ExecutionRunLeg, rows: &[OrderRecord]) -> BTreeSet<String> {
    let identities = leg
        .identity
        .iter()
        .cloned()
        .chain(
            rows.iter()
                .filter(|row| leg_contains_order(leg, row))
                .map(OrderRecord::identity_snapshot),
        )
        .filter(|identity| !identity.internal_order_id.trim().is_empty())
        .collect::<Vec<_>>();
    let internal = identities
        .iter()
        .map(|identity| identity.internal_order_id.clone())
        .collect::<BTreeSet<_>>();
    // The backend's order_ids contains aliases as well as distinct recovery orders.
    // Keep unknown IDs for bounded reads; remove only aliases proved by a typed identity.
    leg.order_ids
        .iter()
        .cloned()
        .chain(internal.iter().cloned())
        .filter(|id| !id.trim().is_empty())
        .filter(|id| {
            internal.contains(id) || !identities.iter().any(|identity| is_alias(identity, id))
        })
        .collect()
}

pub(crate) fn run_order_ids(run: &ExecutionRun, rows: &[OrderRecord]) -> BTreeSet<String> {
    leg_order_ids(&run.long_leg, rows)
        .into_iter()
        .chain(leg_order_ids(&run.short_leg, rows))
        .collect()
}
