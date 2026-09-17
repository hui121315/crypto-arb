use super::*;

#[test]
fn profit_lock_evidence_blocks_statistical_spreads() {
    let proof = arbitrage::profit_proof::evaluate_strategy_profit(
        arbitrage::profit_proof::StrategyProfitProofInput {
            strategy: Some(shared_types::StrategyKind::PerpPriceSpread),
            spot_leg_mode: None,
            funding: arbitrage::profit_proof::FundingWindowInput {
                long_funding_bps: None,
                short_funding_bps: None,
                long_next_settlement_ms: 0,
                short_next_settlement_ms: 0,
                long_interval_hours: 0,
                short_interval_hours: 0,
            },
            executable_price: arbitrage::profit_proof::ExecutablePriceInput::default(),
            gross_edge_bps: Some(50.0),
            total_cost_bps: Some(20.0),
            mismatch_buffer_bps: Some(0.0),
            target_buffer_bps: Some(5.0),
            observed_at_ms: 1,
        },
    );

    let row = profit_lock_evidence(&proof, 1);

    assert_eq!(row.key, "profit_lock");
    assert!(!row.passed);
    assert!(row.detail.contains("class=statistical"));
}
