use super::super::{
    venue_in_scope, InstrumentRegistry, TransferProbeState, TRANSFER_SUPPORTED_VENUES,
};
use super::canonical_currency;
use exchange::{CurrencyTransferNetwork, TRANSFER_NETWORK_FRESHNESS_MS};
use shared_types::ApiProblem;
use std::collections::HashMap;
use std::sync::Arc;

const REFRESH_IN_FLIGHT_TIMEOUT_MS: i64 = 60_000;
const REFRESH_FAILURE_RETRY_MS: i64 = 5 * 60_000;

#[derive(Debug, Default)]
pub(in crate::services::instrument_registry) struct TransferNetworkSnapshot {
    by_family_currency: HashMap<(String, String), Vec<CurrencyTransferNetwork>>,
    pub(super) len: usize,
}

impl TransferNetworkSnapshot {
    pub(super) fn rows(&self, venue: &str, currency: &str) -> &[CurrencyTransferNetwork] {
        self.by_family_currency
            .get(&(
                shared_types::venue_family(venue).to_ascii_lowercase(),
                canonical_currency(currency),
            ))
            .map(Vec::as_slice)
            .unwrap_or_default()
    }
}

impl InstrumentRegistry {
    pub(crate) fn begin_transfer_refresh(&self, venue: &str, now_ms: i64) -> bool {
        let venue = shared_types::venue_family(venue).to_ascii_lowercase();
        if !TRANSFER_SUPPORTED_VENUES.contains(&venue.as_str()) {
            return false;
        }
        let should_refresh = match self.transfer_probe_state(&venue) {
            Some(TransferProbeState::Success { checked_at_ms }) => {
                now_ms < checked_at_ms
                    || now_ms.saturating_sub(checked_at_ms) >= TRANSFER_NETWORK_FRESHNESS_MS
            }
            Some(TransferProbeState::Refreshing { checked_at_ms }) => {
                now_ms < checked_at_ms
                    || now_ms.saturating_sub(checked_at_ms) >= REFRESH_IN_FLIGHT_TIMEOUT_MS
            }
            Some(
                TransferProbeState::Failed { checked_at_ms, .. }
                | TransferProbeState::Unsupported { checked_at_ms, .. }
                | TransferProbeState::Unavailable { checked_at_ms, .. },
            ) => {
                now_ms < checked_at_ms
                    || now_ms.saturating_sub(checked_at_ms) >= REFRESH_FAILURE_RETRY_MS
            }
            None => true,
        };
        if should_refresh {
            self.transfer_probe_by_venue.insert(
                venue,
                TransferProbeState::Refreshing {
                    checked_at_ms: now_ms.max(1),
                },
            );
        }
        should_refresh
    }

    pub(crate) fn begin_transfer_refresh_for(
        &self,
        venue: &str,
        currencies: &[String],
        now_ms: i64,
    ) -> bool {
        let venue = shared_types::venue_family(venue).to_ascii_lowercase();
        if !TRANSFER_SUPPORTED_VENUES.contains(&venue.as_str()) {
            return false;
        }
        let requested = currencies
            .iter()
            .map(|currency| canonical_currency(currency))
            .filter(|currency| !currency.is_empty())
            .collect::<std::collections::BTreeSet<_>>();
        if requested.is_empty() {
            return self.begin_transfer_refresh(&venue, now_ms);
        }
        let all_fresh = {
            let snapshot = self.transfer_snapshot.load();
            requested.iter().all(|currency| {
                let rows = snapshot.rows(&venue, currency);
                !rows.is_empty() && rows.iter().any(|row| row.is_fresh_at(now_ms))
            })
        };
        let should_refresh = match self.transfer_probe_state(&venue) {
            Some(TransferProbeState::Refreshing { checked_at_ms }) => {
                now_ms < checked_at_ms
                    || now_ms.saturating_sub(checked_at_ms) >= REFRESH_IN_FLIGHT_TIMEOUT_MS
            }
            Some(
                TransferProbeState::Failed { checked_at_ms, .. }
                | TransferProbeState::Unsupported { checked_at_ms, .. },
            ) => {
                now_ms < checked_at_ms
                    || now_ms.saturating_sub(checked_at_ms) >= REFRESH_FAILURE_RETRY_MS
            }
            Some(TransferProbeState::Unavailable {
                checked_at_ms,
                currencies,
                ..
            }) => {
                now_ms < checked_at_ms
                    || now_ms.saturating_sub(checked_at_ms) >= REFRESH_FAILURE_RETRY_MS
                    || !requested.is_subset(&currencies)
            }
            Some(TransferProbeState::Success { .. }) => !all_fresh,
            None => true,
        };
        if should_refresh {
            self.transfer_probe_by_venue.insert(
                venue,
                TransferProbeState::Refreshing {
                    checked_at_ms: now_ms.max(1),
                },
            );
        }
        should_refresh
    }

    pub(crate) fn invalidate_transfer_probe(&self, venue: &str) {
        let exact = venue.trim().to_ascii_lowercase();
        let family = shared_types::venue_family(&exact).to_ascii_lowercase();
        self.transfer_probe_by_venue.remove(&exact);
        self.transfer_probe_by_venue.remove(&family);
    }

    #[cfg(test)]
    pub(crate) fn replace_transfer_venue(
        &self,
        venue: &str,
        rows: Vec<CurrencyTransferNetwork>,
    ) -> usize {
        self.replace_transfer_venue_scope(venue, &[], rows)
    }

    pub(crate) fn replace_transfer_venue_scope(
        &self,
        venue: &str,
        currencies: &[String],
        rows: Vec<CurrencyTransferNetwork>,
    ) -> usize {
        let venue_key = venue.trim().to_ascii_lowercase();
        let family_key = shared_types::venue_family(&venue_key).to_ascii_lowercase();
        let requested = currencies
            .iter()
            .map(|currency| canonical_currency(currency))
            .filter(|currency| !currency.is_empty())
            .collect::<std::collections::BTreeSet<_>>();
        let accepted = rows
            .into_iter()
            .filter(|row| {
                venue_in_scope(&row.venue, &venue_key)
                    && row.is_structurally_valid()
                    && (requested.is_empty()
                        || requested.contains(&canonical_currency(&row.currency)))
            })
            .collect::<Vec<_>>();
        if accepted.is_empty() {
            return 0;
        }
        let checked_at_ms = accepted
            .iter()
            .map(|row| row.checked_at_ms)
            .max()
            .unwrap_or_else(common::time::now_ms);
        let current = self.transfer_snapshot.load();
        let mut by_family_currency = current.by_family_currency.clone();
        by_family_currency.retain(|(candidate, currency), _| {
            candidate != &family_key || (!requested.is_empty() && !requested.contains(currency))
        });
        let count = accepted.len();
        for row in accepted {
            let key = (
                shared_types::venue_family(&row.venue).to_ascii_lowercase(),
                canonical_currency(&row.currency),
            );
            by_family_currency.entry(key).or_default().push(row);
        }
        let len = by_family_currency.values().map(Vec::len).sum();
        self.transfer_snapshot
            .store(Arc::new(TransferNetworkSnapshot {
                by_family_currency,
                len,
            }));
        self.record_transfer_success(&venue_key, checked_at_ms);
        count
    }

    pub(crate) fn transfer_len(&self) -> usize {
        self.transfer_snapshot.load().len
    }

    pub(crate) fn record_transfer_failure(&self, venue: &str, message: impl Into<String>) {
        self.transfer_probe_by_venue.insert(
            venue.trim().to_ascii_lowercase(),
            TransferProbeState::Failed {
                checked_at_ms: common::time::now_ms().max(1),
                problem: ApiProblem::new("TRANSFER_NETWORK_REFRESH_FAILED", message.into())
                    .with_source("instrument-registry.transfer"),
            },
        );
    }

    pub(crate) fn record_transfer_unavailable(
        &self,
        venue: &str,
        currencies: &[String],
        message: impl Into<String>,
    ) {
        self.transfer_probe_by_venue.insert(
            venue.trim().to_ascii_lowercase(),
            TransferProbeState::Unavailable {
                checked_at_ms: common::time::now_ms().max(1),
                currencies: currencies
                    .iter()
                    .map(|currency| canonical_currency(currency))
                    .filter(|currency| !currency.is_empty())
                    .collect(),
                problem: ApiProblem::new("TRANSFER_NETWORK_UNAVAILABLE", message.into())
                    .with_source("instrument-registry.transfer"),
            },
        );
    }

    pub(crate) fn record_transfer_unsupported(&self, venue: &str, message: impl Into<String>) {
        self.transfer_probe_by_venue.insert(
            venue.trim().to_ascii_lowercase(),
            TransferProbeState::Unsupported {
                checked_at_ms: common::time::now_ms().max(1),
                problem: ApiProblem::new("TRANSFER_NETWORK_UNSUPPORTED", message.into())
                    .with_source("instrument-registry.transfer"),
            },
        );
    }

    fn record_transfer_success(&self, venue: &str, checked_at_ms: i64) {
        self.transfer_probe_by_venue.insert(
            venue.trim().to_ascii_lowercase(),
            TransferProbeState::Success {
                checked_at_ms: checked_at_ms.max(1),
            },
        );
    }

    pub(super) fn transfer_probe_state(&self, venue: &str) -> Option<TransferProbeState> {
        let exact = venue.trim().to_ascii_lowercase();
        self.transfer_probe_by_venue
            .get(&exact)
            .map(|state| state.clone())
            .or_else(|| {
                let family = shared_types::venue_family(&exact);
                (family != exact && TRANSFER_SUPPORTED_VENUES.contains(&family))
                    .then(|| {
                        self.transfer_probe_by_venue
                            .get(family)
                            .map(|state| state.clone())
                    })
                    .flatten()
            })
    }
}
