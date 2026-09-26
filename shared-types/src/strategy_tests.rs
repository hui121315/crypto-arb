use super::{
    is_diagnostic_only_strategy, is_live_executable_strategy, is_p0_executable_strategy,
    is_phase2_strategy, StrategyCategory, StrategyExposure, StrategyKind, StrategyKindInfo,
    StrategyPerformance, StrategyPerformanceSampleStatus, LIVE_EXECUTABLE_STRATEGY_KINDS,
    P0_EXECUTABLE_STRATEGY_KINDS,
};
use crate::ExecutionFillConfidence;
use serde_json::json;

#[test]
fn phase2_strategy_labels_and_categories_are_stable() {
    let cases = [
        (
            StrategyKind::PerpCross,
            "永续跨所",
            StrategyCategory::Futures,
        ),
        (
            StrategyKind::PerpPriceSpread,
            "永续价差",
            StrategyCategory::Futures,
        ),
        (
            StrategyKind::CrossSpotPerp,
            "跨所期现",
            StrategyCategory::Futures,
        ),
        (StrategyKind::SpotCross, "现货跨所", StrategyCategory::Spot),
        (
            StrategyKind::FundingCarry,
            "资金费 Carry",
            StrategyCategory::FundingYield,
        ),
        (
            StrategyKind::OptionsPerpBasis,
            "期权-永续基差",
            StrategyCategory::Options,
        ),
    ];

    for (kind, zh, category) in cases {
        assert_eq!(kind.label_zh(), zh);
        assert_eq!(kind.category(), category);
        assert!(!kind.label_en().is_empty());
        assert!(!kind.description().is_empty());
    }
}

#[test]
fn strategy_info_frontend_exposure_is_p0_only() {
    let p0 = StrategyKindInfo::from_kind(StrategyKind::PerpCross, true);

    assert!(p0.frontend_enabled);
    assert_eq!(p0.exposure, StrategyExposure::MainP0);
    assert!(p0.engine_present);
    assert!(p0.data_contract_ready);
    assert!(p0.execution_supported);
    assert!(p0.live_execution_supported);
    assert!(p0.is_main_p0_executable());
    assert!(p0.is_main_p0_live_executable());
}

#[test]
fn strategy_info_non_p0_execution_is_closed() {
    let non_p0 = StrategyKindInfo::from_kind(StrategyKind::Triangular, true);

    assert!(!non_p0.frontend_enabled);
    assert_eq!(non_p0.exposure, StrategyExposure::Diagnostic);
    assert!(!non_p0.engine_present);
    assert!(!non_p0.data_contract_ready);
    assert!(!non_p0.execution_supported);
    assert!(!non_p0.live_execution_supported);
    assert!(!non_p0.is_main_p0_executable());
    assert!(!non_p0.is_main_p0_live_executable());
}

#[test]
fn strategy_exposure_separates_main_diagnostic_and_hidden() {
    assert_eq!(
        StrategyExposure::for_kind(StrategyKind::SpotPerp),
        StrategyExposure::MainP0
    );
    assert_eq!(
        StrategyExposure::for_kind(StrategyKind::Triangular),
        StrategyExposure::Diagnostic
    );
    assert_eq!(
        StrategyExposure::for_kind(StrategyKind::OnchainDepeg),
        StrategyExposure::Hidden
    );
    assert!(StrategyExposure::MainP0.is_main_p0());
    assert!(!StrategyExposure::Diagnostic.is_main_p0());
    assert!(!StrategyExposure::Hidden.is_main_p0());
}

#[test]
fn options_perp_basis_is_diagnostic_only() {
    let info = StrategyKindInfo::from_kind(StrategyKind::OptionsPerpBasis, true);

    assert!(is_diagnostic_only_strategy(StrategyKind::OptionsPerpBasis));
    assert_eq!(info.exposure, StrategyExposure::Diagnostic);
    assert!(!info.frontend_enabled);
    assert!(!info.engine_present);
    assert!(!info.data_contract_ready);
    assert!(!info.execution_supported);
    assert!(!info.live_execution_supported);
    assert!(!info.is_main_p0_executable());
    assert!(!info.is_main_p0_live_executable());
}

#[test]
fn p0_query_values_are_stable() {
    let values: Vec<_> = P0_EXECUTABLE_STRATEGY_KINDS
        .into_iter()
        .map(StrategyKind::as_query_value)
        .collect();

    assert_eq!(
        values,
        [
            "perp_cross",
            "perp_price_spread",
            "spot_perp",
            "cross_spot_perp",
            "spot_cross"
        ]
    );
    assert!(values
        .iter()
        .all(|value| StrategyKind::from_query_value(value).is_some()));
}

#[test]
fn p0_predicate_tracks_allowlist_and_defaults_non_p0_closed() {
    const ALL_KINDS: [StrategyKind; 11] = [
        StrategyKind::PerpCross,
        StrategyKind::PerpPriceSpread,
        StrategyKind::SpotPerp,
        StrategyKind::CrossSpotPerp,
        StrategyKind::SpotCross,
        StrategyKind::Triangular,
        StrategyKind::FundingCarry,
        StrategyKind::CashAndCarry,
        StrategyKind::OptionsPerpBasis,
        StrategyKind::QuarterlyPerp,
        StrategyKind::OnchainDepeg,
    ];

    for kind in ALL_KINDS {
        assert_eq!(
            is_p0_executable_strategy(kind),
            P0_EXECUTABLE_STRATEGY_KINDS.contains(&kind),
            "predicate must equal allowlist membership for {kind:?}"
        );
        if is_p0_executable_strategy(kind) {
            assert!(
                is_phase2_strategy(kind),
                "P0 kind {kind:?} must be phase2-listed"
            );
        }
    }

    for kind in ALL_KINDS {
        if !P0_EXECUTABLE_STRATEGY_KINDS.contains(&kind) {
            assert!(
                !is_p0_executable_strategy(kind),
                "non-P0 kind {kind:?} must default to non-executable"
            );
        }
    }
}

#[test]
fn live_execution_is_limited_to_cash_flow_proven_strategies() {
    assert_eq!(LIVE_EXECUTABLE_STRATEGY_KINDS, [StrategyKind::PerpCross]);
    for kind in P0_EXECUTABLE_STRATEGY_KINDS {
        let info = StrategyKindInfo::from_kind(kind, true);
        assert_eq!(
            info.live_execution_supported,
            is_live_executable_strategy(kind)
        );
        assert_eq!(
            info.is_main_p0_live_executable(),
            LIVE_EXECUTABLE_STRATEGY_KINDS.contains(&kind)
        );
    }
}

#[test]
fn strategy_performance_carries_sample_window_and_confidence() {
    let value = serde_json::to_value(StrategyPerformance {
        execution_environment: None,
        kind: StrategyKind::PerpCross,
        sample_window_days: 30,
        total_trades_30d: 3,
        trades_30d: 2,
        actual_trades_30d: 1,
        estimated_trades_30d: 1,
        skipped_trades_30d: 1,
        partial_evidence_trades_30d: 1,
        sample_status: StrategyPerformanceSampleStatus::PartialEvidence,
        lowest_fill_confidence: Some(ExecutionFillConfidence::AdapterAck),
        lowest_fill_confidence_score: Some(0.65),
        profitable_trades_30d: 1,
        losing_trades_30d: 0,
        break_even_trades_30d: 0,
        independent_periods_30d: 1,
        data_missing_rate_pct: 100.0 / 3.0,
        hit_rate_pct: 100.0,
        avg_pnl_per_trade_usd: 18.0,
        sharpe_30d: 0.0,
        sortino_30d: 0.0,
        max_drawdown_pct: 0.0,
        max_drawdown_usd: 0.0,
        gross_pnl_30d_usd: 20.0,
        gross_profit_30d_usd: 18.0,
        gross_loss_30d_usd: 0.0,
        profit_factor: None,
        tail_loss_p95_usd: None,
        worst_trade_pnl_usd: Some(18.0),
        finality_latency_p50_ms: Some(120),
        finality_latency_p95_ms: Some(480),
        finality_latency_max_ms: Some(700),
        trade_order_error_rate_pct: Some(0.0),
        actual_net_pnl_30d_usd: 18.0,
        estimated_net_pnl_30d_usd: 6.0,
        net_pnl_30d_usd: 18.0,
        avg_holding_hours: 3.0,
    })
    .expect("serialize strategy performance");

    assert_sample_counts(&value);
    assert_sample_evidence(&value);
}

fn assert_sample_counts(value: &serde_json::Value) {
    assert_eq!(value["sampleWindowDays"], json!(30));
    assert_eq!(value["totalTrades30d"], json!(3));
    assert_eq!(value["trades30d"], json!(2));
    assert_eq!(value["actualTrades30d"], json!(1));
    assert_eq!(value["estimatedTrades30d"], json!(1));
    assert_eq!(value["skippedTrades30d"], json!(1));
    assert_eq!(value["partialEvidenceTrades30d"], json!(1));
    assert_eq!(value["sampleStatus"], json!("partial_evidence"));
}

fn assert_sample_evidence(value: &serde_json::Value) {
    assert_eq!(value["lowestFillConfidence"], json!("adapter_ack"));
    assert_eq!(value["lowestFillConfidenceScore"], json!(0.65));
    assert!(value.get("profitFactor").is_none());
    assert_eq!(value["independentPeriods30d"], json!(1));
    assert_eq!(value["netPnl30dUsd"], json!(18.0));
    assert_eq!(value["estimatedNetPnl30dUsd"], json!(6.0));
    assert_eq!(value["finalityLatencyP95Ms"], json!(480));
    assert_eq!(value["actualNetPnl30dUsd"], json!(18.0));
    assert_eq!(value["estimatedNetPnl30dUsd"], json!(6.0));
}
