use shared_types::{normalized_venue_name, venue_family, VenueOperationHealthSnapshot};

pub(crate) fn filter_snapshot(
    snapshot: VenueOperationHealthSnapshot,
    venue: Option<&str>,
) -> VenueOperationHealthSnapshot {
    let Some(selected) = selected_venue(venue) else {
        return snapshot;
    };
    let generated_at_ms = snapshot.generated_at_ms;
    let rows = snapshot
        .rows
        .into_iter()
        .filter(|row| row_matches_selected_venue(&row.venue, &selected))
        .collect();
    VenueOperationHealthSnapshot::new(rows, generated_at_ms)
}

fn selected_venue(venue: Option<&str>) -> Option<String> {
    let selected = normalized_venue_name(venue?);
    (!selected.is_empty()).then_some(selected)
}

fn row_matches_selected_venue(row_venue: &str, selected: &str) -> bool {
    let row = normalized_venue_name(row_venue);
    if row == selected {
        return true;
    }
    if selected.contains(':') {
        return false;
    }
    normalized_venue_name(venue_family(row_venue)) == selected
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::{ApiProblem, VenueOperationHealth, VenueOperationStatus};

    #[test]
    fn empty_query_keeps_full_snapshot() {
        let snapshot = snapshot(vec![
            operation("binance", VenueOperationStatus::Ok),
            operation("hyperliquid:xyz", VenueOperationStatus::Blocked),
        ]);

        let filtered = filter_snapshot(snapshot, Some(" "));

        assert_eq!(filtered.row_count, 2);
        assert_eq!(filtered.attention_count, 1);
    }

    #[test]
    fn family_query_matches_exact_and_builder_venues() {
        let snapshot = snapshot(vec![
            operation("hyperliquid", VenueOperationStatus::Ok),
            operation("hyperliquid:xyz", VenueOperationStatus::Warn),
            operation("hyperliquid:km", VenueOperationStatus::Blocked),
            operation("okx", VenueOperationStatus::Blocked),
        ]);

        let filtered = filter_snapshot(snapshot, Some(" HyperLiquid "));

        let venues = filtered
            .rows
            .iter()
            .map(|row| row.venue.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            venues,
            vec!["hyperliquid", "hyperliquid:xyz", "hyperliquid:km"]
        );
        assert_eq!(filtered.row_count, 3);
        assert_eq!(filtered.attention_count, 2);
    }

    #[test]
    fn exact_builder_query_does_not_match_family_siblings() {
        let snapshot = snapshot(vec![
            operation("hyperliquid", VenueOperationStatus::Ok),
            operation("hyperliquid:xyz", VenueOperationStatus::Warn),
            operation("hyperliquid:km", VenueOperationStatus::Blocked),
        ]);

        let filtered = filter_snapshot(snapshot, Some("hyperliquid:xyz"));

        assert_eq!(filtered.row_count, 1);
        assert_eq!(filtered.attention_count, 1);
        assert_eq!(filtered.rows[0].venue, "hyperliquid:xyz");
    }

    #[test]
    fn filtering_recomputes_retry_after_ms_from_selected_rows() {
        let mut binance = operation("binance", VenueOperationStatus::Blocked);
        binance.retry_after_ms = Some(60_000);
        let mut okx_runtime = operation("okx", VenueOperationStatus::Warn);
        okx_runtime.retry_after_ms = Some(3_000);
        let mut okx_problem = operation("okx", VenueOperationStatus::Blocked);
        okx_problem.problem =
            Some(ApiProblem::new("RATE_LIMITED", "rate limited").with_retry_after_ms(Some(2_000)));
        let snapshot = snapshot(vec![binance, okx_runtime, okx_problem]);

        let filtered = filter_snapshot(snapshot, Some("okx"));

        assert_eq!(filtered.row_count, 2);
        assert_eq!(filtered.attention_count, 2);
        assert_eq!(filtered.retry_after_ms, Some(3_000));
    }

    fn snapshot(rows: Vec<VenueOperationHealth>) -> VenueOperationHealthSnapshot {
        VenueOperationHealthSnapshot::new(rows, 42)
    }

    fn operation(venue: &str, status: VenueOperationStatus) -> VenueOperationHealth {
        VenueOperationHealth {
            venue: venue.to_owned(),
            operation: "credential_probe:balance_read".to_owned(),
            status,
            source: "test".to_owned(),
            message: "test".to_owned(),
            supported: Some(true),
            configured: Some(true),
            requested: Some(1),
            rows: Some(1),
            freshness_ms: Some(10),
            retry_after_ms: None,
            latency_ms: None,
            latency_p95_ms: None,
            error: None,
            evidence: None,
            problem: None,
            observed_at_ms: 42,
        }
    }
}
