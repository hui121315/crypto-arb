//! 复盘 DTO。

use crate::funding::FundingPaymentIngestReport;
use crate::list::{ListPage, ListStatus};
use crate::live_trading::OrderRecord;
use crate::problem::ApiProblem;
use crate::strategy::{StrategyKind, StrategyPerformance};
use crate::venues::VenueOperationHealth;
use serde::{Deserialize, Serialize};

mod evidence;
mod scope;
pub mod settlements;
pub use scope::ReviewScope;

pub use evidence::{
    ReviewCloseRunEvidence, ReviewLedgerEventEvidence, ReviewLedgerEventTiming,
    ReviewLedgerOrderEvidence, ReviewLedgerPayloadEvidence, ReviewPnlEvidence,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewPnlField {
    Gross,
    Fee,
    Funding,
    Slippage,
    Net,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewDataSource {
    ExecutionLedger,
    MissedOpportunityStore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewLedgerStatus {
    LedgerBacked,
    PartialEvidence,
    NoCompleteRows,
    NoLedgerEvents,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewEnvelope<T> {
    pub rows: Vec<T>,
    pub generated_at_ms: i64,
    pub days: u32,
    pub source: ReviewDataSource,
    pub row_count: usize,
    pub page: ListPage,
    pub status: ListStatus,
    #[serde(default)]
    pub ledger_status: Option<ReviewLedgerStatus>,
    #[serde(default)]
    pub missing_fields: Vec<ReviewPnlField>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_health: Option<VenueOperationHealth>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub funding_payment_ingest: Option<FundingPaymentIngestReport>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub problems: Vec<ApiProblem>,
}

impl<T> ReviewEnvelope<T> {
    pub fn new(
        rows: Vec<T>,
        generated_at_ms: i64,
        days: u32,
        source: ReviewDataSource,
        ledger_status: Option<ReviewLedgerStatus>,
        missing_fields: Vec<ReviewPnlField>,
    ) -> Self {
        let row_count = rows.len();
        let page = ListPage {
            limit: row_count,
            max_limit: row_count,
            start_offset: 0,
            returned_count: row_count,
            total_rows: row_count,
            has_more: false,
            previous_cursor: None,
            next_cursor: None,
            last_cursor: None,
            snapshot_id: None,
        };
        Self {
            rows,
            generated_at_ms,
            days,
            source,
            row_count,
            page,
            status: ListStatus::Fresh,
            ledger_status,
            missing_fields,
            request_id: None,
            storage_health: None,
            funding_payment_ingest: None,
            problems: Vec::new(),
        }
    }

    pub fn with_page(
        mut self,
        page: ListPage,
        status: ListStatus,
        problems: Vec<ApiProblem>,
    ) -> Self {
        self.row_count = page.total_rows;
        self.page = page;
        self.status = status;
        self.problems = problems;
        self
    }

    pub fn with_storage_health(mut self, storage_health: VenueOperationHealth) -> Self {
        self.storage_health = Some(storage_health);
        self
    }

    pub fn with_request_id(mut self, request_id: Option<String>) -> Self {
        if let Some(request_id) = request_id.as_ref() {
            for problem in &mut self.problems {
                if problem.request_id.is_none() {
                    problem.request_id = Some(request_id.clone());
                }
            }
        }
        self.request_id = request_id;
        self
    }

    pub fn with_funding_payment_ingest(mut self, report: FundingPaymentIngestReport) -> Self {
        self.funding_payment_ingest = Some(report);
        self
    }
}

/// One lifecycle-owned review projection shared by REST and `AppWS` consumers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewRuntimeSnapshot {
    pub executed: ReviewEnvelope<ExecutedTrade>,
    pub strategy_performance: ReviewEnvelope<StrategyPerformance>,
    pub generated_at_ms: i64,
}

impl ReviewRuntimeSnapshot {
    pub fn with_request_id(mut self, request_id: Option<String>) -> Self {
        self.executed = self.executed.with_request_id(request_id.clone());
        self.strategy_performance = self.strategy_performance.with_request_id(request_id);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutedTrade {
    pub id: String,
    pub strategy: StrategyKind,
    pub symbol: String,
    pub long_venue: String,
    pub short_venue: String,
    pub opened_at_ms: i64,
    pub closed_at_ms: Option<i64>,
    pub holding_minutes: Option<u32>,
    pub gross_pnl_usd: f64,
    pub fee_usd: f64,
    pub funding_usd: f64,
    pub slippage_usd: f64,
    pub net_pnl_usd: f64,
    #[serde(default)]
    pub evidence: ReviewPnlEvidence,
    #[serde(default)]
    pub actual_fields: Vec<ReviewPnlField>,
    #[serde(default)]
    pub estimated_fields: Vec<ReviewPnlField>,
    #[serde(default)]
    pub missing_fields: Vec<ReviewPnlField>,
    pub long_orders: Vec<OrderRecord>,
    pub short_orders: Vec<OrderRecord>,
}

impl ExecutedTrade {
    /// Both legs must identify the same environment; absent or mixed evidence stays unknown.
    #[must_use]
    pub fn execution_environment(&self) -> Option<crate::ExecutionEnvironment> {
        if self.long_orders.is_empty() || self.short_orders.is_empty() {
            return None;
        }
        let environment = self.long_orders[0].intent.mode.environment();
        self.long_orders.iter().chain(&self.short_orders)
            .all(|order| order.intent.mode.environment() == environment)
            .then_some(environment)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissReason {
    RiskBlocked,
    DepthInsufficient,
    LatencyExceeded,
    PriceMoved,
    ManualSkip,
    SignalDecayed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MissedOpportunity {
    pub id: String,
    pub opportunity_id: String,
    pub strategy: StrategyKind,
    pub symbol: String,
    pub detected_at_ms: i64,
    pub expected_pnl_usd: f64,
    pub reason: MissReason,
    pub detail: String,
}

#[cfg(test)]
mod tests;
