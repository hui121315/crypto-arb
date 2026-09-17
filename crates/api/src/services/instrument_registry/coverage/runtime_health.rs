use super::*;
use shared_types::VenueOperationStatus;

impl InstrumentRegistry {
    pub(crate) fn runtime_health(&self, now_ms: i64) -> Vec<InstrumentRegistryRuntimeHealth> {
        SUPPORTED_VENUES
            .iter()
            .map(|venue| self.venue_runtime_health(venue, now_ms.max(1)))
            .collect()
    }

    fn venue_runtime_health(&self, venue: &str, now_ms: i64) -> InstrumentRegistryRuntimeHealth {
        let InstrumentRowsSummary {
            rows,
            execution_ready_rows,
            schema_versions,
            source_urls,
        } = self.venue_rows_summary(venue, now_ms);
        match self.probe_state(venue) {
            Some(VenueProbeState::Restored { checked_at_ms }) => restored_runtime_health(
                venue,
                checked_at_ms,
                now_ms,
                InstrumentRowsSummary {
                    rows,
                    execution_ready_rows,
                    schema_versions,
                    source_urls,
                },
            ),
            Some(VenueProbeState::Success { checked_at_ms }) => {
                let freshness_ms = now_ms.saturating_sub(checked_at_ms).max(0);
                let stale = now_ms < checked_at_ms || freshness_ms >= INSTRUMENT_SPEC_FRESHNESS_MS;
                let status = if rows == 0 {
                    VenueOperationStatus::Blocked
                } else if stale || execution_ready_rows == 0 {
                    VenueOperationStatus::Warn
                } else {
                    VenueOperationStatus::Ok
                };
                InstrumentRegistryRuntimeHealth {
                    venue: venue.to_owned(),
                    status,
                    rows,
                    execution_ready_rows,
                    checked_at_ms,
                    freshness_ms: Some(freshness_ms),
                    schema_versions,
                    source_urls,
                    message: if rows == 0 {
                        "official instrument refresh produced no accepted rows".to_owned()
                    } else if stale {
                        format!("official instrument specs are stale; retained {rows} cached rows")
                    } else if execution_ready_rows == 0 {
                        format!(
                            "official instrument registry has {rows} rows but no execution-ready specs"
                        )
                    } else {
                        format!(
                            "official instrument registry contains {execution_ready_rows}/{rows} execution-ready rows"
                        )
                    },
                    problem: None,
                }
            }
            Some(VenueProbeState::Failed {
                checked_at_ms,
                problem,
            }) => InstrumentRegistryRuntimeHealth {
                venue: venue.to_owned(),
                status: if rows == 0 {
                    VenueOperationStatus::Blocked
                } else {
                    VenueOperationStatus::Warn
                },
                rows,
                execution_ready_rows: 0,
                checked_at_ms,
                freshness_ms: Some(now_ms.saturating_sub(checked_at_ms).max(0)),
                schema_versions,
                source_urls,
                message: format!("instrument refresh failed; retained {rows} cached rows"),
                problem: Some(problem),
            },
            Some(VenueProbeState::Unsupported {
                checked_at_ms,
                problem,
            }) => InstrumentRegistryRuntimeHealth {
                venue: venue.to_owned(),
                status: VenueOperationStatus::Unsupported,
                rows,
                execution_ready_rows: 0,
                checked_at_ms,
                freshness_ms: Some(now_ms.saturating_sub(checked_at_ms).max(0)),
                schema_versions,
                source_urls,
                message: "adapter does not implement official instrument specs".to_owned(),
                problem: Some(problem),
            },
            None => InstrumentRegistryRuntimeHealth {
                venue: venue.to_owned(),
                status: VenueOperationStatus::Unknown,
                rows,
                execution_ready_rows: 0,
                checked_at_ms: now_ms,
                freshness_ms: None,
                schema_versions,
                source_urls,
                message: "instrument registry has not completed its first probe".to_owned(),
                problem: None,
            },
        }
    }

    fn venue_rows_summary(&self, venue: &str, now_ms: i64) -> InstrumentRowsSummary {
        let lookup = self.instrument_lookup_snapshot();
        let Some(summary) = lookup.health(venue) else {
            return InstrumentRowsSummary::default();
        };
        InstrumentRowsSummary {
            rows: summary.rows(),
            execution_ready_rows: summary
                .execution_ready_rows_at(now_ms, INSTRUMENT_SPEC_FRESHNESS_MS),
            schema_versions: summary.schema_versions(),
            source_urls: summary.source_urls(),
        }
    }
}

#[derive(Default)]
struct InstrumentRowsSummary {
    rows: usize,
    execution_ready_rows: usize,
    schema_versions: Vec<String>,
    source_urls: Vec<String>,
}

fn restored_runtime_health(
    venue: &str,
    checked_at_ms: i64,
    now_ms: i64,
    summary: InstrumentRowsSummary,
) -> InstrumentRegistryRuntimeHealth {
    InstrumentRegistryRuntimeHealth {
        venue: venue.to_owned(),
        status: VenueOperationStatus::Warn,
        rows: summary.rows,
        execution_ready_rows: 0,
        checked_at_ms,
        freshness_ms: Some(now_ms.saturating_sub(checked_at_ms).max(0)),
        schema_versions: summary.schema_versions,
        source_urls: summary.source_urls,
        message: format!(
            "restored {} historical instrument rows; fresh official probe required",
            summary.rows
        ),
        problem: None,
    }
}
