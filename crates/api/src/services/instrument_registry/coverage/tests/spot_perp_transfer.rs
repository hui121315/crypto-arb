use super::*;
use crate::services::instrument_registry::CandidateTransferStatus;
use exchange::CurrencyTransferNetwork;
use rust_decimal::Decimal;
use shared_types::{
    ArbitrageType, ExecutionCostProfile, FeeProduct, FeeScheduleEvidence, HedgeLegRole,
    LegCostBreakdown, OneCycleCostProfile, OpportunityLegMarketEvidence, ProfitabilityEvidence,
    ProfitabilityEvidenceStatus, RoundTripCostBreakdown, SpotLegMode, TradeFeeSnapshot,
    TradeFeeSource,
};

#[test]
fn spot_perp_requests_mobility_only_after_a_profitable_ws_candidate() {
    let registry = listing_registry();
    let mut row = spot_perp_opportunity();

    registry.apply_listing_gate(std::slice::from_mut(&mut row), NOW);

    assert!(row
        .execution_blockers
        .iter()
        .any(|blocker| blocker.starts_with("现货-永续充提状态未通过：")));
    assert!(registry.should_refresh_transfer_for_candidate(&row, NOW));
    assert!(matches!(
        registry.candidate_transfer_status(&row, NOW),
        CandidateTransferStatus::Warming { .. }
    ));

    let mut no_profit = spot_perp_opportunity();
    no_profit.net_single_yield = 0.0;
    if let Some(cost) = no_profit.execution_cost.as_mut() {
        cost.one_cycle.net_bps = 0.0;
        cost.one_cycle.covers_round_trip_cost = false;
    }
    registry.apply_listing_gate(std::slice::from_mut(&mut no_profit), NOW);
    assert!(!registry.should_refresh_transfer_for_candidate(&no_profit, NOW));
    assert!(!no_profit
        .execution_blockers
        .iter()
        .any(|blocker| blocker.starts_with("现货-永续充提状态未通过：")));
}

#[test]
fn spot_perp_requires_base_and_quote_mobility_before_execution() {
    let registry = ready_registry();
    let mut row = spot_perp_opportunity();

    registry.apply_listing_gate(std::slice::from_mut(&mut row), NOW);

    assert!(row.execution_eligible, "{:?}", row.execution_blockers);
    assert!(row
        .risk_warnings
        .iter()
        .any(|warning| warning.contains("Base 经 bitcoin 可充可提")));
    assert!(matches!(
        registry.candidate_transfer_status(&row, NOW),
        CandidateTransferStatus::Available { .. }
    ));
    assert_eq!(automation::select_monitor_candidates([&row], NOW).len(), 1);

    assert_eq!(
        registry.replace_transfer_venue(
            "binance",
            vec![
                transfer("binance", "BTC", "BTC", true, false),
                transfer("binance", "USDT", "TRC20", true, true),
            ],
        ),
        2
    );
    let mut blocked = spot_perp_opportunity();
    registry.apply_listing_gate(std::slice::from_mut(&mut blocked), NOW);
    assert!(!blocked.execution_eligible);
    assert!(blocked
        .execution_blockers
        .iter()
        .any(|blocker| blocker.contains("当前没有同时开放充值和提币的网络")));
    assert_eq!(
        automation::select_monitor_candidates([&blocked], NOW).len(),
        1
    );
}

fn listing_registry() -> InstrumentRegistry {
    let registry = InstrumentRegistry::default();
    assert_eq!(
        registry.replace_venue(
            "binance",
            vec![
                spot_instrument("binance", "BTCUSDT"),
                instrument("binance", "BTCUSDT"),
            ],
        ),
        2
    );
    registry
}

fn ready_registry() -> InstrumentRegistry {
    let registry = listing_registry();
    assert_eq!(
        registry.replace_transfer_venue(
            "binance",
            vec![
                transfer("binance", "BTC", "BTC", true, true),
                transfer("binance", "USDT", "TRC20", true, true),
            ],
        ),
        2
    );
    registry
}

fn spot_instrument(venue: &str, native_symbol: &str) -> VenueInstrument {
    let mut row = instrument(venue, native_symbol);
    row.canonical_symbol = "BTC".into();
    row.product_type = Some("spot".into());
    row.quote_asset = Some("USDT".into());
    row.settle_asset = None;
    row.margin_asset = None;
    row.funding_interval_ms = None;
    row
}

fn spot_perp_opportunity() -> ArbitrageOpportunityDto {
    let mut row = opportunity();
    row.id = "spot-perp-mobility".into();
    row.arb_type = ArbitrageType::SpotFutures;
    row.type_label = "现货-永续".into();
    row.strategy_kind = Some(StrategyKind::SpotPerp);
    row.strategy_category = Some(StrategyKind::SpotPerp.category());
    row.spot_leg_mode = Some(SpotLegMode::BuySpot);
    row.long_exchange = "binance".into();
    row.short_exchange = "binance".into();
    row.long_price = Some(10.0);
    row.short_price = Some(10.2);
    row.net_single_yield = 0.015;
    row.long_leg_market_evidence = Some(market_evidence("binance", "BTCUSDT"));
    row.short_leg_market_evidence = Some(market_evidence("binance", "BTCUSDT"));
    row.execution_cost = Some(verified_cost());
    row
}

fn verified_cost() -> ExecutionCostProfile {
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
        recommended_hold_hours: 1.0,
        net_bps_at_recommended_hold: 150.0,
        round_trip: Some(RoundTripCostBreakdown {
            long_leg: leg(HedgeLegRole::Long, fee(FeeProduct::Spot)),
            short_leg: leg(HedgeLegRole::Short, fee(FeeProduct::Perp)),
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
                fee_evidence_ids: vec!["fee-spot".into(), "fee-perp".into()],
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

fn fee(product: FeeProduct) -> TradeFeeSnapshot {
    let product_label = match product {
        FeeProduct::Spot => "spot",
        FeeProduct::Perp => "perp",
        FeeProduct::Margin => "margin",
        FeeProduct::Unknown => "unknown",
    };
    TradeFeeSnapshot {
        venue: "binance".into(),
        symbol: "BTCUSDT".into(),
        product,
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
            evidence_id: format!("fee-{product_label}"),
            source_name: "official".into(),
            source_url: "https://official.example/fees".into(),
            checked_at_ms: NOW,
            effective_at_ms: None,
            schedule_version: Some("test".into()),
            tier: Some("base".into()),
            scope: Some("BTCUSDT".into()),
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

fn transfer(
    venue: &str,
    currency: &str,
    network: &str,
    deposit_enabled: bool,
    withdraw_enabled: bool,
) -> CurrencyTransferNetwork {
    CurrencyTransferNetwork {
        venue: venue.into(),
        currency: currency.into(),
        network: network.into(),
        canonical_network: exchange::canonical_network_id(network),
        contract_address: None,
        deposit_enabled,
        withdraw_enabled,
        withdrawal_fee: withdraw_enabled.then_some(Decimal::new(1, 2)),
        withdrawal_fee_rate: withdraw_enabled.then_some(Decimal::ZERO),
        withdrawal_step: withdraw_enabled.then_some(Decimal::new(1, 6)),
        min_withdraw: withdraw_enabled.then_some(Decimal::new(1, 2)),
        min_deposit: deposit_enabled.then_some(Decimal::ZERO),
        requires_tag: false,
        credit_confirmations: None,
        unlock_confirmations: None,
        network_status: None,
        checked_at_ms: NOW,
        source_url: "https://official.example/transfer-networks".into(),
    }
}
