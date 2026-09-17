//! 机会列表视图模型结构、快照内 memo 缓存与从 `OpportunityListRow` 的构造（含成本视图）。
//! 派生/格式化助手见 `format.rs`，展示文案方法见 `labels.rs`，测试夹具见 `testing.rs`。

use crate::panels::modules::leg_label::leg_label;
use crate::panels::modules::market_evidence::leg_evidence_label;
use crate::panels::modules::opportunity_format::{price, risk_label, strategy_label};
use crate::panels::modules::rate_format::{signed_bps_percent, unsigned_bps_percent};
use shared_types::{
    is_p0_executable_strategy, opportunity_build_blockers_allow_preflight,
    OpportunityLegMarketEvidence, OpportunityListLegFunding, OpportunityListRow, RiskLevel,
    SpotLegMode, StrategyCategory, StrategyKind,
};
use std::sync::Arc;

use super::format::{execution_blockers, has_fresh_market_evidence, pct, source_label};

pub(crate) type OpportunityListViewRow = Arc<OpportunityListViewModel>;

#[derive(Clone, PartialEq)]
pub(crate) struct OpportunityListViewModel {
    pub id: String,
    pub snapshot_id: String,
    pub pair: String,
    pub strategy_label: String,
    pub strategy_kind: Option<StrategyKind>,
    pub strategy_category: Option<StrategyCategory>,
    pub spot_leg_mode: Option<SpotLegMode>,
    pub data_source: String,
    pub updated_at_ms: i64,
    pub long_venue: String,
    pub short_venue: String,
    pub long_leg: String,
    pub short_leg: String,
    pub long_price: String,
    pub short_price: String,
    pub long_market_evidence: Option<String>,
    pub short_market_evidence: Option<String>,
    pub long_market_evidence_raw: Option<OpportunityLegMarketEvidence>,
    pub short_market_evidence_raw: Option<OpportunityLegMarketEvidence>,
    pub long_funding: Option<OpportunityListLegFunding>,
    pub short_funding: Option<OpportunityListLegFunding>,
    pub net_edge: String,
    pub net_basis_bps: f64,
    pub predicted_funding_bps: Option<f64>,
    pub est_apr_pct: Option<f64>,
    pub gross_one_cycle_bps: f64,
    pub gross_one_cycle: String,
    pub round_trip_cost: String,
    pub cost_verified: bool,
    pub cost_total_bps: f64,
    pub cost_wear_bps: f64,
    pub fee_evidence_count: usize,
    pub fee_evidence_complete: bool,
    pub fee_evidence_ids: Vec<String>,
    pub one_cycle_penalty: f64,
    pub one_cycle_net: String,
    pub one_cycle_net_bps: f64,
    pub one_cycle_covers_cost: bool,
    pub breakeven_periods: u32,
    pub breakeven_hours: f64,
    pub recommended_hold_hours: f64,
    pub net_bps_at_recommended_hold: f64,
    pub risk: String,
    pub risk_level: RiskLevel,
    pub settlement_countdown_seconds: Option<i64>,
    pub time_to_settlement_ms: i64,
    pub optimal_position: f64,
    pub max_position: f64,
    pub execution_eligible: bool,
    pub execution_blockers: Vec<String>,
}

impl OpportunityListViewModel {
    pub(in crate::panels::modules::opportunity_view_model) fn from_row(
        row: OpportunityListRow,
        snapshot_id: &str,
    ) -> Self {
        let long_market_evidence_raw = row.long_leg.market_evidence.clone();
        let short_market_evidence_raw = row.short_leg.market_evidence.clone();
        let long_funding = row.long_leg.funding.clone();
        let short_funding = row.short_leg.funding.clone();
        let predicted_funding_bps =
            native_next_funding_bps(long_funding.as_ref(), short_funding.as_ref());
        let cost = CostView::from_row(&row);
        let p0_scope = row.strategy_kind.is_some_and(is_p0_executable_strategy);
        let build_blockers_allowed =
            opportunity_build_blockers_allow_preflight(row.strategy_kind, &row.execution.blockers);
        let market_evidence_ready = has_fresh_market_evidence(
            long_market_evidence_raw.as_ref(),
            short_market_evidence_raw.as_ref(),
        );
        let execution_blockers = execution_blockers(
            row.execution.blockers.clone(),
            p0_scope,
            market_evidence_ready,
            cost.verified,
        );
        Self {
            id: row.id,
            snapshot_id: snapshot_id.to_owned(),
            pair: row.symbol,
            strategy_label: strategy_label(row.strategy_kind),
            strategy_kind: row.strategy_kind,
            strategy_category: row.strategy_category,
            spot_leg_mode: row.spot_leg_mode,
            data_source: source_label(&row.data_source),
            updated_at_ms: row.updated_at.timestamp_millis(),
            long_venue: row.long_leg.venue.clone(),
            short_venue: row.short_leg.venue.clone(),
            long_leg: leg_label(&row.long_leg.venue, &row.long_leg.action),
            short_leg: leg_label(&row.short_leg.venue, &row.short_leg.action),
            long_price: price(row.long_leg.price),
            short_price: price(row.short_leg.price),
            long_market_evidence: leg_evidence_label(long_market_evidence_raw.as_ref()),
            short_market_evidence: leg_evidence_label(short_market_evidence_raw.as_ref()),
            long_market_evidence_raw,
            short_market_evidence_raw,
            long_funding,
            short_funding,
            net_edge: pct(row.metrics.net_single_yield * 100.0),
            net_basis_bps: row.metrics.net_single_yield * 10_000.0,
            predicted_funding_bps,
            est_apr_pct: row.metrics.annualized_funding_bps.map(|bps| bps / 100.0),
            gross_one_cycle_bps: cost.gross_one_cycle_bps,
            gross_one_cycle: cost.gross_one_cycle,
            round_trip_cost: cost.round_trip_cost,
            cost_verified: cost.verified,
            cost_total_bps: cost.total_bps,
            cost_wear_bps: cost.wear_bps,
            fee_evidence_count: cost.fee_evidence_count,
            fee_evidence_complete: cost.fee_evidence_complete,
            fee_evidence_ids: cost.fee_evidence_ids,
            one_cycle_penalty: cost.one_cycle_penalty,
            one_cycle_net: cost.one_cycle_net,
            one_cycle_net_bps: cost.one_cycle_net_bps,
            one_cycle_covers_cost: cost.one_cycle_covers_cost,
            breakeven_periods: cost.breakeven_periods,
            breakeven_hours: cost.breakeven_hours,
            recommended_hold_hours: cost.recommended_hold_hours,
            net_bps_at_recommended_hold: cost.net_bps_at_recommended_hold,
            risk: risk_label(row.metrics.risk_level).into(),
            risk_level: row.metrics.risk_level,
            settlement_countdown_seconds: row.metrics.settlement_countdown_seconds,
            time_to_settlement_ms: row.metrics.time_to_settlement_ms,
            optimal_position: row.execution.optimal_position,
            max_position: row.execution.max_position,
            execution_eligible: row.execution.eligible
                && p0_scope
                && build_blockers_allowed
                && market_evidence_ready
                && cost.verified,
            execution_blockers,
        }
    }
}

fn native_next_funding_bps(
    long: Option<&OpportunityListLegFunding>,
    short: Option<&OpportunityListLegFunding>,
) -> Option<f64> {
    if long.is_none() && short.is_none() {
        return None;
    }
    let long_rate = long.map_or(0.0, |funding| funding.rate);
    let short_rate = short.map_or(0.0, |funding| funding.rate);
    (long_rate.is_finite() && short_rate.is_finite()).then_some((short_rate - long_rate) * 10_000.0)
}

#[derive(Clone)]
struct CostView {
    verified: bool,
    gross_one_cycle_bps: f64,
    gross_one_cycle: String,
    round_trip_cost: String,
    total_bps: f64,
    wear_bps: f64,
    fee_evidence_count: usize,
    fee_evidence_complete: bool,
    fee_evidence_ids: Vec<String>,
    one_cycle_penalty: f64,
    one_cycle_net: String,
    one_cycle_net_bps: f64,
    one_cycle_covers_cost: bool,
    breakeven_periods: u32,
    breakeven_hours: f64,
    recommended_hold_hours: f64,
    net_bps_at_recommended_hold: f64,
}

impl CostView {
    fn from_row(row: &OpportunityListRow) -> Self {
        if let Some(one_cycle_net_bps) = row
            .cost
            .verified
            .then_some(row.cost.one_cycle_net_bps)
            .flatten()
        {
            return Self {
                verified: true,
                gross_one_cycle_bps: row.cost.gross_edge_bps,
                gross_one_cycle: signed_bps_percent(row.cost.gross_edge_bps),
                round_trip_cost: unsigned_bps_percent(row.cost.total_cost_bps),
                total_bps: row.cost.total_cost_bps,
                wear_bps: row.cost.wear_bps,
                fee_evidence_count: row.cost.fee_evidence_count,
                fee_evidence_complete: row.cost.fee_evidence_complete,
                fee_evidence_ids: row.cost.fee_evidence_ids.clone(),
                one_cycle_penalty: row.cost.one_cycle_penalty,
                one_cycle_net: signed_bps_percent(one_cycle_net_bps),
                one_cycle_net_bps,
                one_cycle_covers_cost: row.cost.one_cycle_covers_cost,
                breakeven_periods: row.cost.breakeven_periods,
                breakeven_hours: row.cost.breakeven_hours,
                recommended_hold_hours: row.cost.recommended_hold_hours,
                net_bps_at_recommended_hold: row.cost.net_bps_at_recommended_hold,
            };
        }
        Self {
            verified: false,
            gross_one_cycle_bps: 0.0,
            gross_one_cycle: "成本未验证".into(),
            round_trip_cost: "成本未验证".into(),
            total_bps: 0.0,
            wear_bps: 0.0,
            fee_evidence_count: row.cost.fee_evidence_count,
            fee_evidence_complete: row.cost.fee_evidence_complete,
            fee_evidence_ids: row.cost.fee_evidence_ids.clone(),
            one_cycle_penalty: row.cost.one_cycle_penalty,
            one_cycle_net: "单次未验证".into(),
            one_cycle_net_bps: 0.0,
            one_cycle_covers_cost: false,
            breakeven_periods: 0,
            breakeven_hours: 0.0,
            recommended_hold_hours: 0.0,
            net_bps_at_recommended_hold: 0.0,
        }
    }
}
