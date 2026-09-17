use super::*;
use crate::models::{
    FundingMarketEvidence, IndexCompositionRisk, OpportunityHistoryStats, RawOpportunityExtra,
};
use pretty_assertions::assert_eq;
use shared_types::{
    ArbitrageOpportunityDto, FeeProduct, FundingDiffSampleHealth, FundingDiffWindowStats,
    FundingRateData, HedgeLegRole, IndexCompositionEvidence, IndexCompositionQuality,
    IndexCompositionStatus, LegCostBreakdown, MarketDataHealth, MarketDataQuality,
    MarketDataSourceKind, OpportunityLegMarketEvidence, RoundTripCostBreakdown, SpotLegMode,
    StrategyCategory, StrategyKind, TradeFeeEvidence, TradeFeeSnapshot, TradeFeeSource,
};

fn fr(ex: &str, rate_8h: f64, vol: f64, next: i64) -> FundingRateData {
    FundingRateData {
        symbol: "BTC".into(),
        exchange: ex.into(),
        rate: rate_8h,
        rate_8h,
        predicted_rate: None,
        next_funding_time: next,
        funding_interval: 8,
        volume_24h: vol,
        timestamp: 0,
        smoothed_rate: None,
        rate_std: None,
        is_outlier: false,
    }
}

fn raw(long: &str, short: &str) -> RawOpportunity {
    // One aligned native settlement covers the 15 bps test round-trip cost.
    let observed_at_ms = Utc::now().timestamp_millis();
    let next_funding_ms = observed_at_ms + 8 * 3_600_000;
    RawOpportunity {
        symbol: "BTC".into(),
        arb_type: ArbitrageType::CrossExchange,
        long_exchange: long.into(),
        short_exchange: short.into(),
        long_rate: fr(long, 0.0001, 1_500_000_000.0, next_funding_ms),
        short_rate: fr(short, 0.0021, 1_000_000_000.0, next_funding_ms),
        spread_8h: 0.002,
        single_yield: 0.002,
        extra: RawOpportunityExtra {
            strategy_kind: Some(StrategyKind::PerpCross),
            annualized_funding_bps: None,
            long_price: Some(100.0),
            short_price: Some(100.02),
            long_leg_market_evidence: Some(leg_market_evidence(long, 100.0, observed_at_ms)),
            short_leg_market_evidence: Some(leg_market_evidence(short, 100.02, observed_at_ms)),
            long_funding_evidence: Some(funding_evidence(long, observed_at_ms)),
            short_funding_evidence: Some(funding_evidence(short, observed_at_ms)),
            cost_round_trip: Some(verified_round_trip_cost_for(long, short)),
            ..Default::default()
        },
    }
}

fn funding_evidence(_venue: &str, observed_at_ms: i64) -> FundingMarketEvidence {
    FundingMarketEvidence {
        quality: MarketDataQuality::Fresh,
        source: MarketDataSourceKind::WsPush,
        observed_at_ms,
    }
}

fn leg_market_evidence(
    venue: &str,
    price: f64,
    observed_at_ms: i64,
) -> OpportunityLegMarketEvidence {
    OpportunityLegMarketEvidence {
        venue: venue.into(),
        symbol: "BTCUSDT".into(),
        price: Some(price),
        health: MarketDataHealth {
            quality: MarketDataQuality::Fresh,
            source: MarketDataSourceKind::WsPush,
            freshness_ms: Some(10),
            retry_after_ms: None,
            last_error: None,
            observed_at_ms,
            coverage: None,
            problem: None,
        },
    }
}

#[test]
fn liquidity_score_clamped() {
    assert!((liquidity_score(0.0, 100.0, 1000.0) - 0.0).abs() < 1e-9);
    assert!((liquidity_score(1e10, 1e10, 1e7) - 100.0).abs() < 1e-9);
    let mid = liquidity_score(5e6, 5e6, 1e7);
    assert!((mid - 50.0).abs() < 1e-9);
}

#[test]
fn list_risk_reflects_strategy_realization_instead_of_scan_cadence() {
    let perp_cross = raw("binance", "okx");
    let mut spot_perp = perp_cross.clone();
    spot_perp.arb_type = ArbitrageType::SpotFutures;
    spot_perp.extra.strategy_kind = Some(StrategyKind::SpotPerp);

    assert_eq!(
        strategy_registry::list_risk_level(&perp_cross),
        RiskLevel::Medium
    );
    assert_eq!(
        strategy_registry::list_risk_level(&spot_perp),
        RiskLevel::High
    );
}

#[test]
fn type_labels_chinese() {
    assert_eq!(type_label(ArbitrageType::CrossExchange), "跨所期期");
    assert_eq!(type_label(ArbitrageType::SpotFutures), "同所期现");
    assert_eq!(type_label(ArbitrageType::CrossSpotFutures), "跨所期现");
    assert_eq!(type_label(ArbitrageType::SpotCross), "现货跨所");
    assert_eq!(type_label(ArbitrageType::Triangular), "三角套利");
    assert_eq!(type_label(ArbitrageType::FundingCarry), "资金费 Carry");
    assert_eq!(type_label(ArbitrageType::OptionsPerpBasis), "期权-永续基差");
}

#[test]
fn min_nonzero_picks_other_when_self_zero() {
    assert_eq!(0_i64.min_nonzero(100), 100);
    assert_eq!(100_i64.min_nonzero(0), 100);
    assert_eq!(0_i64.min_nonzero(0), 0);
    assert_eq!(50_i64.min_nonzero(100), 50);
}

#[test]
fn recommendation_requires_verified_positive_profit() {
    let profitable = dto_from_raw(&raw("binance", "okx"));
    let cost = profitable
        .execution_cost
        .as_ref()
        .expect("verified execution cost");

    assert_eq!(
        recommendation_from_profit(cost, RiskLevel::Low, true),
        Recommendation::Buy
    );

    let mut unprofitable = cost.clone();
    unprofitable.one_cycle.net_bps = -1.0;
    unprofitable.one_cycle.covers_round_trip_cost = false;
    assert_eq!(
        recommendation_from_profit(&unprofitable, RiskLevel::Low, true),
        Recommendation::Avoid
    );
}

#[test]
fn recommendation_holds_when_profit_is_verified_but_execution_is_not_ready() {
    let profitable = dto_from_raw(&raw("binance", "okx"));
    let cost = profitable
        .execution_cost
        .as_ref()
        .expect("verified execution cost");

    assert_eq!(
        recommendation_from_profit(cost, RiskLevel::Low, false),
        Recommendation::Hold
    );
    assert_eq!(
        recommendation_from_profit(cost, RiskLevel::High, true),
        Recommendation::Hold
    );
}

#[test]
fn perp_cross_never_amortizes_an_unprofitable_joint_event() {
    let mut r = raw("binance", "okx");
    set_native_spread(&mut r, 0.0);
    let cost = CostBreakdown {
        fee_rate: 0.0,
        slippage: 0.0,
        round_trip_cost: 0.001,
    };
    let profile = execution_cost_profile(&r, &cost, u32::MAX);

    assert_eq!(profile.breakeven_periods, 0);
    assert_eq!(profile.recommended_hold_periods, 0);
    assert_eq!(profile.one_cycle.net_bps, -10.0);
}

#[test]
fn one_time_strategy_skips_periodic_cost_warning() {
    let mut r = raw("binance", "okx");
    r.arb_type = ArbitrageType::Triangular;
    r.spread_8h = 0.000_1;
    let cost = CostBreakdown {
        fee_rate: 0.0,
        slippage: 0.0,
        round_trip_cost: 0.01,
    };

    let warnings = build_warnings(&r, &cost);

    assert!(!warnings.iter().any(|item| item.contains("摊销")));
}

#[test]
fn build_dto_basic_shape() {
    let dto = basic_dto();

    assert_basic_pairing(&dto);
    assert_risk_fields(&dto);
    assert_futures_fields(&dto);
}

#[test]
fn perp_cross_does_not_expose_an_annualized_funding_value() {
    let mut r = raw("hyperliquid", "binance");
    r.long_rate.rate_8h = 0.000_8;
    r.long_rate.funding_interval = 1;
    r.short_rate.rate_8h = 0.0;
    r.short_rate.funding_interval = 1;
    r.extra.annualized_funding_bps = None;

    let m = RiskMetrics {
        volatility: 0.2,
        sharpe_ratio: 1.5,
        sample_size: 50,
        ..Default::default()
    };
    let p = PositionSizing {
        kelly_fraction: 0.05,
        optimal_position: 5_000.0,
        max_position: 10_000.0,
        risk_adjusted_position: 5_000.0,
    };
    let c = CostBreakdown {
        fee_rate: 0.000_45,
        slippage: 0.000_2,
        round_trip_cost: 0.001_5,
    };
    let dto = OpportunityBuilder {
        raw: &r,
        metrics: &m,
        position: &p,
        cost: &c,
        min_holding_periods: 4,
        net_single_yield: 0.000_3,
        data_source: "rest",
        confidence: 0.9,
    }
    .build();

    assert_eq!(dto.annualized_funding_bps, None);
}

#[test]
fn p0_builder_never_fabricates_annualized_funding_from_legacy_rate() {
    for (arb_type, kind) in [
        (ArbitrageType::CrossExchange, StrategyKind::PerpPriceSpread),
        (ArbitrageType::SpotCross, StrategyKind::SpotCross),
    ] {
        let mut r = raw("binance", "okx");
        r.arb_type = arb_type;
        r.extra.strategy_kind = Some(kind);
        r.extra.annualized_funding_bps = None;
        r.long_rate.rate = 0.000_01;
        r.long_rate.rate_8h = 0.50;
        r.short_rate.rate = 0.000_02;
        r.short_rate.rate_8h = 0.75;

        let dto = dto_from_raw(&r);

        assert_eq!(dto.annualized_funding_bps, None, "strategy={kind:?}");
        assert_eq!(dto.long_rate_8h, r.long_rate.rate, "strategy={kind:?}");
        assert_eq!(dto.short_rate_8h, r.short_rate.rate, "strategy={kind:?}");
        assert_eq!(dto.spread_8h, r.single_yield, "strategy={kind:?}");
    }
}

#[test]
fn explicit_native_funding_projection_survives_without_legacy_recalculation() {
    let mut r = raw("binance", "okx");
    r.arb_type = ArbitrageType::CrossSpotFutures;
    r.extra.strategy_kind = Some(StrategyKind::CrossSpotPerp);
    r.extra.annualized_funding_bps = Some(876.0);
    r.long_rate.rate = 0.0;
    r.long_rate.rate_8h = 0.0;
    r.short_rate.rate = 0.000_01;
    r.short_rate.rate_8h = 0.50;

    let dto = dto_from_raw(&r);

    assert_eq!(dto.annualized_funding_bps, Some(876.0));
    assert_eq!(dto.short_rate_8h, 0.000_01);
    assert_eq!(dto.spread_8h, r.single_yield);
}

#[test]
fn native_one_cycle_yield_drives_cost_prediction_and_hold_hours() {
    let mut r = raw("hyperliquid:xyz", "okx");
    r.long_rate.rate = 0.000_01;
    r.long_rate.rate_8h = 0.000_08;
    r.long_rate.funding_interval = 1;
    r.short_rate.rate = 0.000_16;
    r.short_rate.rate_8h = 0.000_16;
    r.short_rate.funding_interval = 8;
    r.spread_8h = 0.000_08;
    r.single_yield = 0.000_15;
    r.extra.annualized_funding_bps = Some(876.0);
    let settlement = Utc::now().timestamp_millis() + 30 * 60_000;
    r.long_rate.next_funding_time = settlement;
    r.short_rate.next_funding_time = settlement;
    r.extra.short_price = Some(100.000_5);
    if let Some(evidence) = r.extra.short_leg_market_evidence.as_mut() {
        evidence.price = r.extra.short_price;
    }
    r.extra.cost_round_trip = None;

    let metrics = RiskMetrics {
        volatility: 0.2,
        sharpe_ratio: 1.5,
        sample_size: 50,
        ..Default::default()
    };
    let position = PositionSizing {
        kelly_fraction: 0.05,
        optimal_position: 5_000.0,
        max_position: 10_000.0,
        risk_adjusted_position: 5_000.0,
    };
    let cost = CostBreakdown {
        fee_rate: 0.0,
        slippage: 0.0,
        round_trip_cost: 0.0,
    };

    let dto = OpportunityBuilder {
        raw: &r,
        metrics: &metrics,
        position: &position,
        cost: &cost,
        min_holding_periods: 1,
        net_single_yield: 0.000_15,
        data_source: "rest",
        confidence: 0.9,
    }
    .build();

    let cost = dto.execution_cost.as_ref().expect("execution cost");
    let prediction = dto
        .predicted_next_funding
        .as_ref()
        .expect("predicted next funding");

    assert_eq!(dto.settlement_interval, 1);
    assert!(dto
        .min_hold_hours
        .is_some_and(|hours| (hours - 0.5).abs() < 0.01));
    assert!((cost.gross_edge_bps - 1.5).abs() < 1e-12);
    assert_eq!(cost.breakeven_periods, 1);
    assert_eq!(cost.recommended_hold_periods, 1);
    assert!((cost.recommended_hold_hours - 0.5).abs() < 0.01);
    assert!((prediction.long_bps - 0.1).abs() < 1e-12);
    assert!((prediction.short_bps - 1.6).abs() < 1e-12);
    assert!((prediction.net_bps - 1.5).abs() < 1e-12);
    assert_eq!(dto.annualized_funding_bps, None);
}

#[test]
fn build_dto_id_is_stable_for_same_pair() {
    let first = basic_dto();
    let second = basic_dto();

    assert_eq!(first.id, "perp_cross_binance_okx_BTC_lm_BTCUSDT_sm_BTCUSDT");
    assert_eq!(first.id, second.id);
}

#[test]
fn spot_leg_mode_is_part_of_the_stable_opportunity_identity() {
    let mut buy_spot = raw("kucoin", "bitget");
    buy_spot.arb_type = ArbitrageType::CrossSpotFutures;
    buy_spot.extra.strategy_kind = Some(StrategyKind::CrossSpotPerp);
    buy_spot.extra.spot_leg_mode = Some(SpotLegMode::BuySpot);
    let mut borrow_sell = buy_spot.clone();
    borrow_sell.extra.spot_leg_mode = Some(SpotLegMode::BorrowAndSell);

    let buy_spot = dto_from_raw(&buy_spot);
    let borrow_sell = dto_from_raw(&borrow_sell);

    assert_eq!(
        buy_spot.id,
        "cross_spot_perp_buy_spot_kucoin_bitget_BTC_lm_BTCUSDT_sm_BTCUSDT"
    );
    assert_eq!(
        borrow_sell.id,
        "cross_spot_perp_borrow_sell_kucoin_bitget_BTC_lm_BTCUSDT_sm_BTCUSDT"
    );
    assert_ne!(buy_spot.id, borrow_sell.id);
}

#[test]
fn native_leg_symbols_are_part_of_the_stable_opportunity_identity() {
    let mut usd = raw("okx", "bitget");
    usd.symbol = "ETH".into();
    usd.arb_type = ArbitrageType::CrossSpotFutures;
    usd.extra.strategy_kind = Some(StrategyKind::CrossSpotPerp);
    usd.extra.spot_leg_mode = Some(SpotLegMode::BuySpot);
    usd.extra
        .long_leg_market_evidence
        .as_mut()
        .expect("long market evidence")
        .symbol = "ETH/USD".into();
    usd.extra
        .short_leg_market_evidence
        .as_mut()
        .expect("short market evidence")
        .symbol = "ETH".into();
    let mut usdc = usd.clone();
    usdc.extra
        .long_leg_market_evidence
        .as_mut()
        .expect("long market evidence")
        .symbol = "ETH/USDC".into();

    let usd = dto_from_raw(&usd);
    let usdc = dto_from_raw(&usdc);

    assert_eq!(
        usd.id,
        "cross_spot_perp_buy_spot_okx_bitget_ETH_lm_ETH~2FUSD_sm_ETH"
    );
    assert_eq!(
        usdc.id,
        "cross_spot_perp_buy_spot_okx_bitget_ETH_lm_ETH~2FUSDC_sm_ETH"
    );
    assert_ne!(usd.id, usdc.id);
}

fn basic_dto() -> ArbitrageOpportunityDto {
    dto_from_raw(&raw("binance", "okx"))
}

fn dto_from_raw(r: &RawOpportunity) -> ArbitrageOpportunityDto {
    let m = RiskMetrics {
        volatility: 0.2,
        sharpe_ratio: 1.5,
        sample_size: 50,
        ..Default::default()
    };
    let p = PositionSizing {
        kelly_fraction: 0.05,
        optimal_position: 5_000.0,
        max_position: 10_000.0,
        risk_adjusted_position: 5_000.0,
    };
    let c = CostBreakdown {
        fee_rate: 0.000_45,
        slippage: 0.000_2,
        round_trip_cost: 0.001_5,
    };
    let min_holding_periods = strategy_registry::min_holding_periods(r, &c);
    let net_single_yield = strategy_registry::net_single_yield(r, &c, min_holding_periods);
    let b = OpportunityBuilder {
        raw: r,
        metrics: &m,
        position: &p,
        cost: &c,
        min_holding_periods,
        net_single_yield,
        data_source: "rest",
        confidence: 0.9,
    };
    b.build()
}

fn assert_basic_pairing(dto: &ArbitrageOpportunityDto) {
    assert_eq!(dto.symbol, "BTC");
    assert_eq!(dto.long_exchange, "binance");
    assert_eq!(dto.short_exchange, "okx");
    assert_eq!(dto.score, 0.0);
    assert!(dto.score_breakdown.is_none());
    assert!(dto.ranking_key.is_none());
    assert_eq!(dto.long_funding_interval, 8);
    assert_eq!(dto.settlement_interval, 8);
    assert!(dto.long_action.contains("binance"));
    assert!(dto.short_action.contains("okx"));
    assert!(dto.execution_eligible);
    assert!(dto.execution_blockers.is_empty());
}

fn assert_risk_fields(dto: &ArbitrageOpportunityDto) {
    assert!(matches!(dto.risk_level, RiskLevel::Medium));
    assert!(!dto.settlement_time_diff);
}

fn assert_futures_fields(dto: &ArbitrageOpportunityDto) {
    assert_eq!(dto.strategy_kind, Some(StrategyKind::PerpCross));
    assert_eq!(dto.strategy_category, Some(StrategyCategory::Futures));
    assert!(dto.basis_bps.is_some_and(|bps| (bps - 5.0).abs() < 1e-9));
    assert_eq!(dto.annualized_funding_bps, None);
    assert_eq!(
        dto.predicted_next_funding
            .as_ref()
            .map(|funding| funding.net_bps),
        Some(20.0)
    );
    assert_eq!(dto.funding_window_alignment_minutes, Some(0));
    assert_eq!(dto.funding_cap_distance_bps, None);
    assert!(dto
        .min_hold_hours
        .is_some_and(|hours| (hours - 8.0).abs() < 0.01));
    assert!(dto
        .settlement_countdown_seconds
        .is_some_and(|seconds| seconds > 28_700 && seconds <= 28_800));
    assert_execution_cost_fields(dto);
}

fn assert_execution_cost_fields(dto: &ArbitrageOpportunityDto) {
    let cost = dto.execution_cost.as_ref().expect("execution cost profile");
    assert_eq!(cost.gross_edge_bps, 20.0);
    assert_eq!(cost.total_cost_bps, 15.0);
    assert_eq!(cost.wear_bps, 8.0);
    assert_eq!(cost.one_cycle.open_fee_bps, 3.5);
    assert_eq!(cost.one_cycle.close_fee_bps, 3.5);
    assert_eq!(cost.one_cycle.open_slippage_bps, 4.0);
    assert_eq!(cost.one_cycle.close_slippage_bps, 4.0);
    assert_eq!(cost.one_cycle.net_bps, 5.0);
    assert!(cost.one_cycle.covers_round_trip_cost);
    assert_eq!(cost.breakeven_periods, 1);
    assert_eq!(cost.recommended_hold_periods, 1);
    assert!((cost.recommended_hold_hours - 8.0).abs() < 0.01);
}

#[test]
fn higher_native_edge_increases_verified_one_cycle_profit() {
    let low = dto_from_raw(&raw("binance", "okx"));
    let mut high_raw = raw("binance", "okx");
    set_native_spread(&mut high_raw, 0.004);
    let high = dto_from_raw(&high_raw);

    assert!(low.execution_eligible);
    assert!(low.execution_blockers.is_empty());
    let low_net = low
        .execution_cost
        .as_ref()
        .map(|cost| cost.one_cycle.net_bps)
        .expect("low execution cost");
    let high_net = high
        .execution_cost
        .as_ref()
        .map(|cost| cost.one_cycle.net_bps)
        .expect("high execution cost");
    assert!(low_net < high_net);
    assert!(high
        .execution_cost
        .as_ref()
        .is_some_and(|cost| cost.one_cycle.net_bps > 0.0 && cost.one_cycle.covers_round_trip_cost));
}

#[test]
fn profitability_evidence_carries_verified_fee_ids() {
    let mut r = raw("binance", "kucoin");
    r.extra.cost_round_trip = Some(verified_round_trip_cost());

    let dto = dto_from_raw(&r);
    let evidence = &dto
        .execution_cost
        .as_ref()
        .and_then(|cost| cost.round_trip.as_ref())
        .expect("round-trip cost")
        .profitability_evidence;

    assert!(evidence.is_cost_verified());
    assert_eq!(
        evidence.fee_evidence_ids,
        vec!["fee:binance:perp:vip0", "fee:kucoin:perp:vip0"]
    );
}

#[test]
fn profitability_evidence_rejects_stale_or_problem_fee_snapshots() {
    let mut r = raw("binance", "kucoin");
    let mut round_trip = verified_round_trip_cost();
    let now_ms = Utc::now().timestamp_millis();
    if let Some(snapshot) = round_trip.long_leg.fee_snapshot.as_mut() {
        snapshot.valid_until_ms = now_ms - 1;
    }
    if let Some(snapshot) = round_trip.short_leg.fee_snapshot.as_mut() {
        snapshot.verification_problem = Some("official fee schedule mismatch".into());
    }
    r.extra.cost_round_trip = Some(round_trip);

    let dto = dto_from_raw(&r);
    let evidence = &dto
        .execution_cost
        .as_ref()
        .and_then(|cost| cost.round_trip.as_ref())
        .expect("round-trip cost")
        .profitability_evidence;

    assert!(!evidence.is_cost_verified());
    assert!(evidence.fee_evidence_ids.is_empty());
}

fn verified_round_trip_cost() -> RoundTripCostBreakdown {
    verified_round_trip_cost_for("binance", "kucoin")
}

fn verified_round_trip_cost_for(long_venue: &str, short_venue: &str) -> RoundTripCostBreakdown {
    let long = fee_snapshot(long_venue, &format!("fee:{long_venue}:perp:vip0"));
    let short = fee_snapshot(short_venue, &format!("fee:{short_venue}:perp:vip0"));
    RoundTripCostBreakdown {
        long_leg: leg_cost(HedgeLegRole::Long, long),
        short_leg: leg_cost(HedgeLegRole::Short, short),
        open_fee_bps: 3.5,
        close_fee_bps: 3.5,
        open_slippage_bps: 4.0,
        close_slippage_bps: 4.0,
        borrow_or_financing_bps: 0.0,
        funding_window_mismatch_buffer_bps: 0.0,
        min_profit_buffer_bps: 0.0,
        total_cost_bps: 15.0,
        one_cycle_net_bps: 5.0,
        profitability_evidence: Default::default(),
    }
}

fn leg_cost(role: HedgeLegRole, fee_snapshot: TradeFeeSnapshot) -> LegCostBreakdown {
    LegCostBreakdown {
        role,
        venue: fee_snapshot.venue.clone(),
        symbol: fee_snapshot.symbol.clone(),
        product: fee_snapshot.product,
        open_fee_bps: fee_snapshot.open_fee_bps,
        close_fee_bps: fee_snapshot.close_fee_bps,
        open_slippage_bps: 2.0,
        close_slippage_bps: 2.0,
        fee_snapshot: Some(fee_snapshot),
    }
}

fn fee_snapshot(venue: &str, evidence_id: &str) -> TradeFeeSnapshot {
    let now_ms = Utc::now().timestamp_millis();
    TradeFeeSnapshot {
        venue: venue.into(),
        symbol: "BTCUSDT".into(),
        product: FeeProduct::Perp,
        account_id: None,
        maker_fee_bps: 1.75,
        taker_fee_bps: 1.75,
        open_fee_bps: 1.75,
        close_fee_bps: 1.75,
        source: TradeFeeSource::OfficialSchedule,
        fetched_at_ms: now_ms - 60_000,
        valid_until_ms: now_ms + 86_400_000,
        freshness_ms: Some(1_000),
        evidence: Some(TradeFeeEvidence {
            evidence_id: evidence_id.into(),
            source_name: format!("{venue} unit test fee fixture"),
            source_url: "https://example.com/official-fees".into(),
            checked_at_ms: now_ms - 60_000,
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

#[test]
fn large_price_deviation_penalizes_and_blocks_pure_funding_execution() {
    let mut wide = raw("binance", "okx");
    wide.extra.price_deviation = Some(0.02);
    wide.extra.short_price = Some(102.0);
    let penalized = dto_from_raw(&wide);

    assert!(!penalized.execution_eligible);
    assert!(penalized
        .execution_blockers
        .iter()
        .any(|blocker| blocker.contains("双边可成交价格差")));
    assert!(penalized
        .risk_warnings
        .iter()
        .any(|warning| warning.contains("双边价格差")));
}

#[test]
fn price_deviation_falls_back_to_verified_leg_prices() {
    let mut row = raw("binance", "okx");
    row.extra.price_deviation = None;
    row.extra.long_price = Some(100.0);
    row.extra.short_price = Some(101.0);

    let gap = price_deviation_bps(&row).expect("leg prices should derive price gap");

    assert!((gap - 99.502_487_562_189_06).abs() < 1e-9);
}

#[test]
fn pair_history_stats_preserve_percentile_evidence() {
    let mut strong_history = raw("binance", "okx");
    strong_history.extra.history_stats = Some(history_stats(92, 0.9, 9, 0, 1.0));
    let strong = dto_from_raw(&strong_history);

    let mut weak_history = raw("binance", "okx");
    weak_history.extra.history_stats = Some(history_stats(20, 0.3, 9, 4, 16.0));
    let weak = dto_from_raw(&weak_history);

    assert_eq!(
        strong
            .funding_diff_window
            .as_ref()
            .map(|window| window.current_percentile),
        Some(92)
    );
    assert_eq!(
        weak.funding_diff_window
            .as_ref()
            .map(|window| window.current_percentile),
        Some(20)
    );
    assert!(strong
        .funding_diff_window
        .as_ref()
        .is_some_and(|window| window.cycles == 9 && window.sample_count == 9));
    assert_eq!(strong.funding_diff_windows.len(), 1);
    assert!(!serde_json::to_value(&strong)
        .expect("serialize opportunity")
        .as_object()
        .expect("opportunity object")
        .contains_key("fundingHistoryPercentile90d"));
}

#[test]
fn stale_pair_history_remains_visible_without_authorizing_profit() {
    let mut stale_history = raw("binance", "okx");
    let mut stats = history_stats(100, 1.0, 9, 0, 0.1);
    stats.window.sample_health = FundingDiffSampleHealth::Stale;
    stats.windows[0].sample_health = FundingDiffSampleHealth::Stale;
    stale_history.extra.history_stats = Some(stats);

    let stale = dto_from_raw(&stale_history);

    assert_eq!(stale.score, 0.0);
    assert!(stale.score_breakdown.is_none());
    assert!(stale.ranking_key.is_none());
    assert_eq!(
        stale
            .funding_diff_window
            .as_ref()
            .map(|window| window.current_percentile),
        Some(100)
    );
}

fn set_native_spread(raw: &mut RawOpportunity, spread: f64) {
    raw.short_rate.rate = raw.long_rate.rate + spread;
    raw.short_rate.rate_8h = raw.long_rate.rate_8h + spread;
    raw.spread_8h = spread;
    raw.single_yield = spread;
}

fn history_stats(
    current_percentile: u8,
    positive_ratio: f64,
    sample_count: usize,
    reversal_count: usize,
    stddev_diff_bps: f64,
) -> OpportunityHistoryStats {
    let window = FundingDiffWindowStats {
        cycles: 9,
        window_hours: 72,
        sample_count,
        mean_diff_bps: 4.0,
        p50_diff_bps: 4.0,
        p75_diff_bps: 6.0,
        p90_diff_bps: 8.0,
        p95_diff_bps: 9.0,
        stddev_diff_bps,
        positive_ratio,
        reversal_count,
        current_percentile,
        source: "test".into(),
        freshness_ms: Some(0),
        sample_health: FundingDiffSampleHealth::Ok,
        problem: None,
        problem_detail: None,
        retry_after_ms: None,
        evidence: Default::default(),
    };
    OpportunityHistoryStats {
        window: window.clone(),
        windows: vec![window],
    }
}

#[test]
fn missing_trade_price_blocks_execution() {
    let mut r = raw("binance", "okx");
    r.extra.long_price = None;
    let dto = dto_from_raw(&r);

    assert!(!dto.execution_eligible);
    assert!(dto
        .execution_blockers
        .iter()
        .any(|blocker| blocker == "缺少交易所双腿报价，仅观察不执行"));
}

#[test]
fn rest_funding_evidence_keeps_a_discovered_candidate_internal() {
    let mut r = raw("binance", "okx");
    r.extra
        .short_funding_evidence
        .as_mut()
        .expect("short funding evidence")
        .source = MarketDataSourceKind::RestBaseline;

    let dto = dto_from_raw(&r);

    assert!(!dto.execution_eligible);
    assert!(dto
        .execution_blockers
        .iter()
        .any(|blocker| blocker == FUNDING_WS_EVIDENCE_BLOCKER));
}

#[test]
fn rest_trade_evidence_keeps_a_discovered_candidate_internal() {
    let mut r = raw("binance", "okx");
    r.extra
        .short_leg_market_evidence
        .as_mut()
        .expect("short market evidence")
        .health
        .source = MarketDataSourceKind::RestBaseline;

    let dto = dto_from_raw(&r);

    assert!(!dto.execution_eligible);
    assert!(dto
        .execution_blockers
        .iter()
        .any(|blocker| blocker == "缺少交易所双腿行情证据，仅观察不执行"));
}

#[test]
fn index_composition_risk_surfaces_status_and_blocker() {
    let mut r = raw("binance", "kucoin");
    r.extra.index_composition = Some(IndexCompositionRisk {
        overlap_score: 0.62,
        long_quality: IndexCompositionQuality::Verified,
        short_quality: IndexCompositionQuality::Verified,
        hidden_price: false,
        blocker: Some("MU 双边指数成分重合度 62% 低于 75%，不能直接构建对冲".into()),
        long_evidence: Some(IndexCompositionEvidence {
            source: "GET /fapi/v1/constituents".into(),
            received_at_ms: 1_700_000_000_000,
            freshness_ms: Some(0),
            source_url: None,
            payload_sha256: None,
            schema_version: None,
        }),
        short_evidence: None,
    });

    let dto = dto_from_raw(&r);
    let profile = dto
        .index_composition
        .as_ref()
        .expect("index composition profile");

    assert_eq!(profile.status, IndexCompositionStatus::Mismatch);
    assert!((profile.overlap_score - 0.62).abs() < f64::EPSILON);
    assert!(!dto.execution_eligible);
    assert!(dto
        .execution_blockers
        .iter()
        .any(|item| item.contains("指数成分重合度")));
    let evidence = profile.long_evidence.as_ref().expect("long evidence");
    assert_eq!(evidence.source, "GET /fapi/v1/constituents");
    assert!(profile.short_evidence.is_none());
}
