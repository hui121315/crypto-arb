use super::*;
use shared_types::UNRECORDED_EVIDENCE_MARKER;

mod message;
mod search;
mod ws_rtt;

fn operation_row(
    venue: &str,
    operation: &str,
    status: VenueOperationStatus,
) -> VenueOperationHealth {
    VenueOperationHealth {
        venue: venue.to_owned(),
        operation: operation.to_owned(),
        status,
        source: "test".to_owned(),
        message: "sample".to_owned(),
        supported: Some(true),
        configured: Some(true),
        requested: None,
        rows: None,
        freshness_ms: None,
        retry_after_ms: None,
        latency_ms: None,
        latency_p95_ms: None,
        error: None,
        evidence: None,
        problem: None,
        observed_at_ms: 1_000,
    }
}
