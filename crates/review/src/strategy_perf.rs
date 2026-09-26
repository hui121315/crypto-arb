use shared_types::{
    ExecutedTrade, ExecutionFillConfidence, LiveOrderState, StrategyKind, StrategyPerformance,
    StrategyPerformanceSampleStatus,
};
use std::collections::BTreeSet;

mod aggregate;

use aggregate::PerformanceAggregate;

const CRYPTO_TRADING_DAYS_PER_YEAR: f64 = 365.0;
const PERFORMANCE_WINDOW_DAYS: u32 = 30;

pub fn compute_performance(trades: &[ExecutedTrade], kind: StrategyKind) -> StrategyPerformance {
    let aggregate = PerformanceAggregate::from_trades(trades, kind);
    performance_from_aggregate(aggregate, kind, None)
}

pub fn compute_performance_by_environment(
    trades: &[ExecutedTrade],
    kind: StrategyKind,
) -> Vec<StrategyPerformance> {
    use shared_types::ExecutionEnvironment::{Live, Paper};
    [Some(Live), Some(Paper), None].into_iter().filter_map(|environment| {
        let aggregate = PerformanceAggregate::from_rows(trades.iter().filter(|trade| {
            trade.strategy == kind && trade.execution_environment() == environment
        }));
        (aggregate.total_trades_30d > 0)
            .then(|| performance_from_aggregate(aggregate, kind, environment))
    }).collect()
}

fn performance_from_aggregate(
    aggregate: PerformanceAggregate<'_>,
    kind: StrategyKind,
    execution_environment: Option<shared_types::ExecutionEnvironment>,
) -> StrategyPerformance {
    let mut ordered_rows = aggregate.rows.clone();
    ordered_rows.sort_by_key(|trade| trade.closed_at_ms.unwrap_or(trade.opened_at_ms));
    let pnl: Vec<f64> = ordered_rows.iter().map(|trade| trade.net_pnl_usd).collect();
    let loss_tail = loss_tail(&pnl);
    let finality = finality_latency(&aggregate.rows);
    let avg_holding_hours = avg_holding_hours(&aggregate.rows);

    StrategyPerformance {
        kind,
        execution_environment,
        sample_window_days: PERFORMANCE_WINDOW_DAYS,
        total_trades_30d: aggregate.total_trades_30d,
        trades_30d: aggregate.trades_30d,
        actual_trades_30d: aggregate.actual_trades_30d,
        estimated_trades_30d: aggregate.estimated_trades_30d,
        skipped_trades_30d: aggregate.skipped_trades_30d,
        partial_evidence_trades_30d: aggregate.partial_evidence_trades_30d,
        sample_status: sample_status(
            aggregate.total_trades_30d,
            aggregate.actual_trades_30d,
            aggregate.skipped_trades_30d,
            aggregate.partial_evidence_trades_30d,
        ),
        lowest_fill_confidence: aggregate.lowest_fill_confidence,
        lowest_fill_confidence_score: aggregate
            .lowest_fill_confidence
            .map(ExecutionFillConfidence::score),
        profitable_trades_30d: aggregate.profitable_trades_30d,
        losing_trades_30d: aggregate.losing_trades_30d,
        break_even_trades_30d: aggregate.break_even_trades_30d,
        independent_periods_30d: independent_closed_runs(&aggregate.rows),
        data_missing_rate_pct: pct(
            aggregate.skipped_trades_30d as f64,
            aggregate.total_trades_30d as f64,
        ),
        hit_rate_pct: pct(
            aggregate.profitable_trades_30d as f64,
            aggregate.actual_trades_30d as f64,
        ),
        avg_pnl_per_trade_usd: if aggregate.actual_trades_30d == 0 {
            0.0
        } else {
            aggregate.net / aggregate.actual_trades_30d as f64
        },
        sharpe_30d: sharpe_ratio(&pnl),
        sortino_30d: sortino_ratio(&pnl),
        max_drawdown_pct: max_drawdown_pct(&pnl),
        max_drawdown_usd: max_drawdown_usd(&pnl),
        gross_pnl_30d_usd: aggregate.gross,
        gross_profit_30d_usd: aggregate.gross_profit_30d_usd,
        gross_loss_30d_usd: aggregate.gross_loss_30d_usd,
        profit_factor: aggregate.profit_factor,
        tail_loss_p95_usd: loss_tail.p95,
        worst_trade_pnl_usd: pnl.iter().copied().min_by(f64::total_cmp),
        finality_latency_p50_ms: finality.p50,
        finality_latency_p95_ms: finality.p95,
        finality_latency_max_ms: finality.max,
        trade_order_error_rate_pct: trade_order_error_rate(&aggregate.rows),
        actual_net_pnl_30d_usd: aggregate.actual_net_pnl_30d_usd,
        estimated_net_pnl_30d_usd: aggregate.estimated_net_pnl_30d_usd,
        net_pnl_30d_usd: aggregate.net,
        avg_holding_hours,
    }
}

fn independent_closed_runs(rows: &[&ExecutedTrade]) -> u32 {
    rows.iter()
        .map(|trade| trade.id.as_str())
        .filter(|id| !id.is_empty())
        .collect::<BTreeSet<_>>()
        .len() as u32
}

#[derive(Debug, Clone, Copy, Default)]
struct LossTail {
    p95: Option<f64>,
}

fn loss_tail(pnl: &[f64]) -> LossTail {
    let mut losses = pnl
        .iter()
        .copied()
        .filter(|value| *value < -f64::EPSILON)
        .map(f64::abs)
        .collect::<Vec<_>>();
    losses.sort_by(f64::total_cmp);
    LossTail {
        p95: percentile_f64(&losses, 0.95),
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct FinalityLatency {
    p50: Option<u64>,
    p95: Option<u64>,
    max: Option<u64>,
}

fn finality_latency(rows: &[&ExecutedTrade]) -> FinalityLatency {
    let mut samples = rows
        .iter()
        .flat_map(|trade| trade.long_orders.iter().chain(&trade.short_orders))
        .filter(|order| order.state == LiveOrderState::Filled)
        .filter_map(|order| {
            order
                .updated_at_ms
                .checked_sub(order.intent.created_at_ms)
                .and_then(|latency| u64::try_from(latency).ok())
        })
        .collect::<Vec<_>>();
    samples.sort_unstable();
    FinalityLatency {
        p50: percentile_u64(&samples, 0.50),
        p95: percentile_u64(&samples, 0.95),
        max: samples.last().copied(),
    }
}

fn trade_order_error_rate(rows: &[&ExecutedTrade]) -> Option<f64> {
    let orders = rows
        .iter()
        .flat_map(|trade| trade.long_orders.iter().chain(&trade.short_orders))
        .collect::<Vec<_>>();
    if orders.is_empty() {
        return None;
    }
    let errors = orders
        .iter()
        .filter(|order| {
            matches!(
                order.state,
                LiveOrderState::Rejected | LiveOrderState::Failed | LiveOrderState::Unknown
            )
        })
        .count();
    Some(pct(errors as f64, orders.len() as f64))
}

fn percentile_f64(sorted: &[f64], percentile: f64) -> Option<f64> {
    percentile_index(sorted.len(), percentile).map(|index| sorted[index])
}

fn percentile_u64(sorted: &[u64], percentile: f64) -> Option<u64> {
    percentile_index(sorted.len(), percentile).map(|index| sorted[index])
}

fn percentile_index(len: usize, percentile: f64) -> Option<usize> {
    if len == 0 {
        return None;
    }
    Some(
        ((percentile.clamp(0.0, 1.0) * len as f64).ceil() as usize)
            .saturating_sub(1)
            .min(len - 1),
    )
}

fn sample_status(
    total_trades_30d: u32,
    actual_trades_30d: u32,
    skipped_trades_30d: u32,
    partial_evidence_trades_30d: u32,
) -> StrategyPerformanceSampleStatus {
    if total_trades_30d == 0 {
        StrategyPerformanceSampleStatus::NoTrades
    } else if actual_trades_30d == 0 {
        StrategyPerformanceSampleStatus::NoCompleteSample
    } else if skipped_trades_30d > 0 || partial_evidence_trades_30d > 0 {
        StrategyPerformanceSampleStatus::PartialEvidence
    } else {
        StrategyPerformanceSampleStatus::Complete
    }
}

mod statistics;

use statistics::*;

#[cfg(test)]
mod tests;
