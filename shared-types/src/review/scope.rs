use super::ExecutedTrade;
use crate::ExecutionRun;
use serde::{Deserialize, Serialize};

/// Exact record context. This selects whole ledger groups, never a symbol or PnL subset.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewScope {
    pub run_id: Option<String>,
    pub ticket_id: Option<String>,
    pub opportunity_id: Option<String>,
    pub close_run_id: Option<String>,
}

impl ReviewScope {
    pub fn has_filter(&self) -> bool {
        self.fields().iter().any(|(_, value)| value.is_some())
    }

    pub fn is_valid(&self) -> bool {
        (self.run_id.is_some() || self.close_run_id.is_some())
            && self.fields().iter().all(|(_, value)| {
                value.is_none_or(|value| !value.trim().is_empty() && value.chars().count() <= 160)
            })
    }

    pub fn fields(&self) -> [(&'static str, Option<&str>); 4] {
        [
            ("runId", self.run_id.as_deref()),
            ("ticketId", self.ticket_id.as_deref()),
            ("opportunityId", self.opportunity_id.as_deref()),
            ("closeRunId", self.close_run_id.as_deref()),
        ]
    }

    pub fn matches(&self, row: &ExecutedTrade, known_run: Option<&ExecutionRun>) -> bool {
        if !self.is_valid() {
            return false;
        }
        let close_matches = row.evidence.close_run_evidence.iter().any(|close| {
            optional_matches(&self.close_run_id, &close.close_run_id)
                && optional_matches(&self.run_id, &close.run_id)
                && optional_matches(&self.ticket_id, &close.ticket_id)
                && optional_matches(&self.opportunity_id, &close.opportunity_id)
        });
        if self.close_run_id.is_some() || close_matches {
            return close_matches;
        }
        row.evidence.ledger_events.iter().any(|event| {
            let Some(run_id) = event.order.run_id.as_deref() else {
                return false;
            };
            let Some(ticket_id) = event.order.ticket_id.as_deref() else {
                return false;
            };
            optional_matches(&self.run_id, run_id)
                && optional_matches(&self.ticket_id, ticket_id)
                && self.opportunity_id.as_ref().is_none_or(|opportunity| {
                    known_run.is_some_and(|run| {
                        run.run_id == run_id
                            && run.ticket_id == ticket_id
                            && run.opportunity_id == *opportunity
                    })
                })
        })
    }
}

fn optional_matches(expected: &Option<String>, actual: &str) -> bool {
    expected
        .as_deref()
        .is_none_or(|expected| expected == actual)
}
