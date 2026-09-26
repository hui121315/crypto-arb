use crate::panels::modules::opportunity_counts::OpportunityCountMeta;
use crate::state::arbitrage_stream::stream_problem_label;
use crate::state::load_state::LoadState;
use leptos::prelude::*;
use shared_types::{
    is_hyperliquid_builder_venue, ApiProblem, ExecutionEnvironment, NextFundingSlot as FundingSlot,
    OpportunityEnvelopeStatus, OpportunityStreamEvent, RiskStatusSlot, RuntimeProblem,
    TradingStatusResponse, VenueOperationEvidence, VenueOperationHealth,
    VenueOperationHealthSnapshot, VenueOperationKind, VenueOperationStatus,
    UNRECORDED_EVIDENCE_MARKER,
};
use std::collections::BTreeMap;

const BACKGROUND_TASK_SCOPE: &str = "background_task";
const ARBITRAGE_SNAPSHOT_TASK: &str = "arbitrage_snapshot";
const FRESH_SCAN_STALE_AFTER_SECS: u64 = 60;

fn dot_class(red: bool) -> &'static str {
    if red {
        "slot-dot red"
    } else {
        "slot-dot ok"
    }
}

fn scalar_slot_class(degraded: bool) -> &'static str {
    if degraded {
        "slot degraded"
    } else {
        "slot"
    }
}

fn credential_configuration_summary(snapshot: Option<&VenueOperationHealthSnapshot>) -> String {
    let Some(snapshot) = snapshot else {
        return String::new();
    };
    let requirements = snapshot
        .rows
        .iter()
        .filter(|row| {
            row.supported != Some(false)
                && row.configured == Some(false)
                && VenueOperationKind::parse(&row.operation) == VenueOperationKind::PrivateRead
        })
        .map(|row| (row.venue.as_str(), row.message.as_str()))
        .collect::<BTreeMap<_, _>>();
    if requirements.is_empty() {
        return String::new();
    }
    format!(
        "需配置：{}",
        requirements
            .values()
            .copied()
            .collect::<Vec<_>>()
            .join("；")
    )
}

fn configuration_summary_for_environment(
    snapshot: Option<&VenueOperationHealthSnapshot>,
    environment: Option<ExecutionEnvironment>,
) -> String {
    let summary = credential_configuration_summary(snapshot);
    if summary.is_empty() {
        return summary;
    }
    if environment == Some(ExecutionEnvironment::Paper) {
        format!("模拟模式无需私有凭证；启用实盘时{summary}")
    } else {
        summary
    }
}

mod api;
mod app_ws;
mod execution_mode;
mod funding;
mod market_data;
mod net_delta;
mod operation;
mod order_elapsed;
mod risk;
mod readiness;
mod scan;
#[cfg(test)]
mod tests;
mod ws;

pub(super) use api::*;
pub use app_ws::*;
pub use execution_mode::*;
pub use funding::*;
pub use market_data::*;
pub use net_delta::*;
use operation::*;
pub use order_elapsed::*;
pub use risk::*;
pub(super) use readiness::*;
pub(crate) use scan::*;
pub use ws::*;
