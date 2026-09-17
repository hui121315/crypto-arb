use arbitrage::models::{RawOpportunity, RawOpportunityExtra};
use arbitrage::{CostBreakdown, OpportunityBuilder, PositionSizing, RiskMetrics};
use shared_types::instrument_registry::{InstrumentAssetClass, VenueInstrument};
use shared_types::{
    ArbitrageOpportunityDto, ArbitrageType, ExecutionCostProfile, FeeProduct, FeeScheduleEvidence,
    FundingRateData, HedgeLegRole, InstrumentListingStatus, InstrumentMetadataSource,
    LegCostBreakdown, MarketDataHealth, MarketDataQuality, MarketDataSourceKind,
    OneCycleCostProfile, OpportunityLegMarketEvidence, OrderBookInfo, ProfitabilityEvidence,
    RankingKey, RoundTripCostBreakdown, StrategyKind, TickerInfo, TradeFeeSnapshot, TradeFeeSource,
};

pub(super) fn automation_opportunity(now_ms: i64) -> ArbitrageOpportunityDto {
    let raw = RawOpportunity {
        symbol: "BTC".to_owned(),
        arb_type: ArbitrageType::CrossExchange,
        long_exchange: "binance".to_owned(),
        short_exchange: "okx".to_owned(),
        long_rate: funding_rate("binance", now_ms),
        short_rate: funding_rate("okx", now_ms),
        spread_8h: 0.004,
        single_yield: 0.004,
        extra: RawOpportunityExtra {
            strategy_kind: Some(StrategyKind::PerpCross),
            long_price: Some(100.0),
            short_price: Some(100.05),
            long_leg_market_evidence: Some(market_evidence("binance", 100.0, now_ms)),
            short_leg_market_evidence: Some(market_evidence("okx", 100.05, now_ms)),
            ..RawOpportunityExtra::default()
        },
    };
    let mut opportunity = OpportunityBuilder {
        raw: &raw,
        metrics: &RiskMetrics::default(),
        position: &PositionSizing::default(),
        cost: &CostBreakdown::default(),
        min_holding_periods: 1,
        net_single_yield: raw.single_yield,
        data_source: "automation-paper-e2e",
        confidence: 1.0,
    }
    .build();
    opportunity.id = "automation-paper-opportunity".to_owned();
    opportunity.score = 90.0;
    opportunity.execution_eligible = true;
    opportunity.execution_blockers.clear();
    opportunity.ranking_key = Some(RankingKey {
        final_score: 90.0,
        one_cycle_net_bps: 30.0,
        ..RankingKey::default()
    });
    opportunity.execution_cost = Some(execution_cost(now_ms));
    opportunity
}

fn funding_rate(venue: &str, now_ms: i64) -> FundingRateData {
    let native_rate = match venue {
        "binance" => 0.0,
        "okx" => 0.004,
        _ => unreachable!("paper E2E only seeds binance and okx"),
    };
    FundingRateData {
        symbol: "BTC".to_owned(),
        exchange: venue.to_owned(),
        rate: native_rate,
        rate_8h: native_rate,
        predicted_rate: None,
        next_funding_time: now_ms.saturating_add(8 * 60 * 60 * 1_000),
        funding_interval: 8,
        volume_24h: 1_000_000.0,
        timestamp: now_ms,
        smoothed_rate: None,
        rate_std: None,
        is_outlier: false,
    }
}

fn market_evidence(venue: &str, price: f64, now_ms: i64) -> OpportunityLegMarketEvidence {
    OpportunityLegMarketEvidence {
        venue: venue.to_owned(),
        symbol: "BTC".to_owned(),
        price: Some(price),
        health: MarketDataHealth {
            quality: MarketDataQuality::Fresh,
            source: MarketDataSourceKind::WsPush,
            freshness_ms: Some(0),
            retry_after_ms: None,
            last_error: None,
            observed_at_ms: now_ms,
            coverage: None,
            problem: None,
        },
    }
}

fn execution_cost(now_ms: i64) -> ExecutionCostProfile {
    let long_fee = fee_snapshot("binance", now_ms);
    let short_fee = fee_snapshot("okx", now_ms);
    let profitability_evidence = ProfitabilityEvidence::from_fee_snapshots(
        "automation-paper-e2e",
        now_ms,
        &[&long_fee, &short_fee],
        None,
    );
    ExecutionCostProfile {
        gross_edge_bps: 40.0,
        fee_bps: 4.0,
        wear_bps: 6.0,
        total_cost_bps: 10.0,
        one_cycle: OneCycleCostProfile {
            gross_edge_bps: 40.0,
            open_fee_bps: 2.0,
            close_fee_bps: 2.0,
            open_slippage_bps: 3.0,
            close_slippage_bps: 3.0,
            net_bps: 30.0,
            covers_round_trip_cost: true,
            ..OneCycleCostProfile::default()
        },
        breakeven_periods: 1,
        breakeven_hours: 8.0,
        recommended_hold_periods: 1,
        recommended_hold_hours: 8.0,
        net_bps_at_recommended_hold: 30.0,
        round_trip: Some(RoundTripCostBreakdown {
            long_leg: leg_cost(HedgeLegRole::Long, long_fee),
            short_leg: leg_cost(HedgeLegRole::Short, short_fee),
            open_fee_bps: 2.0,
            close_fee_bps: 2.0,
            open_slippage_bps: 3.0,
            close_slippage_bps: 3.0,
            borrow_or_financing_bps: 0.0,
            funding_window_mismatch_buffer_bps: 0.0,
            min_profit_buffer_bps: 0.0,
            total_cost_bps: 10.0,
            one_cycle_net_bps: 30.0,
            profitability_evidence,
        }),
    }
}

fn leg_cost(role: HedgeLegRole, fee_snapshot: TradeFeeSnapshot) -> LegCostBreakdown {
    LegCostBreakdown {
        role,
        venue: fee_snapshot.venue.clone(),
        symbol: "BTC".to_owned(),
        product: FeeProduct::Perp,
        open_fee_bps: 1.0,
        close_fee_bps: 1.0,
        open_slippage_bps: 1.5,
        close_slippage_bps: 1.5,
        fee_snapshot: Some(fee_snapshot),
    }
}

fn fee_snapshot(venue: &str, now_ms: i64) -> TradeFeeSnapshot {
    let source_url = match venue {
        "binance" => "https://www.binance.com/en/fee/futureFee",
        _ => "https://www.okx.com/help/trading-fee-rules-faq",
    };
    TradeFeeSnapshot {
        venue: venue.to_owned(),
        symbol: "BTC".to_owned(),
        product: FeeProduct::Perp,
        account_id: None,
        maker_fee_bps: 1.0,
        taker_fee_bps: 1.0,
        open_fee_bps: 1.0,
        close_fee_bps: 1.0,
        source: TradeFeeSource::OfficialSchedule,
        fetched_at_ms: now_ms,
        valid_until_ms: now_ms.saturating_add(60_000),
        freshness_ms: Some(0),
        evidence: Some(FeeScheduleEvidence {
            evidence_id: format!("automation-paper-{venue}-fee"),
            source_name: format!("{venue} official fee schedule"),
            source_url: source_url.to_owned(),
            checked_at_ms: now_ms,
            effective_at_ms: Some(now_ms),
            schedule_version: Some("automation-paper-e2e-v1".to_owned()),
            tier: Some("base".to_owned()),
            scope: Some("perp".to_owned()),
            problem: None,
        }),
        verification_problem: None,
        note: None,
    }
}

pub(super) fn seed_books(
    state: &crate::state::AppState,
    long_price: f64,
    short_price: f64,
    now_ms: i64,
) {
    for (exchange, bid, ask) in [
        ("binance", long_price - 0.05, long_price),
        ("okx", short_price, short_price + 0.05),
    ] {
        state.market_data().store_orderbook(
            OrderBookInfo {
                symbol: "BTC".to_owned(),
                exchange: exchange.to_owned(),
                bids: vec![[bid, 1_000.0]],
                asks: vec![[ask, 1_000.0]],
                timestamp: now_ms,
            },
            crate::services::market_data::MarketSource::WsPush,
        );
        state.market_data().store_ticker_rows(
            &[TickerInfo {
                symbol: "BTC".to_owned(),
                exchange: exchange.to_owned(),
                bid,
                ask,
                last: (bid + ask) / 2.0,
                volume_24h: 1_000_000.0,
                timestamp: now_ms,
            }],
            crate::services::market_data::MarketSource::WsPush,
        );
    }
}

pub(super) fn seed_instruments(state: &crate::state::AppState, now_ms: i64) -> anyhow::Result<()> {
    for (venue, native_symbol) in [("binance", "BTCUSDT"), ("okx", "BTC-USDT-SWAP")] {
        let source_url = crate::services::instrument_registry::instrument_metadata_evidence(venue)
            .map(|evidence| evidence.path.clone());
        state
            .instrument_registry()
            .upsert(VenueInstrument {
                venue: venue.to_owned(),
                native_symbol: native_symbol.to_owned(),
                canonical_symbol: "BTC".to_owned(),
                display_symbol: native_symbol.to_owned(),
                asset_class: InstrumentAssetClass::Crypto,
                product_type: Some("perpetual".to_owned()),
                quote_asset: Some("USDT".to_owned()),
                settle_asset: Some("USDT".to_owned()),
                margin_asset: Some("USDT".to_owned()),
                contract_size: Some(1.0),
                execution_supported: true,
                price_tick: Some(0.1),
                qty_step: Some(0.001),
                min_qty: Some(0.001),
                min_notional: Some(1.0),
                listing_status: InstrumentListingStatus::Trading,
                funding_interval_ms: Some(28_800_000),
                builder_dex: None,
                source: InstrumentMetadataSource::OfficialEndpoint,
                source_url,
                checked_at_ms: now_ms,
                schema_version: Some("automation-paper-e2e-v1".to_owned()),
            })
            .map_err(anyhow::Error::msg)?;
    }
    Ok(())
}
