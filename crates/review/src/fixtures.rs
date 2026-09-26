use crate::executed::normalize_net_pnl;
use shared_types::{
    ExecutedTrade, MissReason, MissedOpportunity, ReviewPnlEvidence, StrategyKind,
    StrategyPerformance, StrategyPerformanceSampleStatus,
};

pub fn sample_executed(now_ms: i64) -> Vec<ExecutedTrade> {
    vec![normalize_net_pnl(
        TradeSeed {
            id: "exec-btc-001",
            strategy: StrategyKind::PerpCross,
            symbol: "BTC",
            long_venue: "OKX",
            short_venue: "Hyperliquid",
            opened_at_ms: now_ms - 9 * 60 * 60_000,
            closed_at_ms: Some(now_ms - 42 * 60_000),
            holding_minutes: Some(498),
            gross_pnl_usd: 2_180.0,
            fee_usd: 126.0,
            funding_usd: 410.0,
            slippage_usd: 74.0,
        }
        .into_trade(),
    )]
}

pub fn sample_missed(now_ms: i64) -> Vec<MissedOpportunity> {
    vec![MissedOpportunity {
        id: "missed-btc-001".into(),
        opportunity_id: "btc-perp-cross".into(),
        strategy: StrategyKind::PerpCross,
        symbol: "BTC".into(),
        detected_at_ms: now_ms - 36 * 60_000,
        expected_pnl_usd: 570.0,
        reason: MissReason::ManualSkip,
        detail: "Dry-run 阶段手动跳过，保留归因样例。".into(),
    }]
}

pub fn sample_strategy_performance() -> Vec<StrategyPerformance> {
    vec![
        perf(StrategyKind::PerpCross, 42, 68.0, 184.0, 7_728.0),
        perf(StrategyKind::SpotPerp, 18, 61.0, 126.0, 2_268.0),
        perf(StrategyKind::CrossSpotPerp, 9, 72.0, 203.0, 1_827.0),
    ]
}

struct TradeSeed {
    id: &'static str,
    strategy: StrategyKind,
    symbol: &'static str,
    long_venue: &'static str,
    short_venue: &'static str,
    opened_at_ms: i64,
    closed_at_ms: Option<i64>,
    holding_minutes: Option<u32>,
    gross_pnl_usd: f64,
    fee_usd: f64,
    funding_usd: f64,
    slippage_usd: f64,
}

impl TradeSeed {
    fn into_trade(self) -> ExecutedTrade {
        ExecutedTrade {
            id: self.id.into(),
            strategy: self.strategy,
            symbol: self.symbol.into(),
            long_venue: self.long_venue.into(),
            short_venue: self.short_venue.into(),
            opened_at_ms: self.opened_at_ms,
            closed_at_ms: self.closed_at_ms,
            holding_minutes: self.holding_minutes,
            gross_pnl_usd: self.gross_pnl_usd,
            fee_usd: self.fee_usd,
            funding_usd: self.funding_usd,
            slippage_usd: self.slippage_usd,
            net_pnl_usd: 0.0,
            evidence: ReviewPnlEvidence::default(),
            actual_fields: Vec::new(),
            estimated_fields: Vec::new(),
            missing_fields: Vec::new(),
            long_orders: Vec::new(),
            short_orders: Vec::new(),
        }
    }
}

fn perf(
    kind: StrategyKind,
    trades_30d: u32,
    hit_rate_pct: f64,
    avg_pnl_per_trade_usd: f64,
    net_pnl_30d_usd: f64,
) -> StrategyPerformance {
    StrategyPerformance {
        execution_environment: Some(shared_types::ExecutionEnvironment::Paper),
        kind,
        sample_window_days: 30,
        total_trades_30d: trades_30d,
        trades_30d,
        actual_trades_30d: trades_30d,
        estimated_trades_30d: 0,
        skipped_trades_30d: 0,
        partial_evidence_trades_30d: 0,
        sample_status: StrategyPerformanceSampleStatus::Complete,
        lowest_fill_confidence: None,
        lowest_fill_confidence_score: None,
        profitable_trades_30d: trades_30d,
        losing_trades_30d: 0,
        break_even_trades_30d: 0,
        independent_periods_30d: 3,
        data_missing_rate_pct: 0.0,
        hit_rate_pct,
        avg_pnl_per_trade_usd,
        sharpe_30d: 2.1,
        sortino_30d: 3.4,
        max_drawdown_pct: 1.8,
        max_drawdown_usd: net_pnl_30d_usd * 0.018,
        gross_pnl_30d_usd: net_pnl_30d_usd * 1.18,
        gross_profit_30d_usd: net_pnl_30d_usd * 1.18,
        gross_loss_30d_usd: net_pnl_30d_usd * 0.18,
        profit_factor: Some(1.18 / 0.18),
        tail_loss_p95_usd: Some(avg_pnl_per_trade_usd * 0.5),
        worst_trade_pnl_usd: Some(-avg_pnl_per_trade_usd * 0.75),
        finality_latency_p50_ms: Some(180),
        finality_latency_p95_ms: Some(480),
        finality_latency_max_ms: Some(900),
        trade_order_error_rate_pct: Some(0.0),
        actual_net_pnl_30d_usd: net_pnl_30d_usd,
        estimated_net_pnl_30d_usd: 0.0,
        net_pnl_30d_usd,
        avg_holding_hours: 7.6,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_executed_has_component_net_pnl() {
        let rows = sample_executed(100_000);

        assert_eq!(rows[0].net_pnl_usd, 2_464.0);
    }
}
