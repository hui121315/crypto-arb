//! Replayable execution-run evidence shared by the API and workstation.

use crate::execution_ledger::{ExecutionFillConfidence, ExecutionLedgerEventType};
use crate::hedge::{HedgeLegRole, OrderCompilePlan};
use crate::live_trading::{OrderUpdateSource, VenueOrderIdentity};
use crate::problem::ApiProblem;
use crate::workflow::HedgeTicketView;
use crate::ExecutionRunState;
use serde::{Deserialize, Serialize};

pub const EXECUTION_RUN_EVIDENCE_SCHEMA_VERSION: u8 = 2;
pub const EXECUTION_RUN_TIMELINE_LIMIT: usize = 96;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionRunEventKind {
    #[default]
    Preview,
    Preflight,
    Submit,
    OrderUpdate,
    Fill,
    Cancel,
    Funding,
    Unwind,
    Reconcile,
    Failure,
    Closed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionRunTimelineEvent {
    pub event_id: String,
    pub kind: ExecutionRunEventKind,
    pub state: ExecutionRunState,
    pub source: OrderUpdateSource,
    pub message: String,
    pub occurred_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub leg_role: Option<HedgeLegRole>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order_identity: Option<VenueOrderIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ledger_event_type: Option<ExecutionLedgerEventType>,
    #[serde(default)]
    pub finality_confidence: ExecutionFillConfidence,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<ApiProblem>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionRunLegEvidence {
    pub role: HedgeLegRole,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compile_plan: Option<OrderCompilePlan>,
    #[serde(default)]
    pub finality_confidence: ExecutionFillConfidence,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_finality_event_id: Option<String>,
}

impl ExecutionRunLegEvidence {
    pub fn new(role: HedgeLegRole) -> Self {
        Self {
            role,
            compile_plan: None,
            finality_confidence: ExecutionFillConfidence::Unknown,
            last_finality_event_id: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionRunEvidence {
    pub schema_version: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<ExecutionRunTimelineEvent>,
    #[serde(default)]
    pub dropped_event_count: usize,
    pub long_leg: ExecutionRunLegEvidence,
    pub short_leg: ExecutionRunLegEvidence,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hedge_ticket_view: Option<HedgeTicketView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_reconciled_at_ms: Option<i64>,
}

impl Default for ExecutionRunEvidence {
    fn default() -> Self {
        Self {
            schema_version: EXECUTION_RUN_EVIDENCE_SCHEMA_VERSION,
            request_id: None,
            events: Vec::new(),
            dropped_event_count: 0,
            long_leg: ExecutionRunLegEvidence::new(HedgeLegRole::Long),
            short_leg: ExecutionRunLegEvidence::new(HedgeLegRole::Short),
            hedge_ticket_view: None,
            last_reconciled_at_ms: None,
        }
    }
}

impl ExecutionRunEvidence {
    pub fn leg(&self, role: HedgeLegRole) -> &ExecutionRunLegEvidence {
        match role {
            HedgeLegRole::Long => &self.long_leg,
            HedgeLegRole::Short => &self.short_leg,
        }
    }

    pub fn leg_mut(&mut self, role: HedgeLegRole) -> &mut ExecutionRunLegEvidence {
        match role {
            HedgeLegRole::Long => &mut self.long_leg,
            HedgeLegRole::Short => &mut self.short_leg,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_evidence_keeps_explicit_leg_roles() {
        let evidence = ExecutionRunEvidence::default();

        assert_eq!(
            evidence.schema_version,
            EXECUTION_RUN_EVIDENCE_SCHEMA_VERSION
        );
        assert_eq!(evidence.long_leg.role, HedgeLegRole::Long);
        assert_eq!(evidence.short_leg.role, HedgeLegRole::Short);
        assert!(evidence.events.is_empty());
    }
}
