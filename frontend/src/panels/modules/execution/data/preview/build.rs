use leptos::prelude::*;
use shared_types::{ApiProblem, HedgeDepthStatus, HedgeExecutionParams, OrderType};

use super::super::super::problem::execution_problem_text;
use super::super::super::selection::ExecutionSelection;
use super::format::{
    money, parse_margin_mode, parse_number, parse_order_type, parse_time_in_force,
};
use super::model::*;

const SAFE_PREVIEW_NOTIONAL_USD: f64 = 750.0;

pub(super) fn failed_for_inputs(signals: PreviewSignals, problem: &ApiProblem) -> ExecutionPreview {
    let query = preview_query(signals);
    failed_preview(&query.seed, &query.input, problem)
}

pub(super) fn failed_preview(
    seed: &PreviewSeed,
    input: &PreviewInput,
    problem: &ApiProblem,
) -> ExecutionPreview {
    let mut preview = pending_preview(seed, input);
    preview.readiness = PreviewReadiness::Error;
    preview.source = "预检错误";
    let problem_text = execution_problem_text("Preview 请求失败", problem);
    preview.risk.note.clone_from(&problem_text);
    preview.risk.blockers.insert(0, problem_text);
    preview
}

pub(super) fn stale_preview(
    mut preview: ExecutionPreview,
    problem: &ApiProblem,
) -> ExecutionPreview {
    preview.idempotency_key = None;
    preview.readiness = PreviewReadiness::Stale;
    preview.source = "预检失效";
    preview.long_allowed = false;
    preview.short_allowed = false;
    let problem_text = execution_problem_text("上次预览已失效", problem);
    preview.risk.note.clone_from(&problem_text);
    preview.risk.blockers.insert(0, problem_text);
    preview
}

pub(super) fn pending_preview(seed: &PreviewSeed, input: &PreviewInput) -> ExecutionPreview {
    ExecutionPreview {
        opportunity_id: seed.opportunity_id.clone(),
        opportunity_snapshot_id: seed.opportunity_snapshot_id.clone(),
        idempotency_key: None,
        ticket_id: None,
        expires_at_ms: None,
        readiness: PreviewReadiness::Pending,
        source: "等待预检",
        estimated_funding_usd: 0.0,
        open_cost_usd: 0.0,
        close_cost_usd: 0.0,
        slippage_cost_usd: 0.0,
        one_cycle_cost: None,
        max_loss_usd: 0.0,
        used_capital_usd: 0.0,
        liquidation: PreviewLiquidation {
            current_account_pct: None,
            after_hedge_pct: None,
            positions_evidence: None,
        },
        execution_mode_label: "等待",
        long_allowed: false,
        short_allowed: false,
        long_notional_usd: input.long_notional_usd,
        short_notional_usd: input.short_notional_usd,
        long_reference_price: input.long_price,
        short_reference_price: input.short_price,
        long_market_evidence: seed.long_market_evidence.clone(),
        short_market_evidence: seed.short_market_evidence.clone(),
        depth: PreviewDepth {
            long_5bps: None,
            long_10bps: None,
            long_20bps: None,
            short_5bps: None,
            short_10bps: None,
            short_20bps: None,
            executable_status: HedgeDepthStatus::Unknown,
            executable_amount_usd: None,
            executable_reason: None,
            long_reason: None,
            short_reason: None,
            long_depth_health: None,
            short_depth_health: None,
        },
        fee_evidence: Vec::new(),
        profit_evidence: seed.profit_evidence.clone(),
        order_plans: Vec::new(),
        identity_evidence_required: false,
        risk: PreviewRisk {
            note: if !seed.has_opportunity() {
                "请先从机会扫描或期货套利选择一条机会。".into()
            } else {
                "等待预检完成，提交保持禁用。".into()
            },
            guards: Vec::new(),
            blockers: seed.execution_blockers.clone(),
        },
    }
}

pub(super) fn preview_input(seed: &PreviewSeed, signals: PreviewSignals) -> PreviewInput {
    let capital_usd =
        parse_number(&signals.capital_usd.get()).unwrap_or_else(|| default_capital_usd(seed));
    let leverage = parse_number(&signals.leverage.get())
        .unwrap_or_else(|| default_leverage(seed))
        .clamp(0.1, 20.0);
    let default_notional = capital_usd * leverage;
    PreviewInput {
        capital_usd,
        leverage,
        order_type: parse_order_type(&signals.order_type.get()),
        limit_offset_bps: parse_number(&signals.limit_offset_bps.get())
            .unwrap_or_default()
            .clamp(-500.0, 500.0),
        long_price: parse_number(&signals.long_price.get()),
        short_price: parse_number(&signals.short_price.get()),
        long_notional_usd: parse_number(&signals.long_notional_usd.get())
            .unwrap_or(default_notional)
            .max(1.0),
        short_notional_usd: parse_number(&signals.short_notional_usd.get())
            .unwrap_or(default_notional)
            .max(1.0),
        execution_params: execution_params(seed, signals),
    }
}

pub(crate) fn default_capital_text(selection: &ExecutionSelection) -> String {
    let capital =
        capped_default_capital_usd(selection.default_capital_usd, selection.default_leverage);
    format!("{:.0}", capital.floor())
}

pub(crate) fn default_leverage_text(selection: &ExecutionSelection) -> String {
    format!("{:.1}", selection.default_leverage)
}

pub(crate) fn default_limit_offset_text(selection: &ExecutionSelection) -> String {
    format!("{:.1}", selection.default_limit_offset_bps)
}

pub(crate) fn quantity_from_notional_text(pair: &str, price: &str, notional: &str) -> String {
    let Some(notional) = parse_number(notional).filter(|value| *value > f64::EPSILON) else {
        return "名义缺证据".to_owned();
    };
    let Some(price) = parse_number(price).filter(|value| *value > f64::EPSILON) else {
        return format!("{} 名义", money(notional));
    };
    let qty = notional / price;
    format!("{qty:.4} {pair}")
}

pub(super) fn pending_for_inputs(signals: PreviewSignals) -> ExecutionPreview {
    let query = preview_query(signals);
    pending_preview(&query.seed, &query.input)
}

pub(super) fn preview_query(signals: PreviewSignals) -> PreviewQuery {
    let selection = signals.selection.get();
    let seed = PreviewSeed::from_selection(&selection);
    let input = preview_input(&seed, signals);
    PreviewQuery { seed, input }
}

fn execution_params(seed: &PreviewSeed, signals: PreviewSignals) -> HedgeExecutionParams {
    let order_type = parse_order_type(&signals.order_type.get());
    HedgeExecutionParams {
        capital_usd: parse_number(&signals.capital_usd.get())
            .unwrap_or_else(|| default_capital_usd(seed)),
        leverage: parse_number(&signals.leverage.get())
            .unwrap_or_else(|| default_leverage(seed))
            .clamp(0.1, 20.0),
        order_type,
        market_order_style: None,
        margin_mode: parse_margin_mode(&signals.margin_mode.get()),
        time_in_force: parse_time_in_force(&signals.time_in_force.get()),
        post_only: matches!(order_type, OrderType::PostOnly),
        limit_offset_bps: parse_number(&signals.limit_offset_bps.get())
            .unwrap_or_default()
            .clamp(-500.0, 500.0),
    }
}

fn default_capital_usd(seed: &PreviewSeed) -> f64 {
    capped_default_capital_usd(seed.default_capital_usd, seed.default_leverage)
}

fn default_leverage(seed: &PreviewSeed) -> f64 {
    seed.default_leverage
}

fn capped_default_capital_usd(default_capital_usd: f64, default_leverage: f64) -> f64 {
    let safe_capital = SAFE_PREVIEW_NOTIONAL_USD / default_leverage.max(0.1);
    default_capital_usd.min(safe_capital).max(1.0)
}
