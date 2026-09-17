use arbitrage::models::{RawOpportunity, RawOpportunityExtra};
use arbitrage::{CostBreakdown, OpportunityBuilder, PositionSizing, RiskMetrics};
use shared_types::{
    ArbitrageType, FundingRateData, OpportunityLegMarketEvidence, SpotLegMode, StrategyKind,
};

#[test]
fn dto_carries_spot_leg_mode_and_blocks_reverse_spot_mode() {
    let raw = RawOpportunity {
        symbol: "MU".into(),
        arb_type: ArbitrageType::SpotFutures,
        long_exchange: "okx".into(),
        short_exchange: "binance".into(),
        long_rate: funding("MU", "okx"),
        short_rate: funding("MU", "binance"),
        spread_8h: 0.000_2,
        single_yield: 0.000_2,
        extra: RawOpportunityExtra {
            strategy_kind: Some(StrategyKind::SpotPerp),
            spot_leg_mode: Some(SpotLegMode::BorrowAndSell),
            long_price: Some(100.0),
            short_price: Some(100.2),
            long_leg_market_evidence: Some(evidence("okx", "MU", 100.0)),
            short_leg_market_evidence: Some(evidence("binance", "MU", 100.2)),
            ..RawOpportunityExtra::default()
        },
    };

    let dto = OpportunityBuilder {
        raw: &raw,
        metrics: &RiskMetrics::default(),
        position: &PositionSizing::default(),
        cost: &CostBreakdown::default(),
        min_holding_periods: 1,
        net_single_yield: 0.000_2,
        data_source: "test",
        confidence: 0.9,
    }
    .build();

    assert_eq!(dto.spot_leg_mode, Some(SpotLegMode::BorrowAndSell));
    assert!(!dto.execution_eligible);
    assert!(dto
        .execution_blockers
        .iter()
        .any(|blocker| blocker.contains("现货库存或借币 API 证据")));
}

fn funding(symbol: &str, exchange: &str) -> FundingRateData {
    FundingRateData {
        symbol: symbol.into(),
        exchange: exchange.into(),
        rate: 0.000_1,
        rate_8h: 0.000_1,
        predicted_rate: None,
        next_funding_time: 1_700_000_000_000,
        funding_interval: 8,
        volume_24h: 100_000.0,
        timestamp: 1_700_000_000_000,
        smoothed_rate: None,
        rate_std: None,
        is_outlier: false,
    }
}

fn evidence(venue: &str, symbol: &str, price: f64) -> OpportunityLegMarketEvidence {
    OpportunityLegMarketEvidence {
        venue: venue.into(),
        symbol: symbol.into(),
        price: Some(price),
        health: shared_types::MarketDataHealth {
            quality: shared_types::MarketDataQuality::Fresh,
            source: shared_types::MarketDataSourceKind::LocalCache,
            freshness_ms: Some(10),
            retry_after_ms: None,
            last_error: None,
            observed_at_ms: 1,
            coverage: None,
            problem: None,
        },
    }
}
