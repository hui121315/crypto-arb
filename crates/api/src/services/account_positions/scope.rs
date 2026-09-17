use super::{operation_health, operation_health_from_rows, ACCOUNT_CACHE_OPERATION};
use crate::state::AppState;
use shared_types::{
    normalized_venue_name, venue_family, VenueOperationHealth, VenueOperationStatus,
};
use std::collections::BTreeSet;

pub(super) fn position_scope_venues(venues: &[String]) -> Vec<String> {
    venues
        .iter()
        .map(|venue| normalized_venue_name(venue_family(venue)))
        .filter(|venue| !venue.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub(super) fn operation_health_for_venues(
    state: &AppState,
    venues: &[String],
) -> Vec<VenueOperationHealth> {
    operation_health_from_rows_for_venues(
        &crate::services::venue_operation_health::snapshot(state).rows,
        venues,
    )
}

pub(super) fn refresh_operation_health_after_read(
    state: &AppState,
    rows: Vec<VenueOperationHealth>,
    venues: Option<&[String]>,
) -> Vec<VenueOperationHealth> {
    if rows.iter().any(configured_cache_needs_refresh) {
        venues.map_or_else(
            || operation_health(state),
            |venues| operation_health_for_venues(state, venues),
        )
    } else {
        rows
    }
}

fn configured_cache_needs_refresh(row: &VenueOperationHealth) -> bool {
    row.operation == ACCOUNT_CACHE_OPERATION
        && row.configured == Some(true)
        && row.status != VenueOperationStatus::Ok
}

pub(super) fn operation_health_from_rows_for_venues(
    rows: &[VenueOperationHealth],
    venues: &[String],
) -> Vec<VenueOperationHealth> {
    let requested = position_scope_venues(venues);
    operation_health_from_rows(rows)
        .into_iter()
        .filter(|row| {
            requested
                .binary_search(&normalized_venue_name(venue_family(&row.venue)))
                .is_ok()
        })
        .collect()
}
