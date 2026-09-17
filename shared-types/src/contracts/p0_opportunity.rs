//! Main-P0 opportunity contracts and the fail-closed product projection.

use crate::problem::codes;
use crate::{
    is_p0_executable_strategy, opportunity_build_blockers_allow_preflight, ApiProblem,
    MarketDataQuality, SpotLegMode, StrategyCategory, StrategyKind,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub use crate::{
    OpportunityCountBreakdown, OpportunityEnvelopeScope, OpportunityEnvelopeStatus,
    OpportunityListCost, OpportunityListEnvelope, OpportunityListExecution, OpportunityListLeg,
    OpportunityListLegFunding, OpportunityListMetrics, OpportunityListPage, OpportunityListRow,
    OpportunityListSortKey, OpportunityQueryScopeMeta, OpportunityScanMeta, OpportunityScanOutcome,
    OpportunityStreamEvent, OpportunityStreamEventKind, OpportunityStreamPayload,
    OpportunityStreamWindow, P0_EXECUTABLE_STRATEGY_KINDS,
};

/// Rows carried by the live first-page projection. Larger result sets remain
/// available through cursor pagination instead of being repeated on every WS frame.
pub const OPPORTUNITY_PRODUCT_PAGE_SIZE: usize = 50;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpportunityCore {
    pub id: String,
    pub symbol: String,
    pub strategy_kind: StrategyKind,
    pub strategy_category: StrategyCategory,
    pub type_label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spot_leg_mode: Option<SpotLegMode>,
}

pub type OpportunityMetrics = OpportunityListMetrics;
pub type OpportunityExecution = OpportunityListExecution;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpportunityEvidence {
    pub long_leg: OpportunityListLeg,
    pub short_leg: OpportunityListLeg,
    pub cost: OpportunityListCost,
    pub source: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct P0OpportunityContract {
    pub core: OpportunityCore,
    pub metrics: OpportunityMetrics,
    pub execution: OpportunityExecution,
    pub evidence: OpportunityEvidence,
}

impl TryFrom<OpportunityListRow> for P0OpportunityContract {
    type Error = Box<ApiProblem>;

    fn try_from(row: OpportunityListRow) -> Result<Self, Self::Error> {
        let strategy_kind = require_p0_strategy(row.strategy_kind)?;
        let strategy_category = require_strategy_category(row.strategy_category, strategy_kind)?;
        validate_execution_evidence(&row)?;
        Ok(Self {
            core: OpportunityCore {
                id: row.id,
                symbol: row.symbol,
                strategy_kind,
                strategy_category,
                type_label: row.type_label,
                spot_leg_mode: row.spot_leg_mode,
            },
            metrics: row.metrics,
            execution: row.execution,
            evidence: OpportunityEvidence {
                long_leg: row.long_leg,
                short_leg: row.short_leg,
                cost: row.cost,
                source: row.data_source,
                updated_at: row.updated_at,
            },
        })
    }
}

impl TryFrom<&OpportunityListRow> for P0OpportunityContract {
    type Error = Box<ApiProblem>;

    fn try_from(row: &OpportunityListRow) -> Result<Self, Self::Error> {
        Self::try_from(row.clone())
    }
}

impl From<P0OpportunityContract> for OpportunityListRow {
    fn from(contract: P0OpportunityContract) -> Self {
        Self {
            id: contract.core.id,
            symbol: contract.core.symbol,
            strategy_kind: Some(contract.core.strategy_kind),
            strategy_category: Some(contract.core.strategy_category),
            type_label: contract.core.type_label,
            spot_leg_mode: contract.core.spot_leg_mode,
            long_leg: contract.evidence.long_leg,
            short_leg: contract.evidence.short_leg,
            metrics: contract.metrics,
            cost: contract.evidence.cost,
            execution: contract.execution,
            data_source: contract.evidence.source,
            updated_at: contract.evidence.updated_at,
        }
    }
}

fn require_p0_strategy(
    strategy_kind: Option<StrategyKind>,
) -> Result<StrategyKind, Box<ApiProblem>> {
    strategy_kind
        .filter(|kind| is_p0_executable_strategy(*kind))
        .ok_or_else(|| contract_problem("P0 opportunity requires an allowlisted strategy kind"))
}

fn require_strategy_category(
    strategy_category: Option<StrategyCategory>,
    strategy_kind: StrategyKind,
) -> Result<StrategyCategory, Box<ApiProblem>> {
    strategy_category
        .filter(|category| *category == strategy_kind.category())
        .ok_or_else(|| {
            contract_problem("P0 opportunity strategy category is missing or mismatched")
        })
}

fn validate_execution_evidence(row: &OpportunityListRow) -> Result<(), Box<ApiProblem>> {
    if !row.execution.eligible {
        return Ok(());
    }
    let evidence_is_complete =
        opportunity_build_blockers_allow_preflight(row.strategy_kind, &row.execution.blockers)
            && row.cost.verified
            && row.cost.fee_evidence_complete
            && leg_has_fresh_evidence(&row.long_leg)
            && leg_has_fresh_evidence(&row.short_leg);
    if evidence_is_complete {
        Ok(())
    } else {
        Err(contract_problem(
            "build-eligible P0 opportunity requires allowed blockers, fresh legs and verified cost evidence",
        ))
    }
}

fn leg_has_fresh_evidence(leg: &OpportunityListLeg) -> bool {
    leg.market_evidence
        .as_ref()
        .is_some_and(|evidence| evidence.health.quality == MarketDataQuality::Fresh)
}

fn contract_problem(message: &'static str) -> Box<ApiProblem> {
    Box::new(
        ApiProblem::new(codes::OPPORTUNITY_NOT_EXECUTABLE, message)
            .with_source("shared-types::contracts::p0_opportunity"),
    )
}
