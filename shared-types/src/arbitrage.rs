//! 套利机会与统计 DTO。

use crate::enums::{ArbitrageType, Recommendation, RiskLevel};
use crate::fees::{RoundTripCostBreakdown, YieldBasis};
use crate::funding::FundingDiffWindowStats;
use crate::hedge::{
    ExecutionRun, ExecutionRunState, HedgeExecutionParams, HedgeLegRole, HedgeTicket,
    HedgeTicketOrderPlans, OrderCompilePlan, RecoveryAction,
};
use crate::history::{HistoryResponse, OpportunityHistoryRow};
use crate::index_composition::{IndexCompositionRiskProfile, IndexCompositionSnapshot};
use crate::list::ListStatus;
use crate::live_trading::{ExecutionEnvironment, OrderIntent, OrderRecord, RiskDecision};
use crate::market::{
    MarketDataEnvelope, MarketDataHealth, MarketDataQuality, MarketDataRowEvidence,
    MarketDataSnapshotStatus, MarketDataSourceKind, OrderBookInfo,
};
use crate::orders::{AccountBindingEvidence, AccountDataHealth, AccountFieldQuality};
use crate::problem::ApiProblem;
use crate::strategy::{is_p0_executable_strategy, StrategyCategory, StrategyKind};
use crate::venues::VenueOperationHealth;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const HEDGE_PREVIEW_MARKET_MAX_AGE_MS: i64 = 30_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FundingPrediction {
    pub long_bps: f64,
    pub short_bps: f64,
    pub net_bps: f64,
    pub confidence: f64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpportunityDataCoverage {
    pub funding_symbols: usize,
    pub funding_venues: usize,
    pub funding_rows: usize,
    pub perp_tickers: usize,
    pub spot_ticks: usize,
    pub index_compositions: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpportunityScanOutcome {
    Found,
    TrueEmpty,
    FilteredEmpty,
    #[default]
    Warming,
    PartialUpstream,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpportunityScanMeta {
    pub candidate_count: usize,
    pub emitted_count: usize,
    #[serde(default)]
    pub scan_outcome: OpportunityScanOutcome,
    pub dropped_extreme_yield_count: usize,
    pub dropped_non_positive_yield_count: usize,
    pub dropped_unprofitable_after_cost_count: usize,
    pub dropped_below_min_net_yield_count: usize,
    pub history_append_ok: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scan_started_at: Option<DateTime<Utc>>,
    pub scan_ms: u64,
    pub publish_ms: u64,
    pub market_data_problem_count: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub degraded_venues: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub market_data_status: Option<MarketDataSnapshotStatus>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub funding_row_evidence: Vec<MarketDataRowEvidence>,
    pub coverage: OpportunityDataCoverage,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpportunityScanReport {
    pub opportunities: Vec<ArbitrageOpportunityDto>,
    pub meta: OpportunityScanMeta,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpportunityEnvelopeStatus {
    #[default]
    Fresh,
    Warming,
    Stale,
    Degraded,
    Error,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpportunityEnvelopeScope {
    #[default]
    MainP0,
    RegistrySnapshot,
    Custom,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpportunityCountBreakdown {
    pub total_count: usize,
    pub executable_count: usize,
    #[serde(default)]
    pub strategy_counts: HashMap<StrategyKind, usize>,
    #[serde(default)]
    pub executable_strategy_counts: HashMap<StrategyKind, usize>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpportunityListSortKey {
    /// Legacy query value. Servers treat it as a profitability sort alias.
    Score,
    Settlement,
    #[default]
    NetSingleYield,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpotLegMode {
    BuySpot,
    SellInventory,
    BorrowAndSell,
}

impl SpotLegMode {
    pub const fn label_zh(self) -> &'static str {
        match self {
            Self::BuySpot => "买入现货",
            Self::SellInventory => "卖出现货库存",
            Self::BorrowAndSell => "借币卖出现货",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpportunityListPage {
    pub page_size: usize,
    pub start_offset: usize,
    pub returned_count: usize,
    pub total_rows: usize,
    pub has_next_page: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_cursor: Option<String>,
    pub sort_key: OpportunityListSortKey,
    pub snapshot_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpportunityQueryScopeMeta {
    pub global_total_count: usize,
    pub strategy_scope_count: usize,
    pub symbol_scope_count: usize,
    pub filtered_count: usize,
    pub page_count: usize,
    pub candidate_count: usize,
    pub emitted_count: usize,
}

/// One bounded first-page projection carried by the opportunity stream.
///
/// `strategy_kind = None` is the combined P0 window. The remaining windows
/// contain one strategy each, so product modules can switch tabs without a
/// second REST read of the same lifecycle snapshot.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpportunityStreamWindow {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy_kind: Option<StrategyKind>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ids: Vec<String>,
    pub page: OpportunityListPage,
    pub scope_meta: OpportunityQueryScopeMeta,
    pub query_key: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpportunityListFilterMeta {
    pub scope: OpportunityEnvelopeScope,
    #[serde(default)]
    pub strategy_kinds: Vec<StrategyKind>,
    pub symbol: Option<String>,
    pub min_yield: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpportunityListRequestMeta {
    pub fast: bool,
    pub fresh: bool,
    pub filter: OpportunityListFilterMeta,
    pub sort_key: OpportunityListSortKey,
    pub requested_page_size: Option<usize>,
    pub applied_page_size: usize,
    pub max_page_size: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpportunityListLegFunding {
    pub rate: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval_hours: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_funding_time_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpportunityListLeg {
    pub venue: String,
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub market_evidence: Option<OpportunityLegMarketEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub funding: Option<OpportunityListLegFunding>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpportunityListMetrics {
    /// Legacy input compatibility only. New responses never serialize a strategy score.
    #[serde(default, skip_serializing)]
    pub score: f64,
    pub risk_level: RiskLevel,
    pub net_single_yield: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annualized_funding_bps: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub one_cycle_net_bps: Option<f64>,
    pub time_to_settlement_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settlement_countdown_seconds: Option<i64>,
    pub liquidity_score: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpportunityListCost {
    pub verified: bool,
    pub gross_edge_bps: f64,
    pub total_cost_bps: f64,
    pub wear_bps: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub one_cycle_net_bps: Option<f64>,
    pub one_cycle_covers_cost: bool,
    pub breakeven_periods: u32,
    pub breakeven_hours: f64,
    pub recommended_hold_hours: f64,
    pub net_bps_at_recommended_hold: f64,
    pub fee_evidence_count: usize,
    pub fee_evidence_complete: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fee_evidence_ids: Vec<String>,
    #[serde(default)]
    pub one_cycle_penalty: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpportunityListExecution {
    pub eligible: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blockers: Vec<String>,
    pub optimal_position: f64,
    pub max_position: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpportunityListRow {
    pub id: String,
    pub symbol: String,
    pub strategy_kind: Option<StrategyKind>,
    pub strategy_category: Option<StrategyCategory>,
    pub type_label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spot_leg_mode: Option<SpotLegMode>,
    pub long_leg: OpportunityListLeg,
    pub short_leg: OpportunityListLeg,
    pub metrics: OpportunityListMetrics,
    pub cost: OpportunityListCost,
    pub execution: OpportunityListExecution,
    pub data_source: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpportunityListEnvelope {
    pub rows: Vec<OpportunityListRow>,
    pub page: OpportunityListPage,
    #[serde(default)]
    pub request_meta: OpportunityListRequestMeta,
    pub scope_meta: OpportunityQueryScopeMeta,
    #[serde(default)]
    pub main_p0_counts: OpportunityCountBreakdown,
    #[serde(default)]
    pub registry_counts: OpportunityCountBreakdown,
    pub meta: OpportunityScanMeta,
    pub status: OpportunityEnvelopeStatus,
    pub scope: OpportunityEnvelopeScope,
    pub query_key: String,
    pub source: String,
    pub cached_at: DateTime<Utc>,
    #[serde(default)]
    pub observed_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freshness_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ApiProblem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub partial_failures: Vec<ApiProblem>,
    #[serde(rename = "listing", default, skip_serializing_if = "String::is_empty")]
    pub instrument_coverage_diagnostics: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpportunityStreamEventKind {
    #[default]
    SnapshotInvalidated,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpportunityStreamEvent {
    pub event: OpportunityStreamEventKind,
    pub snapshot_id: String,
    pub scope_meta: OpportunityQueryScopeMeta,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changed_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changed_rows: Vec<OpportunityListRow>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub removed_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub top_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub windows: Vec<OpportunityStreamWindow>,
    pub main_p0_counts: OpportunityCountBreakdown,
    pub registry_counts: OpportunityCountBreakdown,
    pub meta: OpportunityScanMeta,
    pub status: OpportunityEnvelopeStatus,
    pub scope: OpportunityEnvelopeScope,
    pub query_key: String,
    pub source: String,
    pub cached_at: DateTime<Utc>,
    #[serde(default)]
    pub observed_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freshness_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ApiProblem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub partial_failures: Vec<ApiProblem>,
}

pub type OpportunityStreamPayload = OpportunityStreamEvent;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpportunityEnvelope {
    pub opportunities: Vec<ArbitrageOpportunityDto>,
    pub count: usize,
    pub total_count: usize,
    pub filtered_count: usize,
    pub before_limit_count: usize,
    pub returned_count: usize,
    pub visible_count: usize,
    pub executable_count: usize,
    #[serde(default)]
    pub strategy_counts: HashMap<StrategyKind, usize>,
    #[serde(default)]
    pub executable_strategy_counts: HashMap<StrategyKind, usize>,
    pub main_p0_counts: OpportunityCountBreakdown,
    pub registry_counts: OpportunityCountBreakdown,
    pub meta: OpportunityScanMeta,
    pub status: OpportunityEnvelopeStatus,
    pub scope: OpportunityEnvelopeScope,
    pub query_key: String,
    pub source: String,
    pub cached_at: DateTime<Utc>,
    #[serde(default)]
    pub observed_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freshness_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scan_started_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ApiProblem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub partial_failures: Vec<ApiProblem>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpportunityDetailRequest {
    #[serde(default)]
    pub depth: Option<u32>,
    #[serde(default)]
    pub history_limit: Option<usize>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpportunityRequestLimitMeta {
    pub requested: Option<usize>,
    pub applied: usize,
    pub max: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpportunityDetailRequestMeta {
    pub orderbook_depth: OpportunityRequestLimitMeta,
    pub history_limit: OpportunityRequestLimitMeta,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpportunityDetailEnvelope {
    #[serde(default)]
    pub request_meta: OpportunityDetailRequestMeta,
    pub opportunity: ArbitrageOpportunityDto,
    pub long_orderbook: MarketDataEnvelope<Option<OrderBookInfo>>,
    pub short_orderbook: MarketDataEnvelope<Option<OrderBookInfo>>,
    pub history: HistoryResponse<OpportunityHistoryRow>,
    pub long_index_composition: MarketDataEnvelope<Option<IndexCompositionSnapshot>>,
    pub short_index_composition: MarketDataEnvelope<Option<IndexCompositionSnapshot>>,
    pub status: OpportunityEnvelopeStatus,
    pub source: String,
    pub observed_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freshness_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ApiProblem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub partial_failures: Vec<ApiProblem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionCostProfile {
    pub gross_edge_bps: f64,
    pub fee_bps: f64,
    pub wear_bps: f64,
    pub total_cost_bps: f64,
    #[serde(default)]
    pub one_cycle: OneCycleCostProfile,
    pub breakeven_periods: u32,
    pub breakeven_hours: f64,
    pub recommended_hold_periods: u32,
    pub recommended_hold_hours: f64,
    pub net_bps_at_recommended_hold: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub round_trip: Option<RoundTripCostBreakdown>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OneCycleCostProfile {
    pub gross_edge_bps: f64,
    pub open_fee_bps: f64,
    pub close_fee_bps: f64,
    pub open_slippage_bps: f64,
    pub close_slippage_bps: f64,
    pub funding_window_mismatch_buffer_bps: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(target_arch = "wasm32", serde(skip_serializing))]
    pub yield_basis: Option<YieldBasis>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(target_arch = "wasm32", serde(skip_serializing))]
    pub long_next_settlement_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(target_arch = "wasm32", serde(skip_serializing))]
    pub short_next_settlement_ms: Option<i64>,
    pub target_buffer_bps: f64,
    pub net_bps: f64,
    pub covers_round_trip_cost: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScoreBreakdown {
    pub base_score: f64,
    pub yield_component: f64,
    pub liquidity_component: f64,
    pub risk_component: f64,
    pub cost_efficiency_component: f64,
    pub history_adjustment: f64,
    pub one_cycle_net_bps: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fee_evidence_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub fee_evidence_complete: bool,
    #[serde(default)]
    pub one_cycle_penalty: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price_gap_bps: Option<f64>,
    pub price_gap_penalty: f64,
    pub profit_quality: f64,
    pub price_gap_quality: f64,
    pub settlement_quality: f64,
    pub push_priority_bonus: f64,
    pub index_composition_penalty: f64,
    pub final_score: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RankingKey {
    pub final_score: f64,
    pub one_cycle_net_bps: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fee_evidence_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub fee_evidence_complete: bool,
    #[serde(default)]
    pub one_cycle_penalty: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price_gap_bps: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settlement_minutes: Option<f64>,
    pub liquidity_score: f64,
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpportunityLegMarketEvidence {
    pub venue: String,
    pub symbol: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price: Option<f64>,
    pub health: MarketDataHealth,
}

/// Executable stable-quote conversion used to compare or settle an opportunity.
///
/// An empty conversion list means both main legs use the same quote asset. Cross-quote
/// opportunities carry every required direction so discovery can request the exact spot
/// market over WS before the row becomes product-visible.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpportunityQuoteConversion {
    pub from_quote: String,
    pub to_quote: String,
    pub rate: f64,
    pub venue: String,
    pub symbol: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub market_evidence: Option<OpportunityLegMarketEvidence>,
}

/// 统一的套利机会 DTO（前端唯一消费结构）。
///
/// 对应 Python `core/arbitrage/interfaces.py::ArbitrageOpportunityDTO`。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArbitrageOpportunityDto {
    pub id: String,
    pub symbol: String,
    #[serde(rename = "type")]
    pub arb_type: ArbitrageType,
    pub type_label: String,

    pub long_exchange: String,
    pub short_exchange: String,

    /// Legacy comparison slots. For `PerpCross`, these carry the two native next-settlement
    /// rates and their edge; that strategy never applies eight-hour normalization.
    pub spread_8h: f64,
    pub long_rate_8h: f64,
    pub short_rate_8h: f64,

    pub long_rate: f64,
    pub short_rate: f64,

    pub single_yield: f64,
    pub net_single_yield: f64,
    pub raw_single_yield: f64,
    pub settlement_interval: u32,
    pub risk_adjusted_yield: f64,
    pub trading_cost_rate: f64,
    pub min_holding_periods: u32,

    pub risk_level: RiskLevel,
    pub volatility: f64,
    pub sharpe_ratio: f64,

    /// Legacy input compatibility only. New responses never serialize a strategy score.
    #[serde(default, skip_serializing)]
    pub score: f64,
    #[serde(default, skip_serializing)]
    pub score_breakdown: Option<ScoreBreakdown>,
    #[serde(default, skip_serializing)]
    pub ranking_key: Option<RankingKey>,
    pub recommendation: Recommendation,

    pub optimal_position: f64,
    pub max_position: f64,

    pub liquidity_score: f64,
    pub volume_24h: f64,
    #[serde(default)]
    pub long_volume_24h: f64,
    #[serde(default)]
    pub short_volume_24h: f64,

    pub data_source: String,
    pub confidence: f64,

    pub updated_at: DateTime<Utc>,

    /// 策略操作说明。
    pub long_funding_interval: u32,
    pub short_funding_interval: u32,
    #[serde(default)]
    pub settlement_time_diff: bool,
    #[serde(default)]
    pub strategy_description: String,
    #[serde(default)]
    pub long_action: String,
    #[serde(default)]
    pub short_action: String,

    /// 结算时间。
    #[serde(default)]
    pub long_next_funding_time: i64,
    #[serde(default)]
    pub short_next_funding_time: i64,
    #[serde(default)]
    pub time_to_settlement_ms: i64,
    #[serde(default)]
    pub is_snipe_ready: bool,

    /// 价格与基差信息。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub long_price: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub short_price: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub long_leg_market_evidence: Option<OpportunityLegMarketEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short_leg_market_evidence: Option<OpportunityLegMarketEvidence>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quote_conversions: Vec<OpportunityQuoteConversion>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price_deviation: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub basis_spread: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub basis_annual_cost: Option<f64>,

    #[serde(default)]
    pub risk_warnings: Vec<String>,
    #[serde(default = "default_execution_eligible")]
    pub execution_eligible: bool,
    #[serde(default)]
    pub execution_blockers: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_cost: Option<ExecutionCostProfile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index_composition: Option<IndexCompositionRiskProfile>,

    #[serde(default)]
    pub strategy_kind: Option<StrategyKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy_category: Option<StrategyCategory>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spot_leg_mode: Option<SpotLegMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub basis_bps: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annualized_funding_bps: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub triangular_path: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub onchain_metadata: Option<OnchainMetadata>,
    #[serde(default)]
    pub predicted_next_funding: Option<FundingPrediction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub funding_diff_window: Option<FundingDiffWindowStats>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub funding_diff_windows: Vec<FundingDiffWindowStats>,
    #[serde(default)]
    pub borrow_cost_bps_per_day: Option<f64>,
    #[serde(default)]
    pub funding_window_alignment_minutes: Option<i32>,
    #[serde(default)]
    pub funding_cap_distance_bps: Option<f64>,
    #[serde(default)]
    pub min_hold_hours: Option<f64>,
    #[serde(default)]
    pub settlement_countdown_seconds: Option<i64>,
}

fn default_execution_eligible() -> bool {
    false
}

/// Returns whether an opportunity may enter hedge preview.
///
/// Preview deliberately excludes orderbook depth. The ticket builder fetches both books for the
/// requested notional and turns missing or insufficient depth into a scoped ticket blocker before
/// confirmation. Opportunity rows cannot authorize execution by themselves.
pub fn is_hedge_preview_ready(row: &ArbitrageOpportunityDto) -> bool {
    is_hedge_preview_ready_at(row, current_timestamp_ms())
}

#[cfg(target_arch = "wasm32")]
fn current_timestamp_ms() -> i64 {
    js_sys::Date::now().clamp(0.0, i64::MAX as f64) as i64
}

#[cfg(not(target_arch = "wasm32"))]
fn current_timestamp_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            duration.as_millis().min(i64::MAX as u128) as i64
        })
}

pub fn is_hedge_preview_ready_at(row: &ArbitrageOpportunityDto, now_ms: i64) -> bool {
    row.strategy_kind.is_some_and(is_p0_executable_strategy)
        && row.execution_eligible
        && row.execution_blockers.is_empty()
        && has_fresh_market_evidence(row.long_leg_market_evidence.as_ref(), now_ms)
        && has_fresh_market_evidence(row.short_leg_market_evidence.as_ref(), now_ms)
        && has_fresh_quote_conversions(&row.quote_conversions, now_ms)
        && has_verified_round_trip_cost(row.execution_cost.as_ref(), now_ms)
        && has_positive_conservative_net(row.execution_cost.as_ref())
}

fn has_fresh_quote_conversions(conversions: &[OpportunityQuoteConversion], now_ms: i64) -> bool {
    conversions.iter().all(|conversion| {
        conversion.rate.is_finite()
            && conversion.rate > 0.0
            && !conversion.venue.trim().is_empty()
            && !conversion.symbol.trim().is_empty()
            && has_fresh_market_evidence(conversion.market_evidence.as_ref(), now_ms)
    })
}

fn has_positive_conservative_net(cost: Option<&ExecutionCostProfile>) -> bool {
    cost.is_some_and(|cost| {
        cost.one_cycle.covers_round_trip_cost
            && cost.one_cycle.net_bps.is_finite()
            && cost.one_cycle.net_bps > f64::EPSILON
    })
}

fn has_fresh_market_evidence(evidence: Option<&OpportunityLegMarketEvidence>, now_ms: i64) -> bool {
    evidence.is_some_and(|evidence| {
        evidence.health.quality == MarketDataQuality::Fresh
            && evidence.health.source == MarketDataSourceKind::WsPush
            && evidence.health.observed_at_ms > 0
            && evidence.health.observed_at_ms <= now_ms
            && now_ms.saturating_sub(evidence.health.observed_at_ms)
                <= HEDGE_PREVIEW_MARKET_MAX_AGE_MS
    })
}

fn has_verified_round_trip_cost(cost: Option<&ExecutionCostProfile>, now_ms: i64) -> bool {
    cost.and_then(|cost| cost.round_trip.as_ref())
        .is_some_and(|cost| {
            leg_has_fresh_verified_fee_snapshot(&cost.long_leg, now_ms)
                && leg_has_fresh_verified_fee_snapshot(&cost.short_leg, now_ms)
        })
}

fn leg_has_fresh_verified_fee_snapshot(leg: &crate::fees::LegCostBreakdown, now_ms: i64) -> bool {
    leg.fee_snapshot
        .as_ref()
        .is_some_and(|snapshot| snapshot.is_fresh_verified(now_ms))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainMetadata {
    pub chain: String,
    pub dex: String,
    pub pool_address: String,
    pub gas_usd_estimate: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ArbitrageStats {
    pub total: usize,
    pub max_yield: f64,
    pub avg_yield: f64,
    pub spot_futures_count: usize,
    pub cross_futures_count: usize,
    pub cross_spot_count: usize,
    pub options_hedge_count: usize,
}

/// 套利引擎运行参数。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArbitrageConfig {
    pub min_spread: f64,
    pub min_volume_24h: f64,
    pub min_net_yield: f64,
    pub max_single_yield: f64,
    pub ewma_span: u32,
    pub outlier_zscore: f64,
    pub default_slippage: f64,
    pub risk_free_rate: f64,
    pub default_volatility: f64,
    pub max_position_ratio: f64,
    pub max_exchange_ratio: f64,
    pub max_symbol_ratio: f64,
    pub kelly_scaling: f64,
    pub rest_qps_limit: u32,
    pub ws_reconnect_delay_secs: u32,
    pub circuit_breaker_threshold: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HedgePreviewRequest {
    #[serde(default)]
    pub opportunity_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opportunity_snapshot_id: Option<String>,
    pub capital_usd: f64,
    pub leverage: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub long_price: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short_price: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub long_notional_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short_notional_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_params: Option<HedgeExecutionParams>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HedgePreviewPositionsEvidence {
    pub status: ListStatus,
    pub source: String,
    pub observed_at_ms: i64,
    pub row_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_account_liq_distance_pct: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub problems: Vec<ApiProblem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub operation_health: Vec<VenueOperationHealth>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub field_quality: Vec<AccountFieldQuality>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub row_health: Vec<AccountDataHealth>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub account_bindings: Vec<AccountBindingEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HedgePreviewResponse {
    pub opportunity_id: String,
    #[serde(default)]
    pub opportunity_snapshot_id: String,
    /// Echoes the request binding when preflight adopts a newer snapshot of the same opportunity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_opportunity_snapshot_id: Option<String>,
    pub ticket: HedgeTicket,
    #[serde(default)]
    pub workflow_view: crate::workflow::HedgeTicketView,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticket_order_plans: Option<HedgeTicketOrderPlans>,
    pub long_leg: OrderIntent,
    pub long_risk: RiskDecision,
    pub short_leg: OrderIntent,
    pub short_risk: RiskDecision,
    /// Legacy response evidence remains deserializable but cannot authorize execution.
    #[serde(default, skip_serializing)]
    pub long_order_plan: Option<OrderCompilePlan>,
    /// Legacy response evidence remains deserializable but cannot authorize execution.
    #[serde(default, skip_serializing)]
    pub short_order_plan: Option<OrderCompilePlan>,
    /// Funding value for the ticket's next aligned native settlement.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_funding_next_settlement_usd: Option<f64>,
    /// Legacy wire field kept for older clients. For `PerpCross` it mirrors
    /// `estimated_funding_next_settlement_usd` and is not an eight-hour normalization.
    pub estimated_funding_per_8h_usd: f64,
    /// Gross strategy edge before open, close, and slippage costs.
    #[serde(default)]
    pub estimated_gross_edge_usd: f64,
    pub estimated_open_cost_usd: f64,
    pub estimated_close_cost_usd: f64,
    pub estimated_slippage_usd: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_account_liq_distance_pct: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_hedge_liq_distance_pct: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub positions_evidence: Option<HedgePreviewPositionsEvidence>,
    pub used_capital_usd: f64,
    pub max_loss_usd: f64,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HedgeConfirmRequest {
    pub idempotency_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticket_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HedgeConfirmStatus {
    Submitted,
    Replayed,
    LongLegFailed,
    ValuationMissing,
    FirstLegPartialUnwindAttempted,
    FirstLegPartialUnwindFailed,
    FirstLegPartialWaitingFillQty,
    HedgeRecheckBlockedUnwindAttempted,
    HedgeRecheckBlockedUnwindFailed,
    HedgeBrokenUnwindAttempted,
    HedgeBrokenUnwindFailed,
    #[serde(other)]
    Unknown,
}

impl HedgeConfirmStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Submitted => "submitted",
            Self::Replayed => "replayed",
            Self::LongLegFailed => "long_leg_failed",
            Self::ValuationMissing => "valuation_missing",
            Self::FirstLegPartialUnwindAttempted => "first_leg_partial_unwind_attempted",
            Self::FirstLegPartialUnwindFailed => "first_leg_partial_unwind_failed",
            Self::FirstLegPartialWaitingFillQty => "first_leg_partial_waiting_fill_qty",
            Self::HedgeRecheckBlockedUnwindAttempted => "hedge_recheck_blocked_unwind_attempted",
            Self::HedgeRecheckBlockedUnwindFailed => "hedge_recheck_blocked_unwind_failed",
            Self::HedgeBrokenUnwindAttempted => "hedge_broken_unwind_attempted",
            Self::HedgeBrokenUnwindFailed => "hedge_broken_unwind_failed",
            Self::Unknown => "unknown",
        }
    }
}

impl std::fmt::Display for HedgeConfirmStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HedgeConfirmPartialCause {
    FirstLegPartial,
    HedgeRecheckBlocked,
    HedgeBroken,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HedgeConfirmUnwindStatus {
    Submitted,
    SubmitFailed,
    AwaitingFillQuantity,
    #[serde(other)]
    Unknown,
}

/// Stable identity and venue-scoped failure evidence for one hedge confirmation.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HedgeConfirmContext {
    pub opportunity_id: String,
    pub idempotency_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticket_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<ExecutionEnvironment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub long_venue: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short_venue: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub long_problem: Option<ApiProblem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short_problem: Option<ApiProblem>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HedgeConfirmPartialOutcome {
    #[serde(default)]
    pub context: HedgeConfirmContext,
    pub cause: HedgeConfirmPartialCause,
    pub original_status: HedgeConfirmStatus,
    pub run_id: String,
    pub run_state: ExecutionRunState,
    pub net_exposure_usd: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery_action: Option<RecoveryAction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_problem: Option<ApiProblem>,
    pub unwind_status: HedgeConfirmUnwindStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unwind_target_leg: Option<HedgeLegRole>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unwind_quantity: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unwind_problem: Option<ApiProblem>,
    pub manual_review_required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HedgeConfirmResponse {
    pub idempotency_key: String,
    pub status: HedgeConfirmStatus,
    #[serde(default)]
    pub context: HedgeConfirmContext,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_run: Option<ExecutionRun>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub long_record: Option<OrderRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub short_record: Option<OrderRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unwind_record: Option<OrderRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<ApiProblem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub partial_outcome: Option<HedgeConfirmPartialOutcome>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Default for ArbitrageConfig {
    fn default() -> Self {
        Self {
            min_spread: 0.000_05,
            min_volume_24h: 100_000.0,
            min_net_yield: 0.000_05,
            max_single_yield: 0.05,
            ewma_span: 10,
            outlier_zscore: 3.0,
            default_slippage: 0.000_2,
            risk_free_rate: 0.05,
            default_volatility: 0.5,
            max_position_ratio: 0.1,
            max_exchange_ratio: 0.3,
            max_symbol_ratio: 0.2,
            kelly_scaling: 0.5,
            rest_qps_limit: 5,
            ws_reconnect_delay_secs: 5,
            circuit_breaker_threshold: 5,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    #[test]
    fn hedge_confirm_status_round_trips_snake_case() -> Result<(), serde_json::Error> {
        let value = serde_json::to_value(HedgeConfirmStatus::FirstLegPartialWaitingFillQty)?;
        assert_eq!(value, json!("first_leg_partial_waiting_fill_qty"));
        let parsed: HedgeConfirmStatus = serde_json::from_value(value)?;
        assert_eq!(parsed, HedgeConfirmStatus::FirstLegPartialWaitingFillQty);
        assert_eq!(parsed.to_string(), "first_leg_partial_waiting_fill_qty");
        Ok(())
    }

    #[test]
    fn hedge_confirm_status_unknown_legacy_string_fails_open_to_unknown(
    ) -> Result<(), serde_json::Error> {
        let parsed: HedgeConfirmStatus = serde_json::from_value(json!("some_legacy_status"))?;
        assert_eq!(parsed, HedgeConfirmStatus::Unknown);
        Ok(())
    }

    #[test]
    fn hedge_confirm_response_accepts_legacy_payload_without_partial_outcome(
    ) -> Result<(), serde_json::Error> {
        let response: HedgeConfirmResponse = serde_json::from_value(json!({
            "idempotencyKey": "idem-1",
            "status": "submitted",
            "longRecord": null,
            "shortRecord": null,
            "unwindRecord": null,
            "error": null
        }))?;

        assert_eq!(response.status, HedgeConfirmStatus::Submitted);
        assert_eq!(response.context, HedgeConfirmContext::default());
        assert!(response.partial_outcome.is_none());
        Ok(())
    }

    #[test]
    fn hedge_confirm_partial_outcome_uses_camel_case_and_snake_case_enums(
    ) -> Result<(), serde_json::Error> {
        let response = HedgeConfirmResponse {
            idempotency_key: "idem-1".into(),
            status: HedgeConfirmStatus::FirstLegPartialUnwindFailed,
            context: HedgeConfirmContext {
                opportunity_id: "opp-1".into(),
                idempotency_key: "idem-1".into(),
                ticket_id: Some("ticket-1".into()),
                run_id: Some("run-1".into()),
                environment: Some(ExecutionEnvironment::Live),
                long_venue: Some("okx".into()),
                short_venue: Some("bybit".into()),
                long_problem: None,
                short_problem: Some(ApiProblem::new("RATE_LIMITED", "short failed")),
            },
            execution_run: None,
            long_record: None,
            short_record: None,
            unwind_record: None,
            problem: None,
            partial_outcome: Some(HedgeConfirmPartialOutcome {
                context: HedgeConfirmContext::default(),
                cause: HedgeConfirmPartialCause::FirstLegPartial,
                original_status: HedgeConfirmStatus::FirstLegPartialUnwindFailed,
                run_id: "run-1".into(),
                run_state: ExecutionRunState::UnwindRequired,
                net_exposure_usd: 42.0,
                recovery_action: Some(RecoveryAction::ManualReview),
                primary_message: Some("first leg partial".into()),
                primary_problem: None,
                unwind_status: HedgeConfirmUnwindStatus::SubmitFailed,
                unwind_target_leg: Some(HedgeLegRole::Long),
                unwind_quantity: Some(0.4),
                unwind_problem: Some(ApiProblem::new(
                    crate::problem::codes::HEDGE_UNWIND_SUBMIT_FAILED,
                    "route down",
                )),
                manual_review_required: true,
            }),
            error: Some("route down".into()),
        };

        let json = serde_json::to_value(response)?;

        assert_eq!(json["partialOutcome"]["cause"], "first_leg_partial");
        assert_eq!(
            json["partialOutcome"]["originalStatus"],
            "first_leg_partial_unwind_failed"
        );
        assert_eq!(json["partialOutcome"]["runState"], "unwind_required");
        assert_eq!(json["partialOutcome"]["unwindStatus"], "submit_failed");
        assert_eq!(json["partialOutcome"]["unwindTargetLeg"], "long");
        assert_eq!(json["partialOutcome"]["manualReviewRequired"], true);
        assert_eq!(json["context"]["ticketId"], "ticket-1");
        assert_eq!(json["context"]["runId"], "run-1");
        assert_eq!(json["context"]["environment"], "live");
        assert_eq!(json["context"]["longVenue"], "okx");
        assert_eq!(json["context"]["shortProblem"]["code"], "RATE_LIMITED");
        Ok(())
    }

    #[test]
    fn legacy_opportunity_without_execution_flag_fails_closed() -> Result<(), serde_json::Error> {
        let dto: ArbitrageOpportunityDto = serde_json::from_value(opportunity_payload(None))?;

        assert!(!dto.execution_eligible);
        Ok(())
    }

    #[test]
    fn explicit_execution_flag_is_preserved() -> Result<(), serde_json::Error> {
        let dto: ArbitrageOpportunityDto = serde_json::from_value(opportunity_payload(Some(true)))?;

        assert!(dto.execution_eligible);
        Ok(())
    }

    #[test]
    fn hedge_preview_requires_market_and_fee_evidence() -> Result<(), serde_json::Error> {
        let mut dto: ArbitrageOpportunityDto =
            serde_json::from_value(opportunity_payload(Some(true)))?;
        dto.strategy_kind = Some(StrategyKind::PerpCross);
        dto.long_leg_market_evidence = Some(market_evidence("binance"));
        dto.short_leg_market_evidence = Some(market_evidence("kucoin"));
        dto.execution_cost = Some(verified_cost());

        assert!(is_hedge_preview_ready_at(&dto, TEST_NOW_MS));

        dto.execution_cost = None;
        assert!(!is_hedge_preview_ready_at(&dto, TEST_NOW_MS));
        Ok(())
    }

    #[test]
    fn opportunity_wire_contract_carries_no_orderbook_depth() -> Result<(), serde_json::Error> {
        let mut dto = execution_ready_opportunity()?;
        dto.execution_cost = Some(verified_cost());
        let value = serde_json::to_value(&dto)?;

        assert!(is_hedge_preview_ready_at(&dto, TEST_NOW_MS));
        assert!(value.get("longLegDepthUsd5bps").is_none());
        assert!(value.get("shortLegDepthUsd5bps").is_none());
        Ok(())
    }

    #[test]
    fn hedge_preview_rejects_fresh_rest_price_evidence() -> Result<(), serde_json::Error> {
        let mut dto = execution_ready_opportunity()?;
        dto.execution_cost = Some(verified_cost());
        if let Some(evidence) = dto.long_leg_market_evidence.as_mut() {
            evidence.health.source = MarketDataSourceKind::RestBaseline;
        }

        assert!(!is_hedge_preview_ready_at(&dto, TEST_NOW_MS));
        Ok(())
    }

    #[test]
    fn hedge_preview_requires_exact_quote_conversion_ws_evidence() -> Result<(), serde_json::Error>
    {
        let mut dto = execution_ready_opportunity()?;
        dto.execution_cost = Some(verified_cost());
        dto.quote_conversions = vec![OpportunityQuoteConversion {
            from_quote: "USDC".into(),
            to_quote: "USDT".into(),
            rate: 0.999,
            venue: "kucoin".into(),
            symbol: "USDC-USDT".into(),
            market_evidence: None,
        }];

        assert!(!is_hedge_preview_ready_at(&dto, TEST_NOW_MS));
        dto.quote_conversions[0].market_evidence = Some(market_evidence("kucoin"));
        assert!(is_hedge_preview_ready_at(&dto, TEST_NOW_MS));
        Ok(())
    }

    #[test]
    fn hedge_preview_rejects_ws_evidence_older_than_the_artifact_window(
    ) -> Result<(), serde_json::Error> {
        let mut dto = execution_ready_opportunity()?;
        dto.execution_cost = Some(verified_cost());
        if let Some(evidence) = dto.long_leg_market_evidence.as_mut() {
            evidence.health.observed_at_ms = TEST_NOW_MS - HEDGE_PREVIEW_MARKET_MAX_AGE_MS - 1;
        }

        assert!(!is_hedge_preview_ready_at(&dto, TEST_NOW_MS));
        Ok(())
    }

    #[test]
    fn hedge_preview_rejects_expired_fee_snapshot() -> Result<(), serde_json::Error> {
        let mut dto = execution_ready_opportunity()?;
        dto.execution_cost = Some(verified_cost());

        assert!(is_hedge_preview_ready_at(&dto, TEST_NOW_MS));
        assert!(!is_hedge_preview_ready_at(&dto, TEST_AFTER_FEE_EXPIRY_MS));
        Ok(())
    }

    #[test]
    fn hedge_preview_rejects_fee_snapshot_verification_problem() -> Result<(), serde_json::Error> {
        let mut dto = execution_ready_opportunity()?;
        let mut cost = verified_cost();
        if let Some(round_trip) = cost.round_trip.as_mut() {
            if let Some(snapshot) = round_trip.long_leg.fee_snapshot.as_mut() {
                snapshot.verification_problem = Some("official fee schedule mismatch".into());
            }
        }
        dto.execution_cost = Some(cost);

        assert!(!is_hedge_preview_ready_at(&dto, TEST_NOW_MS));
        Ok(())
    }

    #[test]
    fn score_breakdown_missing_one_cycle_penalty_defaults_to_zero() -> Result<(), serde_json::Error>
    {
        let breakdown: ScoreBreakdown = serde_json::from_value(json!({
            "baseScore": 10.0,
            "yieldComponent": 1.0,
            "liquidityComponent": 1.0,
            "riskComponent": 1.0,
            "costEfficiencyComponent": 1.0,
            "historyAdjustment": 0.0,
            "oneCycleNetBps": 12.0,
            "feeEvidenceIds": ["fee:binance:perp:vip0"],
            "feeEvidenceComplete": true,
            "priceGapPenalty": 0.5,
            "profitQuality": 0.8,
            "priceGapQuality": 0.9,
            "settlementQuality": 1.0,
            "pushPriorityBonus": 2.0,
            "indexCompositionPenalty": 0.0,
            "finalScore": 11.0
        }))?;

        assert_eq!(breakdown.one_cycle_penalty, 0.0);
        assert_eq!(
            breakdown.fee_evidence_ids,
            vec!["fee:binance:perp:vip0".to_owned()]
        );
        Ok(())
    }

    #[test]
    fn ranking_key_missing_fee_evidence_defaults_fail_closed() -> Result<(), serde_json::Error> {
        let key: RankingKey = serde_json::from_value(json!({
            "finalScore": 11.0,
            "oneCycleNetBps": 12.0,
            "priceGapBps": 3.0,
            "settlementMinutes": 4.0,
            "liquidityScore": 5.0
        }))?;

        assert!(key.fee_evidence_ids.is_empty());
        assert!(!key.fee_evidence_complete);
        assert_eq!(key.one_cycle_penalty, 0.0);
        Ok(())
    }

    #[test]
    fn opportunity_leg_market_evidence_uses_camel_case() -> Result<(), serde_json::Error> {
        let mut payload = opportunity_payload(Some(true));
        if let Some(object) = payload.as_object_mut() {
            object.insert(
                "longLegMarketEvidence".to_owned(),
                json!({
                    "venue": "binance",
                    "symbol": "MU",
                    "price": 100.0,
                    "health": {
                        "quality": "fresh",
                        "source": "local_cache",
                        "freshnessMs": 10,
                        "observedAtMs": 1
                    }
                }),
            );
        }

        let dto: ArbitrageOpportunityDto = serde_json::from_value(payload)?;

        let evidence = dto.long_leg_market_evidence.as_ref();
        assert_eq!(evidence.map(|item| item.venue.as_str()), Some("binance"));
        assert_eq!(evidence.and_then(|item| item.price), Some(100.0));
        Ok(())
    }

    #[test]
    fn opportunity_list_contract_uses_camel_case() -> Result<(), serde_json::Error> {
        let cached_at = Utc::now();
        let envelope = OpportunityListEnvelope {
            rows: Vec::new(),
            page: OpportunityListPage {
                page_size: 25,
                start_offset: 50,
                returned_count: 0,
                total_rows: 50,
                has_next_page: false,
                next_cursor: None,
                previous_cursor: Some("v1:25:scope".into()),
                last_cursor: None,
                sort_key: OpportunityListSortKey::Score,
                snapshot_id: "snap-1".into(),
            },
            request_meta: OpportunityListRequestMeta {
                fast: true,
                fresh: false,
                filter: OpportunityListFilterMeta {
                    scope: OpportunityEnvelopeScope::MainP0,
                    strategy_kinds: vec![StrategyKind::PerpCross],
                    symbol: Some("MU".into()),
                    min_yield: Some(0.1),
                },
                sort_key: OpportunityListSortKey::Score,
                requested_page_size: Some(25),
                applied_page_size: 25,
                max_page_size: 120,
            },
            scope_meta: OpportunityQueryScopeMeta {
                global_total_count: 100,
                strategy_scope_count: 80,
                symbol_scope_count: 10,
                filtered_count: 50,
                page_count: 2,
                candidate_count: 120,
                emitted_count: 100,
            },
            main_p0_counts: OpportunityCountBreakdown {
                total_count: 80,
                executable_count: 12,
                ..OpportunityCountBreakdown::default()
            },
            registry_counts: OpportunityCountBreakdown {
                total_count: 100,
                executable_count: 12,
                ..OpportunityCountBreakdown::default()
            },
            meta: OpportunityScanMeta {
                scan_started_at: Some(cached_at),
                scan_outcome: OpportunityScanOutcome::Found,
                ..OpportunityScanMeta::default()
            },
            status: OpportunityEnvelopeStatus::Fresh,
            scope: OpportunityEnvelopeScope::MainP0,
            query_key: "test".into(),
            source: "snapshot".into(),
            cached_at,
            observed_at_ms: 1,
            freshness_ms: Some(2),
            retry_after_ms: None,
            error: None,
            partial_failures: Vec::new(),
            instrument_coverage_diagnostics: String::new(),
        };

        let json = serde_json::to_value(envelope)?;

        assert_eq!(json["page"]["pageSize"], 25);
        assert_eq!(json["page"]["previousCursor"], "v1:25:scope");
        assert_eq!(json["requestMeta"]["fast"], true);
        assert_eq!(json["requestMeta"]["filter"]["scope"], "main_p0");
        assert_eq!(
            json["requestMeta"]["filter"]["strategyKinds"][0],
            "perp_cross"
        );
        assert_eq!(json["requestMeta"]["requestedPageSize"], 25);
        assert_eq!(json["requestMeta"]["appliedPageSize"], 25);
        assert_eq!(json["requestMeta"]["maxPageSize"], 120);
        assert_eq!(json["scopeMeta"]["globalTotalCount"], 100);
        assert_eq!(json["mainP0Counts"]["totalCount"], 80);
        assert_eq!(json["registryCounts"]["totalCount"], 100);
        assert!(json["meta"]["scanStartedAt"].is_string());
        assert_eq!(json["meta"]["scanOutcome"], "found");
        assert!(json["rows"].is_array());
        Ok(())
    }

    #[test]
    fn opportunity_list_metrics_preserve_missing_settlement_countdown(
    ) -> Result<(), serde_json::Error> {
        let metrics: OpportunityListMetrics = serde_json::from_value(json!({
            "score": 80.0,
            "riskLevel": "low",
            "netSingleYield": 0.001,
            "timeToSettlementMs": 0,
            "liquidityScore": 75.0
        }))?;

        assert_eq!(metrics.settlement_countdown_seconds, None);
        assert!(serde_json::to_value(metrics)?
            .get("settlementCountdownSeconds")
            .is_none());
        Ok(())
    }

    #[test]
    fn opportunity_detail_contract_uses_camel_case() -> Result<(), serde_json::Error> {
        let envelope = OpportunityDetailEnvelope {
            request_meta: OpportunityDetailRequestMeta {
                orderbook_depth: OpportunityRequestLimitMeta {
                    requested: Some(5),
                    applied: 5,
                    max: 100,
                },
                history_limit: OpportunityRequestLimitMeta {
                    requested: Some(6),
                    applied: 6,
                    max: 50,
                },
            },
            opportunity: serde_json::from_value(opportunity_payload(Some(true)))?,
            long_orderbook: empty_market_envelope::<OrderBookInfo>(),
            short_orderbook: empty_market_envelope::<OrderBookInfo>(),
            history: crate::history::HistoryResponse {
                count: 0,
                rows: Vec::new(),
                page: None,
                row_cap: None,
                backend_status: Default::default(),
                storage_health: None,
                source: "memory".into(),
                observed_at_ms: 1,
                latest_at_ms: None,
                freshness_ms: None,
                problem: None,
                retry_after_ms: None,
                problems: Vec::new(),
            },
            long_index_composition: empty_market_envelope::<IndexCompositionSnapshot>(),
            short_index_composition: empty_market_envelope::<IndexCompositionSnapshot>(),
            status: OpportunityEnvelopeStatus::Fresh,
            source: "opportunity-detail".into(),
            observed_at_ms: 1,
            freshness_ms: Some(2),
            retry_after_ms: None,
            error: None,
            partial_failures: Vec::new(),
            request_id: Some("rid-1".into()),
        };

        let json = serde_json::to_value(envelope)?;

        assert!(json.get("longOrderbook").is_some());
        assert_eq!(json["requestMeta"]["orderbookDepth"]["applied"], 5);
        assert!(json.get("shortOrderbook").is_some());
        assert!(json.get("longIndexComposition").is_some());
        assert!(json.get("shortIndexComposition").is_some());
        assert_eq!(json["requestId"], "rid-1");
        assert_eq!(json["freshnessMs"], 2);
        Ok(())
    }

    #[test]
    fn hedge_preview_positions_evidence_uses_camel_case() -> Result<(), serde_json::Error> {
        let evidence = HedgePreviewPositionsEvidence {
            status: ListStatus::Degraded,
            source: "account_position_runtime".into(),
            observed_at_ms: 42,
            row_count: 1,
            current_account_liq_distance_pct: None,
            problems: vec![ApiProblem::new("POSITION_FIELD_UNAVAILABLE", "missing liq")],
            operation_health: Vec::new(),
            field_quality: Vec::new(),
            row_health: Vec::new(),
            account_bindings: Vec::new(),
            request_id: Some("req-1".into()),
            retry_after_ms: Some(2_000),
        };

        let json = serde_json::to_value(evidence)?;

        assert_eq!(json["observedAtMs"], 42);
        assert_eq!(json["rowCount"], 1);
        assert_eq!(json["requestId"], "req-1");
        assert_eq!(json["retryAfterMs"], 2_000);
        assert!(json.get("currentAccountLiqDistancePct").is_none());
        Ok(())
    }

    #[test]
    fn hedge_preview_positions_evidence_accepts_legacy_payload_without_row_evidence(
    ) -> Result<(), serde_json::Error> {
        let evidence: HedgePreviewPositionsEvidence = serde_json::from_value(json!({
            "status": "fresh",
            "source": "account_position_runtime",
            "observedAtMs": 42,
            "rowCount": 1
        }))?;

        assert!(evidence.row_health.is_empty());
        assert!(evidence.account_bindings.is_empty());
        Ok(())
    }

    #[test]
    fn hedge_preview_positions_evidence_serializes_row_health_and_account_bindings(
    ) -> Result<(), serde_json::Error> {
        let mut evidence: HedgePreviewPositionsEvidence = serde_json::from_value(json!({
            "status": "fresh",
            "source": "account_position_runtime",
            "observedAtMs": 42,
            "rowCount": 1
        }))?;
        evidence
            .row_health
            .push(crate::orders::AccountDataHealth::new(
                crate::orders::AccountFieldSubject::position("binance", "BTCUSDT", "long"),
                "account_position_runtime",
                42,
            ));
        evidence
            .account_bindings
            .push(crate::orders::AccountBindingEvidence {
                venue: "binance".into(),
                account_scope: Some("usds_m_futures".into()),
                status: crate::orders::AccountBindingStatus::Verified,
                source: "credential_probe:account_mode_read".into(),
                checked_at_ms: Some(42),
                freshness_ms: Some(0),
                credential_fingerprint: Some("hmac-sha256:test".into()),
                problem: None,
            });

        let json = serde_json::to_value(evidence)?;

        assert_eq!(json["rowHealth"][0]["subject"]["kind"], "position");
        assert_eq!(json["accountBindings"][0]["accountScope"], "usds_m_futures");
        Ok(())
    }

    #[test]
    fn opportunity_stream_payload_accepts_notice_without_rows() -> Result<(), serde_json::Error> {
        let payload = serde_json::to_value(OpportunityStreamEvent {
            event: OpportunityStreamEventKind::SnapshotInvalidated,
            snapshot_id: "snap-1".into(),
            scope_meta: OpportunityQueryScopeMeta {
                global_total_count: 100,
                strategy_scope_count: 80,
                symbol_scope_count: 80,
                filtered_count: 80,
                page_count: 4,
                candidate_count: 120,
                emitted_count: 100,
            },
            changed_ids: Vec::new(),
            changed_rows: Vec::new(),
            removed_ids: Vec::new(),
            top_ids: vec!["opp-1".into(), "opp-2".into()],
            windows: Vec::new(),
            main_p0_counts: OpportunityCountBreakdown {
                total_count: 80,
                executable_count: 12,
                ..OpportunityCountBreakdown::default()
            },
            registry_counts: OpportunityCountBreakdown {
                total_count: 100,
                executable_count: 12,
                ..OpportunityCountBreakdown::default()
            },
            meta: OpportunityScanMeta {
                scan_started_at: Some(Utc::now()),
                ..OpportunityScanMeta::default()
            },
            status: OpportunityEnvelopeStatus::Fresh,
            scope: OpportunityEnvelopeScope::MainP0,
            query_key: "scope=main_p0".into(),
            source: "snapshot".into(),
            cached_at: Utc::now(),
            observed_at_ms: 1,
            freshness_ms: Some(2),
            retry_after_ms: None,
            error: None,
            partial_failures: Vec::new(),
        })?;

        let decoded: OpportunityStreamPayload = serde_json::from_value(payload)?;

        assert_eq!(decoded.snapshot_id, "snap-1");
        assert_eq!(decoded.scope_meta.filtered_count, 80);
        assert_eq!(decoded.top_ids, vec!["opp-1", "opp-2"]);
        assert!(decoded.meta.scan_started_at.is_some());
        Ok(())
    }

    #[test]
    fn opportunity_stream_payload_rejects_legacy_full_envelope() {
        let legacy_payload = json!({
            "opportunities": [],
            "count": 0,
            "totalCount": 0,
            "filteredCount": 0,
            "cachedAt": "2026-01-01T00:00:00Z",
            "source": "legacy-full-envelope"
        });

        let decoded = serde_json::from_value::<OpportunityStreamPayload>(legacy_payload);

        assert!(decoded.is_err());
    }

    const TEST_NOW_MS: i64 = 1_780_186_600_000;
    const TEST_AFTER_FEE_EXPIRY_MS: i64 = 1_780_272_000_001;

    fn execution_ready_opportunity() -> Result<ArbitrageOpportunityDto, serde_json::Error> {
        let mut dto: ArbitrageOpportunityDto =
            serde_json::from_value(opportunity_payload(Some(true)))?;
        dto.strategy_kind = Some(StrategyKind::PerpCross);
        dto.long_leg_market_evidence = Some(market_evidence("binance"));
        dto.short_leg_market_evidence = Some(market_evidence("kucoin"));
        Ok(dto)
    }

    fn opportunity_payload(execution_eligible: Option<bool>) -> Value {
        let mut payload = json!({
            "id": "legacy-opportunity",
            "symbol": "BTC",
            "type": "cross_exchange",
            "typeLabel": "跨所期期",
            "longExchange": "binance",
            "shortExchange": "okx",
            "spread8h": 0.0,
            "longRate8h": 0.0,
            "shortRate8h": 0.0,
            "longRate": 0.0,
            "shortRate": 0.0,
            "singleYield": 0.0,
            "netSingleYield": 0.0,
            "rawSingleYield": 0.0,
            "settlementInterval": 8,
            "riskAdjustedYield": 0.0,
            "tradingCostRate": 0.0,
            "minHoldingPeriods": 1,
            "riskLevel": "low",
            "volatility": 0.0,
            "sharpeRatio": 0.0,
            "score": 0.0,
            "recommendation": "hold",
            "optimalPosition": 0.0,
            "maxPosition": 0.0,
            "liquidityScore": 0.0,
            "volume24h": 0.0,
            "dataSource": "test",
            "confidence": 0.0,
            "updatedAt": "2026-01-01T00:00:00Z",
            "longFundingInterval": 8,
            "shortFundingInterval": 8
        });
        if let (Some(value), Some(object)) = (execution_eligible, payload.as_object_mut()) {
            object.insert("executionEligible".to_owned(), Value::Bool(value));
        }
        payload
    }

    fn market_evidence(venue: &str) -> OpportunityLegMarketEvidence {
        OpportunityLegMarketEvidence {
            venue: venue.into(),
            symbol: "BTCUSDT".into(),
            price: Some(100.0),
            health: MarketDataHealth {
                quality: MarketDataQuality::Fresh,
                source: crate::market::MarketDataSourceKind::WsPush,
                freshness_ms: Some(10),
                retry_after_ms: None,
                last_error: None,
                observed_at_ms: TEST_NOW_MS,
                coverage: Some(crate::market::MarketDataCoverage::new(1, 1)),
                problem: None,
            },
        }
    }

    fn empty_market_envelope<T>() -> MarketDataEnvelope<Option<T>> {
        MarketDataEnvelope {
            data: None,
            health: MarketDataHealth {
                quality: MarketDataQuality::Missing,
                source: crate::market::MarketDataSourceKind::LocalCache,
                freshness_ms: None,
                retry_after_ms: None,
                last_error: Some("missing".into()),
                observed_at_ms: 1,
                coverage: Some(crate::market::MarketDataCoverage::new(1, 0)),
                problem: None,
            },
            retry_after_ms: None,
            row_cap: None,
            row_evidence: Vec::new(),
            fanout: Vec::new(),
        }
    }

    fn verified_cost() -> ExecutionCostProfile {
        ExecutionCostProfile {
            gross_edge_bps: 20.0,
            fee_bps: 10.0,
            wear_bps: 2.0,
            total_cost_bps: 12.0,
            one_cycle: OneCycleCostProfile {
                net_bps: 8.0,
                covers_round_trip_cost: true,
                ..OneCycleCostProfile::default()
            },
            breakeven_periods: 1,
            breakeven_hours: 8.0,
            recommended_hold_periods: 2,
            recommended_hold_hours: 16.0,
            net_bps_at_recommended_hold: 28.0,
            round_trip: Some(RoundTripCostBreakdown {
                long_leg: leg_cost(crate::hedge::HedgeLegRole::Long, "binance"),
                short_leg: leg_cost(crate::hedge::HedgeLegRole::Short, "kucoin"),
                open_fee_bps: 10.0,
                close_fee_bps: 10.0,
                open_slippage_bps: 1.0,
                close_slippage_bps: 1.0,
                borrow_or_financing_bps: 0.0,
                funding_window_mismatch_buffer_bps: 0.0,
                min_profit_buffer_bps: 0.0,
                total_cost_bps: 22.0,
                one_cycle_net_bps: 8.0,
                profitability_evidence: Default::default(),
            }),
        }
    }

    fn leg_cost(role: crate::hedge::HedgeLegRole, venue: &str) -> crate::fees::LegCostBreakdown {
        let snapshot = fee_snapshot(venue);
        crate::fees::LegCostBreakdown {
            role,
            venue: venue.into(),
            symbol: "BTCUSDT".into(),
            product: crate::fees::FeeProduct::Perp,
            open_fee_bps: 5.0,
            close_fee_bps: 5.0,
            open_slippage_bps: 0.5,
            close_slippage_bps: 0.5,
            fee_snapshot: Some(snapshot),
        }
    }

    fn fee_snapshot(venue: &str) -> crate::fees::TradeFeeSnapshot {
        crate::fees::TradeFeeSnapshot {
            venue: venue.into(),
            symbol: "BTCUSDT".into(),
            product: crate::fees::FeeProduct::Perp,
            account_id: None,
            maker_fee_bps: 2.0,
            taker_fee_bps: 5.0,
            open_fee_bps: 5.0,
            close_fee_bps: 5.0,
            source: crate::fees::TradeFeeSource::OfficialSchedule,
            fetched_at_ms: 1_780_185_600_000,
            valid_until_ms: 1_780_272_000_000,
            freshness_ms: Some(1_000),
            evidence: Some(crate::fees::TradeFeeEvidence {
                evidence_id: format!("fee:{venue}:perp:vip0"),
                source_name: format!("{venue} unit test fee fixture"),
                source_url: "https://example.com/fee-fixture".into(),
                checked_at_ms: 1_780_185_600_000,
                effective_at_ms: None,
                schedule_version: Some("2026-05-31".into()),
                tier: Some("VIP0".into()),
                scope: Some("perp taker".into()),
                problem: None,
            }),
            verification_problem: None,
            note: None,
        }
    }
}
