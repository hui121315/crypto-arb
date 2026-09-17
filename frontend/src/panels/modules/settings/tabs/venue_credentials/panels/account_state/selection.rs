use super::*;
use shared_types::{
    AccountBindingEvidence, AccountDataHealth, AccountFieldQuality, AccountFieldSubject,
    VenueAccountSummary,
};

#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct AccountEvidenceSelection {
    pub(super) field_quality: Vec<AccountFieldQuality>,
    pub(super) row_health: Vec<AccountDataHealth>,
    pub(super) bindings: Vec<AccountBindingEvidence>,
    pub(super) problems: Vec<ApiProblem>,
    pub(super) summaries: Vec<VenueAccountSummary>,
}

pub(super) fn selected_account_evidence(
    snapshot: &AccountStateSnapshot,
    venue_id: &str,
) -> AccountEvidenceSelection {
    let bindings = if snapshot.account_bindings.is_empty() {
        snapshot
            .balances
            .account_bindings
            .iter()
            .chain(snapshot.positions.account_bindings.iter())
            .chain(snapshot.open_orders.account_bindings.iter())
            .filter(|row| selected_venue_matches(&row.venue, venue_id))
            .cloned()
            .collect()
    } else {
        snapshot
            .account_bindings
            .iter()
            .filter(|row| selected_venue_matches(&row.venue, venue_id))
            .cloned()
            .collect()
    };
    AccountEvidenceSelection {
        field_quality: snapshot
            .field_quality
            .iter()
            .chain(snapshot.balances.field_quality.iter())
            .chain(snapshot.positions.field_quality.iter())
            .chain(snapshot.open_orders.field_quality.iter())
            .filter(|row| subject_matches_venue(&row.subject, venue_id))
            .cloned()
            .collect(),
        row_health: snapshot
            .balances
            .row_health
            .iter()
            .chain(snapshot.positions.row_health.iter())
            .chain(snapshot.open_orders.row_health.iter())
            .filter(|row| subject_matches_venue(&row.subject, venue_id))
            .cloned()
            .collect(),
        bindings,
        problems: snapshot
            .problems
            .iter()
            .filter(|problem| problem_matches_venue(problem, venue_id))
            .cloned()
            .collect(),
        summaries: snapshot
            .balances
            .account_summaries
            .iter()
            .filter(|row| selected_venue_matches(&row.venue, venue_id))
            .cloned()
            .collect(),
    }
}

fn subject_matches_venue(subject: &AccountFieldSubject, venue_id: &str) -> bool {
    subject
        .venue
        .as_deref()
        .is_some_and(|venue| selected_venue_matches(venue, venue_id))
}

fn selected_venue_matches(row_venue: &str, selected_venue: &str) -> bool {
    let selected = normalized_venue_name(selected_venue);
    let row = normalized_venue_name(row_venue);
    if selected.is_empty() || row.is_empty() {
        return false;
    }
    row == selected
        || (!selected.contains(':') && normalized_venue_name(venue_family(row_venue)) == selected)
}

fn problem_matches_venue(problem: &ApiProblem, venue_id: &str) -> bool {
    problem
        .details
        .as_ref()
        .and_then(|details| details.get("venue"))
        .and_then(|venue| venue.as_str())
        .is_some_and(|venue| selected_venue_matches(venue, venue_id))
}
