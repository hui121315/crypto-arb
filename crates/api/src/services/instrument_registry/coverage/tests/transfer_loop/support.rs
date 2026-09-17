use super::super::*;
use exchange::CurrencyTransferNetwork;
use rust_decimal::Decimal;
use shared_types::{
    ArbitrageOpportunityDto, ArbitrageType, ExecutionCostProfile, FeeProduct, FeeScheduleEvidence,
    HedgeLegRole, LegCostBreakdown, OneCycleCostProfile, OpportunityLegMarketEvidence,
    ProfitabilityEvidence, ProfitabilityEvidenceStatus, RoundTripCostBreakdown, TradeFeeSnapshot,
    TradeFeeSource,
};

pub(super) fn ready_registry() -> InstrumentRegistry {
    let registry = listing_ready_registry();
    assert_eq!(
        registry.replace_transfer_venue(
            "binance",
            vec![
                transfer("binance", "BTC", "BTC", false, true, (1, 3)),
                transfer("binance", "USDT", "TRC20", true, false, (0, 0)),
            ],
        ),
        2
    );
    assert_eq!(
        registry.replace_transfer_venue(
            "okx",
            vec![
                transfer("okx", "BTC", "Bitcoin", true, false, (0, 0)),
                transfer("okx", "USDT", "TRON", false, true, (1, 0)),
            ],
        ),
        2
    );
    registry
}

pub(super) fn listing_ready_registry() -> InstrumentRegistry {
    let registry = InstrumentRegistry::default();
    assert_eq!(
        registry.replace_venue("binance", vec![spot("binance", "BTCUSDT")]),
        1
    );
    assert_eq!(
        registry.replace_venue("okx", vec![spot("okx", "BTC-USDT")]),
        1
    );
    registry
}

fn spot(venue: &str, native_symbol: &str) -> VenueInstrument {
    let mut row = instrument(venue, native_symbol);
    row.canonical_symbol = "BTC".into();
    row.product_type = Some("spot".into());
    row.quote_asset = Some("USDT".into());
    row.settle_asset = None;
    row.margin_asset = None;
    row.funding_interval_ms = None;
    row
}

pub(super) fn transfer(
    venue: &str,
    currency: &str,
    network: &str,
    deposit_enabled: bool,
    withdraw_enabled: bool,
    withdrawal: (i64, i64),
) -> CurrencyTransferNetwork {
    let (fee_hundredths, min_hundredths) = withdrawal;
    CurrencyTransferNetwork {
        venue: venue.into(),
        currency: currency.into(),
        network: network.into(),
        canonical_network: exchange::canonical_network_id(network),
        contract_address: None,
        deposit_enabled,
        withdraw_enabled,
        withdrawal_fee: withdraw_enabled.then(|| Decimal::new(fee_hundredths, 2)),
        withdrawal_fee_rate: withdraw_enabled.then_some(Decimal::ZERO),
        withdrawal_step: withdraw_enabled.then_some(Decimal::new(1, 6)),
        min_withdraw: withdraw_enabled.then(|| Decimal::new(min_hundredths.max(1), 2)),
        min_deposit: deposit_enabled.then_some(Decimal::ZERO),
        requires_tag: false,
        credit_confirmations: None,
        unlock_confirmations: None,
        network_status: None,
        checked_at_ms: NOW,
        source_url: "https://official.example/transfer-networks".into(),
    }
}

pub(super) fn spot_cross_opportunity() -> ArbitrageOpportunityDto {
    let mut row = opportunity();
    row.id = "spot-transfer-loop".into();
    row.arb_type = ArbitrageType::SpotCross;
    row.type_label = "现货跨所".into();
    row.strategy_kind = Some(StrategyKind::SpotCross);
    row.strategy_category = Some(StrategyKind::SpotCross.category());
    row.long_price = Some(10.0);
    row.short_price = Some(10.2);
    row.net_single_yield = 0.015;
    row.risk_adjusted_yield = 0.015;
    row.long_leg_market_evidence = Some(market_evidence("binance", "BTCUSDT"));
    row.short_leg_market_evidence = Some(market_evidence("okx", "BTC-USDT"));
    row.execution_cost = Some(verified_cost());
    row
}

fn verified_cost() -> ExecutionCostProfile {
    let long_fee = fee("binance", "BTCUSDT");
    let short_fee = fee("okx", "BTC-USDT");
    ExecutionCostProfile {
        gross_edge_bps: 200.0,
        fee_bps: 40.0,
        wear_bps: 10.0,
        total_cost_bps: 50.0,
        one_cycle: OneCycleCostProfile {
            gross_edge_bps: 200.0,
            net_bps: 150.0,
            covers_round_trip_cost: true,
            ..Default::default()
        },
        breakeven_periods: 1,
        breakeven_hours: 0.0,
        recommended_hold_periods: 1,
        recommended_hold_hours: 0.0,
        net_bps_at_recommended_hold: 150.0,
        round_trip: Some(RoundTripCostBreakdown {
            long_leg: leg(HedgeLegRole::Long, long_fee),
            short_leg: leg(HedgeLegRole::Short, short_fee),
            open_fee_bps: 20.0,
            close_fee_bps: 20.0,
            open_slippage_bps: 5.0,
            close_slippage_bps: 5.0,
            borrow_or_financing_bps: 0.0,
            funding_window_mismatch_buffer_bps: 0.0,
            min_profit_buffer_bps: 0.0,
            total_cost_bps: 50.0,
            one_cycle_net_bps: 150.0,
            profitability_evidence: ProfitabilityEvidence {
                status: ProfitabilityEvidenceStatus::Partial,
                source: "test".into(),
                observed_at_ms: NOW,
                verified_fee_snapshot_count: 2,
                fee_sources: vec![
                    TradeFeeSource::OfficialSchedule,
                    TradeFeeSource::OfficialSchedule,
                ],
                fee_evidence_ids: vec!["fee-binance".into(), "fee-okx".into()],
                funding_history: None,
                problem: None,
            },
        }),
    }
}

fn leg(role: HedgeLegRole, fee_snapshot: TradeFeeSnapshot) -> LegCostBreakdown {
    LegCostBreakdown {
        role,
        venue: fee_snapshot.venue.clone(),
        symbol: fee_snapshot.symbol.clone(),
        product: fee_snapshot.product,
        open_fee_bps: 10.0,
        close_fee_bps: 10.0,
        open_slippage_bps: 2.5,
        close_slippage_bps: 2.5,
        fee_snapshot: Some(fee_snapshot),
    }
}

fn fee(venue: &str, symbol: &str) -> TradeFeeSnapshot {
    TradeFeeSnapshot {
        venue: venue.into(),
        symbol: symbol.into(),
        product: FeeProduct::Spot,
        account_id: None,
        maker_fee_bps: 2.0,
        taker_fee_bps: 5.0,
        open_fee_bps: 5.0,
        close_fee_bps: 5.0,
        source: TradeFeeSource::OfficialSchedule,
        fetched_at_ms: NOW,
        valid_until_ms: NOW + 60_000,
        freshness_ms: Some(0),
        evidence: Some(FeeScheduleEvidence {
            evidence_id: format!("fee-{venue}"),
            source_name: "official".into(),
            source_url: "https://official.example/fees".into(),
            checked_at_ms: NOW,
            effective_at_ms: None,
            schedule_version: Some("test".into()),
            tier: Some("base".into()),
            scope: Some(symbol.into()),
            problem: None,
        }),
        verification_problem: None,
        note: None,
    }
}

fn market_evidence(venue: &str, symbol: &str) -> OpportunityLegMarketEvidence {
    OpportunityLegMarketEvidence {
        venue: venue.into(),
        symbol: symbol.into(),
        price: Some(10.0),
        health: shared_types::MarketDataHealth {
            quality: shared_types::MarketDataQuality::Fresh,
            source: shared_types::MarketDataSourceKind::WsPush,
            freshness_ms: Some(0),
            retry_after_ms: None,
            last_error: None,
            observed_at_ms: NOW,
            coverage: None,
            problem: None,
        },
    }
}
