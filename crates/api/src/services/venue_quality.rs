use crate::state::AppState;
use shared_types::{VenueQualityEnvelope, VenueQualitySampleStatus, VenueQualitySource};

mod aggregate;

use aggregate::quality_rows;

pub(crate) fn snapshot_envelope(state: &AppState) -> VenueQualityEnvelope {
    let now_ms = common::time::now_ms();
    let operation_health = crate::services::venue_operation_health::snapshot(state);
    let rows = quality_rows(
        state.aggregator().names(),
        state.venue_quality().snapshot().as_ref().clone(),
        exchange::http_quality_window_snapshot(),
        operation_health.rows,
    );
    let source = if rows
        .iter()
        .any(|row| row.sample_status != VenueQualitySampleStatus::NoSample)
    {
        VenueQualitySource::RuntimeSamples
    } else {
        VenueQualitySource::NeutralNoSample
    };
    VenueQualityEnvelope::new(rows, now_ms, source).with_request_id(common::request_id::current())
}

#[cfg(test)]
#[path = "venue_quality/tests.rs"]
mod tests;
