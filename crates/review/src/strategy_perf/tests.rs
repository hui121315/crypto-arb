use super::*;
use shared_types::{ReviewPnlEvidence, ReviewPnlField};

#[test]
fn computes_core_strategy_performance() {
    let trades = vec![
        trade(StrategyKind::PerpCross, 100.0),
        trade(StrategyKind::PerpCross, -50.0),
        trade(StrategyKind::SpotPerp, 20.0),
    ];

    let perf = compute_performance(&trades, StrategyKind::PerpCross);

    assert_core_counts(&perf);
    assert_core_risk(&perf);
}

fn assert_core_counts(perf: &StrategyPerformance) {
    assert_eq!(perf.trades_30d, 2);
    assert_eq!(perf.total_trades_30d, 2);
    assert_eq!(perf.actual_trades_30d, 2);
    assert_eq!(perf.estimated_trades_30d, 0);
    assert_eq!(perf.sample_window_days, 30);
    assert_eq!(
        perf.sample_status,
        StrategyPerformanceSampleStatus::Complete
    );
    assert_eq!(perf.hit_rate_pct, 50.0);
    assert_eq!(perf.net_pnl_30d_usd, 50.0);
    assert_eq!(perf.actual_net_pnl_30d_usd, 50.0);
    assert_eq!(perf.estimated_net_pnl_30d_usd, 0.0);
}

fn assert_core_risk(perf: &StrategyPerformance) {
    assert_eq!(perf.avg_holding_hours, 2.0);
    assert_eq!(perf.profitable_trades_30d, 1);
    assert_eq!(perf.losing_trades_30d, 1);
    assert_eq!(perf.profit_factor, Some(2.0));
    assert_eq!(perf.max_drawdown_usd, 50.0);
    assert_eq!(perf.tail_loss_p95_usd, Some(50.0));
    assert_eq!(perf.independent_periods_30d, 1);
}

#[test]
fn skips_trades_with_missing_net_evidence() {
    let mut missing = trade(StrategyKind::PerpCross, 100.0);
    missing.missing_fields = vec![ReviewPnlField::Net];
    let trades = vec![missing, trade(StrategyKind::PerpCross, 50.0)];

    let perf = compute_performance(&trades, StrategyKind::PerpCross);

    assert_eq!(perf.trades_30d, 1);
    assert_eq!(perf.total_trades_30d, 2);
    assert_eq!(perf.skipped_trades_30d, 1);
    assert_eq!(perf.data_missing_rate_pct, 50.0);
    assert_eq!(
        perf.sample_status,
        StrategyPerformanceSampleStatus::PartialEvidence
    );
    assert_eq!(perf.net_pnl_30d_usd, 50.0);
}

#[test]
fn exposes_partial_sample_and_lowest_fill_confidence() {
    let mut estimated = trade(StrategyKind::PerpCross, 100.0);
    estimated.actual_fields.clear();
    estimated.estimated_fields = vec![ReviewPnlField::Net, ReviewPnlField::Fee];
    estimated
        .evidence
        .record_fill_confidence(ExecutionFillConfidence::VenueFill);
    let mut ack_backed = trade(StrategyKind::PerpCross, 50.0);
    ack_backed
        .evidence
        .record_fill_confidence(ExecutionFillConfidence::AdapterAck);

    let perf = compute_performance(&[estimated, ack_backed], StrategyKind::PerpCross);

    assert_eq!(perf.trades_30d, 2);
    assert_eq!(perf.actual_trades_30d, 1);
    assert_eq!(perf.estimated_trades_30d, 1);
    assert_eq!(perf.actual_net_pnl_30d_usd, 50.0);
    assert_eq!(perf.estimated_net_pnl_30d_usd, 100.0);
    assert_eq!(perf.net_pnl_30d_usd, 50.0);
    assert_eq!(perf.hit_rate_pct, 100.0);
    assert_eq!(perf.avg_pnl_per_trade_usd, 50.0);
    assert_eq!(perf.partial_evidence_trades_30d, 1);
    assert_eq!(
        perf.sample_status,
        StrategyPerformanceSampleStatus::PartialEvidence
    );
    assert_eq!(
        perf.lowest_fill_confidence,
        Some(ExecutionFillConfidence::AdapterAck)
    );
    assert_eq!(perf.lowest_fill_confidence_score, Some(0.65));
}

#[test]
fn estimated_only_samples_do_not_enter_real_performance_metrics() {
    let mut estimated = trade(StrategyKind::PerpCross, 100.0);
    estimated.actual_fields.clear();
    estimated.estimated_fields = vec![ReviewPnlField::Net];

    let perf = compute_performance(&[estimated], StrategyKind::PerpCross);

    assert_eq!(perf.trades_30d, 1);
    assert_eq!(perf.actual_trades_30d, 0);
    assert_eq!(perf.estimated_trades_30d, 1);
    assert_eq!(perf.net_pnl_30d_usd, 0.0);
    assert_eq!(perf.actual_net_pnl_30d_usd, 0.0);
    assert_eq!(perf.estimated_net_pnl_30d_usd, 100.0);
    assert_eq!(perf.profitable_trades_30d, 0);
    assert_eq!(perf.losing_trades_30d, 0);
    assert_eq!(perf.hit_rate_pct, 0.0);
    assert_eq!(perf.avg_pnl_per_trade_usd, 0.0);
    assert_eq!(perf.max_drawdown_usd, 0.0);
    assert_eq!(
        perf.sample_status,
        StrategyPerformanceSampleStatus::NoCompleteSample
    );
}

#[test]
fn independent_count_uses_closed_run_identity_instead_of_time_buckets() {
    let mut first = trade(StrategyKind::PerpCross, 10.0);
    first.id = "run-a".into();
    let mut second = trade(StrategyKind::PerpCross, 20.0);
    second.id = "run-b".into();
    second.opened_at_ms = first.opened_at_ms;

    let distinct = compute_performance(&[first.clone(), second], StrategyKind::PerpCross);
    assert_eq!(distinct.independent_periods_30d, 2);

    first.net_pnl_usd = 30.0;
    let duplicate = compute_performance(&[first.clone(), first], StrategyKind::PerpCross);
    assert_eq!(duplicate.independent_periods_30d, 1);
}

#[test]
fn percentile_and_drawdown_include_initial_losses() {
    let pnl = [-5.0, 10.0, -3.0, -20.0];

    assert_eq!(max_drawdown_usd(&pnl), 23.0);
    assert_eq!(loss_tail(&pnl).p95, Some(20.0));
    assert_eq!(percentile_u64(&[10, 20, 30, 40], 0.50), Some(20));
    assert_eq!(percentile_u64(&[10, 20, 30, 40], 0.95), Some(40));
}

fn trade(kind: StrategyKind, net_pnl_usd: f64) -> ExecutedTrade {
    ExecutedTrade {
        id: "t".into(),
        strategy: kind,
        symbol: "BTC".into(),
        long_venue: "OKX".into(),
        short_venue: "HL".into(),
        opened_at_ms: 0,
        closed_at_ms: Some(1),
        holding_minutes: Some(120),
        gross_pnl_usd: net_pnl_usd,
        fee_usd: 0.0,
        funding_usd: 0.0,
        slippage_usd: 0.0,
        net_pnl_usd,
        evidence: ReviewPnlEvidence::default(),
        actual_fields: vec![ReviewPnlField::Net],
        estimated_fields: Vec::new(),
        missing_fields: Vec::new(),
        long_orders: Vec::new(),
        short_orders: Vec::new(),
    }
}
