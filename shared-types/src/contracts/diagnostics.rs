//! Diagnostic-only contracts kept outside the P0 product facade.

pub use crate::{
    InstrumentCoverageDiagnostic, MarketDataDiagnosticsSnapshot, RestBaselineDiagnostics,
    RuntimeProblem, StorageRuntimeContract, TaskHealthIssue, TaskHealthSummary,
};

pub mod legacy {
    pub use crate::{
        ArbitrageOpportunityDto, ExecutionMode, LlmExternalPayload, OnchainMetadata,
        OptionMarketQuote, SimulationPortfolioSummary, SimulationPosition, SimulationRuntimeMeta,
    };
}
