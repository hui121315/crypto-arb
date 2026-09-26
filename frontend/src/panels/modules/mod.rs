pub(crate) mod automation;
pub(crate) mod cost_copy;
#[cfg(test)]
pub(crate) mod cost_profile;
pub(crate) mod execution;
pub(crate) mod funding_stats;
pub(crate) mod futures;
pub(crate) mod gate_crossex;
pub(crate) mod index_composition;
pub(crate) mod instrument_search;
pub(crate) mod leg_label;
pub(crate) mod market_evidence;
pub(crate) mod onchain;
pub(crate) mod opportunities;
pub(crate) mod opportunity_counts;
pub(crate) mod opportunity_eligibility;
pub(crate) mod opportunity_envelope;
pub(crate) mod opportunity_format;
pub(crate) mod opportunity_runtime;
pub(crate) mod opportunity_toolbar_state;
pub(crate) mod opportunity_view_model;
pub(crate) mod pagination;
pub(crate) mod positions;
pub(crate) mod rate_format;
pub(crate) mod review;
pub(crate) mod settings;
pub(crate) mod strategy_kinds;
pub(crate) mod stocks;
pub(crate) mod strategy_scope;
pub(crate) mod timestamp;

pub(in crate::panels) use automation::{
    automation_module, create_automation_runtime, AutomationRuntime,
};
pub(in crate::panels) use execution::{
    create_execution_runtime, execution_module, ExecutionRuntime, ExecutionSelection,
};
pub(in crate::panels) use futures::{create_futures_runtime, futures_module, FuturesRuntime};
pub(in crate::panels) use gate_crossex::{
    create_gate_crossex_runtime, gate_crossex_module, GateCrossExRuntime,
};
pub(in crate::panels) use onchain::{create_onchain_runtime, onchain_module, OnchainRuntime};
pub(in crate::panels) use stocks::{create_stocks_runtime, stocks_module, StocksRuntime};
pub(in crate::panels) use opportunities::{
    create_opportunities_runtime, opportunities_module, OpportunitiesRuntime,
};
pub(in crate::panels) use positions::{
    create_positions_runtime, positions_module, PositionsRuntime,
};
pub(in crate::panels) use review::{create_review_runtime, review_module, ReviewRuntime};
pub(in crate::panels) use settings::{create_settings_runtime, select_credentials_tab, select_risk_tab, settings_module, SettingsRuntime};
