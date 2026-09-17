use crate::services::{
    action_runs,
    close_run_costs::refresh_cost_reconciliation,
    market_data::{MarketQuality, MarketRead},
};
use crate::state::AppState;
use axum::http::StatusCode;
use common::AppError;
use serde_json::json;
use shared_types::{
    normalized_venue_name, problem::codes, venue_names_equal, ActionRun, ActionRunKind,
    ActionRunStatus, ApiProblem, CloseLeg, CloseLegStatus, CloseRun, CloseRunCompensationAttempt,
    CloseRunCompensationRequest, CloseRunCostComponent, CloseRunCostLedgerEvent,
    CloseRunManualTerminalEvidence, CloseRunManualTerminalRequest, CloseRunNextAction,
    CloseRunNextActionKind, CloseRunStatus, CloseRunUnwindLegEvidence, CloseRunUnwindPlan,
    CloseRunUnwindPlanStatus, ExecutionLedgerEvent, ExecutionLedgerEventType,
    ExecutionLedgerPayload, ExecutionLedgerQuality, ExecutionMode, FillLedgerSnapshot,
    FundingPaymentLedgerRecord, HedgeLegRole, LiveOrderState, MarginMode, OrderBookInfo,
    OrderRecord, OrderSide, OrderSource, OrderType, OrderUpdateSource, PositionPairEvidence,
    PositionSide, SlippageLedgerRecord, TimeInForce, VenueOperationHealth, VenueOperationStatus,
    VenueOrderIdentity, CLOSE_RUN_COMPENSATION_CONFIRMATION_PHRASE,
    CLOSE_RUN_MANUAL_TERMINAL_CONFIRMATION_PHRASE,
};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

mod action;
mod apply;
mod auto_compensation;
mod ledger;
mod matchers;
mod prepare;
mod project;
mod record_order;
mod replay;
mod runtime;
mod summary;
#[cfg(test)]
mod tests;
mod unwind;
mod unwind_notional;

pub(crate) use action::action_status;
use action::*;
use apply::*;
pub(crate) use auto_compensation::{auto_submit_compensation_once, AutoCompensationOutcome};
use ledger::*;
use matchers::*;
use prepare::*;
use project::append_close_run_finality_from_order;
pub(crate) use project::{
    project_finality_problem, project_ledger_event_update, project_ledger_event_update_durable,
    project_order_update, record, record_manual_terminal_evidence, submit_compensation_order,
};
use record_order::*;
pub(crate) use replay::normalize_replayed_paper_finality;
use runtime::*;
use summary::*;
use unwind::*;
use unwind_notional::*;
