//! Execution facts shared by API orchestration, ledger replay and the workstation.

pub use crate::{
    ApiProblem, ExecutionEnvironment, ExecutionFillConfidence, ExecutionLedgerEvent,
    ExecutionLedgerEventType, ExecutionLedgerOrderRef, ExecutionLedgerPayload,
    ExecutionLedgerQuality, ExecutionRun, ExecutionRunEventKind, ExecutionRunEvidence,
    ExecutionRunLeg, ExecutionRunLegEvidence, ExecutionRunState, ExecutionRunTimelineEvent,
    FeeLedgerSnapshot, FeeScheduleEvidence, FillLedgerSnapshot, HedgeTicket, KillSwitchRequest,
    KillSwitchResponse, LiveOrderState, OrderRecord, OrderUpdateSource, ProfitabilityEvidence,
    SubmitOrderRequest, TradeFeeSnapshot, TradingAdapterOption, TradingAdaptersResponse,
    TradingRiskStatus, TradingStatusResponse, TradingVenueCapability, TradingWsChannels,
    VenueOrderIdentity,
};

pub type CostEvidenceMeta = ProfitabilityEvidence;
pub type FeeEvidenceMeta = FeeScheduleEvidence;
pub type OrderFill = FillLedgerSnapshot;
pub type LegFinality = ExecutionRunLegEvidence;
