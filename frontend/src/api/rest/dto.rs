pub use shared_types::contracts::execution::{
    KillSwitchRequest, KillSwitchResponse, SubmitOrderRequest, TradingAdapterOption,
    TradingAdaptersResponse, TradingRiskStatus, TradingStatusResponse, TradingVenueCapability,
    TradingWsChannels,
};

pub type OpportunityListResponse = shared_types::contracts::p0::OpportunityListEnvelope;
pub type OpportunityDetailResponse = shared_types::OpportunityDetailEnvelope;
pub type OpportunityStreamPayload = shared_types::contracts::p0::OpportunityStreamPayload;
pub type HistoryResponse<T> = shared_types::HistoryResponse<T>;
pub type FundingRatesResponse = shared_types::FundingRatesEnvelope;
pub type OpportunityHistoryRow = shared_types::OpportunityHistoryRow;
pub type OrderStreamPayload = shared_types::OrderStreamPayload;
pub type RiskAlertEvent = shared_types::RiskAlertEvent;
pub type ExecutionRunEvent = shared_types::ExecutionRunEvent;
