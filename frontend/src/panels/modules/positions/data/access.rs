use std::collections::BTreeMap;

use shared_types::{
    normalized_venue_name, PortfolioSnapshot, PositionOrigin, PositionRow, VenueOperationHealth,
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::panels::modules::positions) struct PortfolioAccountAccess {
    pub configured_venues: Vec<String>,
    pub unconfigured_venues: Vec<String>,
}

impl PortfolioAccountAccess {
    pub(in crate::panels::modules::positions) fn coverage_incomplete(&self) -> bool {
        !self.unconfigured_venues.is_empty()
    }

    pub(in crate::panels::modules::positions) fn account_data_unavailable(&self) -> bool {
        self.configured_venues.is_empty()
    }

    pub(in crate::panels::modules::positions) fn has_configured_venue(&self) -> bool {
        !self.configured_venues.is_empty()
    }
}

pub(in crate::panels::modules::positions) fn portfolio_account_access(
    snapshot: &PortfolioSnapshot,
) -> PortfolioAccountAccess {
    account_access_from_health(&snapshot.operation_health)
}

pub(in crate::panels::modules::positions) fn has_execution_projection(
    rows: &[PositionRow],
) -> bool {
    !rows.is_empty()
        && rows
            .iter()
            .all(|row| row.origin == PositionOrigin::ExecutionLedger)
}

pub(in crate::panels::modules::positions) fn has_execution_ledger_context(
    snapshot: &PortfolioSnapshot,
) -> bool {
    has_execution_projection(&snapshot.positions) || !snapshot.recent_close_runs.is_empty()
}

fn account_access_from_health(rows: &[VenueOperationHealth]) -> PortfolioAccountAccess {
    let mut venues = BTreeMap::<String, bool>::new();
    for row in rows.iter().filter(|row| account_operation(row)) {
        let venue = normalized_venue_name(&row.venue);
        let configured = row.configured == Some(true);
        venues
            .entry(venue)
            .and_modify(|current| *current |= configured)
            .or_insert(configured);
    }

    let (configured_venues, unconfigured_venues) = venues
        .into_iter()
        .partition::<Vec<_>, _>(|(_, configured)| *configured);
    PortfolioAccountAccess {
        configured_venues: configured_venues
            .into_iter()
            .map(|(venue, _)| venue)
            .collect(),
        unconfigured_venues: unconfigured_venues
            .into_iter()
            .map(|(venue, _)| venue)
            .collect(),
    }
}

fn account_operation(row: &VenueOperationHealth) -> bool {
    row.supported != Some(false)
        && !matches!(row.venue.as_str(), "mock" | "system")
        && matches!(
            row.operation.as_str(),
            "balance" | "positions" | "private_ws_account_stream" | "private_ws_order_stream"
        )
        && row.configured.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::VenueOperationStatus;

    #[test]
    fn unconfigured_account_rows_are_deduplicated_by_venue() {
        let rows = vec![
            health("okx", "balance", false),
            health("okx", "positions", false),
            health("bybit", "private_ws_account_stream", false),
        ];

        let access = account_access_from_health(&rows);

        assert!(access.configured_venues.is_empty());
        assert_eq!(access.unconfigured_venues, vec!["bybit", "okx"]);
        assert!(access.coverage_incomplete());
        assert!(access.account_data_unavailable());
    }

    #[test]
    fn one_configured_operation_marks_the_venue_configured() {
        let rows = vec![
            health("okx", "balance", false),
            health("okx", "positions", true),
            health("system", "storage:portfolio_nav", true),
        ];

        let access = account_access_from_health(&rows);

        assert_eq!(access.configured_venues, vec!["okx"]);
        assert!(access.unconfigured_venues.is_empty());
        assert!(!access.coverage_incomplete());
        assert!(!access.account_data_unavailable());
    }

    #[test]
    fn partial_coverage_keeps_configured_account_data_available() {
        let rows = vec![
            health("bitget", "balance", true),
            health("bitget", "positions", true),
            health("okx", "balance", false),
        ];

        let access = account_access_from_health(&rows);

        assert_eq!(access.configured_venues, vec!["bitget"]);
        assert_eq!(access.unconfigured_venues, vec!["okx"]);
        assert!(access.coverage_incomplete());
        assert!(!access.account_data_unavailable());
    }

    fn health(venue: &str, operation: &str, configured: bool) -> VenueOperationHealth {
        VenueOperationHealth {
            venue: venue.to_owned(),
            operation: operation.to_owned(),
            status: VenueOperationStatus::Blocked,
            source: "account_cache".to_owned(),
            message: String::new(),
            supported: Some(true),
            configured: Some(configured),
            requested: None,
            rows: None,
            freshness_ms: None,
            retry_after_ms: None,
            latency_ms: None,
            latency_p95_ms: None,
            error: None,
            evidence: None,
            problem: None,
            observed_at_ms: 1,
        }
    }
}
