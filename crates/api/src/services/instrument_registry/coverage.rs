use super::{InstrumentRegistry, InstrumentRegistryRuntimeHealth};
use super::{VenueProbeState, SUPPORTED_VENUES};
use shared_types::arbitrage::ArbitrageOpportunityDto;
use shared_types::instrument_coverage::{InstrumentCoverageDiagnostic, InstrumentCoverageEntry};
use shared_types::instrument_coverage::{VenueCoverageEntry, VenueListingState};
use shared_types::instrument_registry::{VenueInstrument, INSTRUMENT_SPEC_FRESHNESS_MS};
use shared_types::instruments::{InstrumentListingStatus, InstrumentMetadataSource};
use shared_types::ApiProblem;
use std::fmt::Write as _;

const LISTING_BLOCKER_PREFIX: &str = "缺交易所挂牌证据或可执行规格：";

fn preferred_coverage_instrument(
    mut instruments: Vec<VenueInstrument>,
    now_ms: i64,
) -> Option<VenueInstrument> {
    instruments.sort_by(|left, right| {
        coverage_candidate_rank(left, now_ms)
            .cmp(&coverage_candidate_rank(right, now_ms))
            .then_with(|| left.native_symbol.cmp(&right.native_symbol))
    });
    instruments.into_iter().next()
}

fn coverage_candidate_rank(instrument: &VenueInstrument, now_ms: i64) -> u8 {
    if instrument.is_hedge_constructible_at(now_ms, INSTRUMENT_SPEC_FRESHNESS_MS) {
        0
    } else if instrument.listing_status == InstrumentListingStatus::Trading
        && instrument.is_fresh_at(now_ms, INSTRUMENT_SPEC_FRESHNESS_MS)
    {
        1
    } else if instrument.listing_status == InstrumentListingStatus::Trading {
        2
    } else {
        3
    }
}

mod evidence;
mod execution_gate;
mod runtime_health;
#[cfg(test)]
mod tests;

use evidence::*;

impl InstrumentRegistry {
    pub(crate) fn begin_instrument_refresh(&self, venue: &str) -> bool {
        use dashmap::mapref::entry::Entry;

        let venue = venue.trim().to_ascii_lowercase();
        match self.instrument_refresh_in_flight.entry(venue) {
            Entry::Vacant(entry) => {
                entry.insert(());
                true
            }
            Entry::Occupied(_) => false,
        }
    }

    pub(crate) fn finish_instrument_refresh(&self, venue: &str) {
        self.instrument_refresh_in_flight
            .remove(&venue.trim().to_ascii_lowercase());
    }

    pub(crate) fn begin_spot_instrument_refresh(&self, venue: &str) -> bool {
        use dashmap::mapref::entry::Entry;

        let venue = venue.trim().to_ascii_lowercase();
        match self.spot_instrument_refresh_in_flight.entry(venue) {
            Entry::Vacant(entry) => {
                entry.insert(());
                true
            }
            Entry::Occupied(_) => false,
        }
    }

    pub(crate) fn finish_spot_instrument_refresh(&self, venue: &str) {
        self.spot_instrument_refresh_in_flight
            .remove(&venue.trim().to_ascii_lowercase());
    }

    pub(crate) fn spot_instrument_refresh_retry_due(
        &self,
        venue: &str,
        now_ms: i64,
        retry_after_ms: i64,
    ) -> bool {
        self.spot_probe_state(venue)
            .is_none_or(|state| match state {
                VenueProbeState::Unsupported { .. } => false,
                VenueProbeState::Restored { .. } => true,
                VenueProbeState::Success { checked_at_ms }
                | VenueProbeState::Failed { checked_at_ms, .. } => {
                    now_ms < checked_at_ms
                        || now_ms.saturating_sub(checked_at_ms) >= retry_after_ms.max(0)
                }
            })
    }

    pub(super) fn record_refresh_success(&self, venue: &str, checked_at_ms: i64) {
        self.probe_by_venue.insert(
            venue.trim().to_ascii_lowercase(),
            VenueProbeState::Success {
                checked_at_ms: checked_at_ms.max(1),
            },
        );
    }

    pub(super) fn record_spot_refresh_success(&self, venue: &str, checked_at_ms: i64) {
        self.spot_probe_by_venue.insert(
            venue.trim().to_ascii_lowercase(),
            VenueProbeState::Success {
                checked_at_ms: checked_at_ms.max(1),
            },
        );
    }

    pub(crate) fn record_refresh_failure(&self, venue: &str, message: impl Into<String>) {
        let problem = ApiProblem::new("INSTRUMENT_COVERAGE_REFRESH_FAILED", message.into())
            .with_source("instrument-registry");
        let checked_at_ms = common::time::now_ms().max(1);
        self.probe_by_venue.insert(
            venue.trim().to_ascii_lowercase(),
            VenueProbeState::Failed {
                checked_at_ms,
                problem,
            },
        );
    }

    pub(crate) fn record_spot_refresh_failure(&self, venue: &str, message: impl Into<String>) {
        let problem = ApiProblem::new("SPOT_INSTRUMENT_COVERAGE_REFRESH_FAILED", message.into())
            .with_source("instrument-registry");
        self.spot_probe_by_venue.insert(
            venue.trim().to_ascii_lowercase(),
            VenueProbeState::Failed {
                checked_at_ms: common::time::now_ms().max(1),
                problem,
            },
        );
    }

    pub(crate) fn record_spot_unsupported(&self, venue: &str, message: impl Into<String>) {
        let problem = ApiProblem::new("SPOT_INSTRUMENT_COVERAGE_UNSUPPORTED", message.into())
            .with_source("instrument-registry");
        self.spot_probe_by_venue.insert(
            venue.trim().to_ascii_lowercase(),
            VenueProbeState::Unsupported {
                checked_at_ms: common::time::now_ms().max(1),
                problem,
            },
        );
    }

    pub(crate) fn record_unsupported(&self, venue: &str, message: impl Into<String>) {
        let problem = ApiProblem::new("INSTRUMENT_COVERAGE_UNSUPPORTED", message.into())
            .with_source("instrument-registry");
        let checked_at_ms = common::time::now_ms().max(1);
        self.probe_by_venue.insert(
            venue.trim().to_ascii_lowercase(),
            VenueProbeState::Unsupported {
                checked_at_ms,
                problem,
            },
        );
    }

    pub(crate) fn coverage(&self, canonical_symbol: &str, now_ms: i64) -> InstrumentCoverageEntry {
        let canonical_symbol = normalize_canonical_symbol(canonical_symbol);
        let venues = SUPPORTED_VENUES
            .iter()
            .map(|venue| self.venue_coverage(venue, &canonical_symbol, now_ms.max(1)))
            .collect();
        InstrumentCoverageEntry {
            canonical_symbol,
            venues,
        }
    }

    pub(crate) fn coverage_diagnostic(
        &self,
        canonical_symbol: &str,
        now_ms: i64,
    ) -> InstrumentCoverageDiagnostic {
        let coverage = self.coverage(canonical_symbol, now_ms);
        let executable_count = coverage
            .venues
            .iter()
            .filter(|entry| entry.is_executable_leg(now_ms))
            .count();
        let venue_count = coverage.venues.len();
        let constructible = executable_count >= 2;
        let mut diagnostics_text = format!(
            "{} 规格就绪 {executable_count}/{venue_count} · {}",
            coverage.canonical_symbol,
            if constructible {
                "可构建跨所双腿"
            } else {
                "仅观察"
            }
        );
        for entry in &coverage.venues {
            let _ = write!(
                diagnostics_text,
                "；{} {} · {} · {} · 核验 {}",
                entry.venue.to_ascii_uppercase(),
                coverage_state_label(entry.state),
                metadata_source_label(entry.source),
                if entry.execution_ready {
                    "规格就绪"
                } else {
                    "规格阻断"
                },
                entry.checked_at_ms,
            );
            if let Some(deadline) = entry.stale_after_ms {
                let _ = write!(diagnostics_text, " · 截止 {deadline}");
            }
            if let Some(problem) = &entry.problem {
                let _ = write!(diagnostics_text, " · {}", problem.message);
            }
        }
        InstrumentCoverageDiagnostic {
            canonical_symbol: coverage.canonical_symbol,
            executable_count,
            venue_count,
            constructible,
            diagnostics_text,
        }
    }

    pub(crate) fn apply_listing_gate(
        &self,
        opportunities: &mut [ArbitrageOpportunityDto],
        now_ms: i64,
    ) {
        execution_gate::apply(self, opportunities, now_ms);
        super::transfer_loop::apply(self, opportunities, now_ms);
    }

    fn venue_coverage(
        &self,
        venue: &str,
        canonical_symbol: &str,
        now_ms: i64,
    ) -> VenueCoverageEntry {
        self.venue_coverage_with_candidates(venue, canonical_symbol, now_ms, None)
    }

    fn venue_coverage_with_candidates(
        &self,
        venue: &str,
        canonical_symbol: &str,
        now_ms: i64,
        candidates: Option<&[VenueInstrument]>,
    ) -> VenueCoverageEntry {
        let probe = self.probe_state(venue);
        match probe {
            Some(VenueProbeState::Restored { checked_at_ms }) => {
                self.restored_coverage(venue, canonical_symbol, checked_at_ms, now_ms, candidates)
            }
            Some(VenueProbeState::Success { checked_at_ms }) => {
                self.success_coverage(venue, canonical_symbol, checked_at_ms, now_ms, candidates)
            }
            Some(VenueProbeState::Failed {
                checked_at_ms,
                problem,
            }) => coverage_entry(
                venue,
                canonical_symbol,
                VenueListingState::Failed,
                checked_at_ms,
                CoverageEvidence {
                    source: InstrumentMetadataSource::Unverified,
                    execution_ready: false,
                    stale_after_ms: None,
                    problem: Some(problem),
                },
            ),
            Some(VenueProbeState::Unsupported {
                checked_at_ms,
                problem,
            }) => coverage_entry(
                venue,
                canonical_symbol,
                VenueListingState::Unsupported,
                checked_at_ms,
                CoverageEvidence {
                    source: InstrumentMetadataSource::Unverified,
                    execution_ready: false,
                    stale_after_ms: None,
                    problem: Some(problem),
                },
            ),
            None => coverage_entry(
                venue,
                canonical_symbol,
                VenueListingState::Unknown,
                now_ms,
                CoverageEvidence {
                    source: InstrumentMetadataSource::Unverified,
                    execution_ready: false,
                    stale_after_ms: None,
                    problem: None,
                },
            ),
        }
    }

    fn success_coverage(
        &self,
        venue: &str,
        canonical_symbol: &str,
        checked_at_ms: i64,
        now_ms: i64,
        candidates: Option<&[VenueInstrument]>,
    ) -> VenueCoverageEntry {
        let instrument = candidates.map_or_else(
            || {
                preferred_coverage_instrument(
                    self.by_key
                        .iter()
                        .filter_map(|entry| {
                            let value = entry.value();
                            (super::venue_in_scope(&value.venue, venue)
                                && normalize_canonical_symbol(&value.canonical_symbol)
                                    == canonical_symbol)
                                .then(|| value.clone())
                        })
                        .collect(),
                    now_ms,
                )
            },
            |values| preferred_coverage_instrument(values.to_vec(), now_ms),
        );
        let evidence_checked_at_ms = instrument.as_ref().map_or(checked_at_ms, |value| {
            value.checked_at_ms.min(checked_at_ms)
        });
        let stale_after_ms = evidence_checked_at_ms.saturating_add(INSTRUMENT_SPEC_FRESHNESS_MS);
        let evidence_fresh = now_ms >= evidence_checked_at_ms && now_ms < stale_after_ms;
        let execution_ready = evidence_fresh
            && instrument.as_ref().is_some_and(|value| {
                value.is_hedge_constructible_at(now_ms, INSTRUMENT_SPEC_FRESHNESS_MS)
            });
        let execution_problem = evidence_fresh
            .then(|| instrument.as_ref().and_then(instrument_execution_problem))
            .flatten();
        let state = if !evidence_fresh {
            VenueListingState::Stale
        } else if instrument
            .as_ref()
            .is_some_and(|value| value.listing_status == InstrumentListingStatus::Trading)
        {
            VenueListingState::Listed
        } else {
            VenueListingState::Unlisted
        };
        coverage_entry(
            venue,
            instrument
                .as_ref()
                .map_or(canonical_symbol, |value| value.native_symbol.as_str()),
            state,
            evidence_checked_at_ms,
            CoverageEvidence {
                source: instrument
                    .as_ref()
                    .map_or(InstrumentMetadataSource::OfficialEndpoint, |value| {
                        value.source
                    }),
                execution_ready,
                stale_after_ms: Some(stale_after_ms),
                problem: execution_problem,
            },
        )
    }

    fn restored_coverage(
        &self,
        venue: &str,
        canonical_symbol: &str,
        checked_at_ms: i64,
        now_ms: i64,
        candidates: Option<&[VenueInstrument]>,
    ) -> VenueCoverageEntry {
        let values = candidates.map_or_else(
            || {
                self.by_key
                    .iter()
                    .filter_map(|entry| {
                        let value = entry.value();
                        (super::venue_in_scope(&value.venue, venue)
                            && normalize_canonical_symbol(&value.canonical_symbol)
                                == canonical_symbol)
                            .then(|| value.clone())
                    })
                    .collect::<Vec<_>>()
            },
            <[VenueInstrument]>::to_vec,
        );
        let Some(instrument) = preferred_coverage_instrument(values, now_ms) else {
            return coverage_entry(
                venue,
                canonical_symbol,
                VenueListingState::Unknown,
                checked_at_ms,
                CoverageEvidence {
                    source: InstrumentMetadataSource::Unverified,
                    execution_ready: false,
                    stale_after_ms: None,
                    problem: None,
                },
            );
        };
        coverage_entry(
            venue,
            &instrument.native_symbol,
            VenueListingState::Stale,
            instrument.checked_at_ms.min(checked_at_ms),
            CoverageEvidence {
                source: instrument.source,
                execution_ready: false,
                stale_after_ms: Some(
                    instrument
                        .checked_at_ms
                        .saturating_add(INSTRUMENT_SPEC_FRESHNESS_MS),
                ),
                problem: None,
            },
        )
    }
}
