use std::collections::{BTreeMap, BTreeSet};

use crate::panels::routing::RunRouteContext;
use shared_types::ExecutedTrade;

// A ledger group can contain several runs. Never infer one from the symbol or row ID.
pub(super) fn related_runs(row: &ExecutedTrade) -> Vec<RunRouteContext> {
    let mut identities = BTreeMap::<(String, String), BTreeSet<String>>::new();
    let valid = |value: &str| !value.is_empty() && value.trim() == value
        && value.chars().count() <= 160 && !value.chars().any(char::is_control);
    for event in &row.evidence.ledger_events {
        if let (Some(run), Some(ticket)) = (&event.order.run_id, &event.order.ticket_id) {
            if valid(run) && valid(ticket) {
                identities.entry((run.clone(), ticket.clone())).or_default();
            }
        }
    }
    for close in &row.evidence.close_run_evidence {
        if valid(&close.run_id) && valid(&close.ticket_id) {
            identities.entry((close.run_id.clone(), close.ticket_id.clone()))
                .or_default().insert(close.opportunity_id.clone());
        }
    }
    identities.into_iter().filter_map(|((run_id, ticket_id), opportunities)| {
        // Conflicting or incomplete explicit identities must not become a broader link.
        if opportunities.len() > 1 || opportunities.iter().any(|id| !valid(id)) { return None; }
        Some(RunRouteContext {
            run_id,
            ticket_id: Some(ticket_id),
            opportunity_id: opportunities.into_iter().next(),
        })
    }).collect()
}
