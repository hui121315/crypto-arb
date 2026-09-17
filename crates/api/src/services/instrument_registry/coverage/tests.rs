use super::*;
use shared_types::instrument_registry::{InstrumentAssetClass, VenueInstrument};
use shared_types::StrategyKind;

mod cross_spot_perp_transfer;
mod execution_gate;
mod preference;
mod spot_perp_transfer;
mod transfer_loop;

const NOW: i64 = 1_800_000_000_000;

#[test]
fn coverage_surfaces_listed_unlisted_failed_stale_and_unsupported() {
    let registry = InstrumentRegistry::default();
    assert_eq!(
        registry.replace_venue("binance", vec![instrument("binance", "BTCUSDT")]),
        1
    );
    assert_eq!(
        registry.replace_venue("okx", vec![instrument("okx", "ETH-USDT-SWAP")]),
        1
    );
    registry.record_refresh_failure("gate", "gate probe failed");
    registry.record_unsupported("kucoin", "adapter unavailable");
    registry.record_refresh_success("bybit", NOW - INSTRUMENT_SPEC_FRESHNESS_MS - 1);

    let coverage = registry.coverage("BTC-USDT", NOW);
    assert_eq!(state(&coverage, "binance"), VenueListingState::Listed);
    assert_eq!(state(&coverage, "okx"), VenueListingState::Unlisted);
    assert_eq!(state(&coverage, "gate"), VenueListingState::Failed);
    assert_eq!(state(&coverage, "bybit"), VenueListingState::Stale);
    assert_eq!(state(&coverage, "kucoin"), VenueListingState::Unsupported);
}

#[test]
fn diagnostic_projection_keeps_states_sources_and_fail_closed_count() {
    let registry = InstrumentRegistry::default();
    assert_eq!(
        registry.replace_venue("binance", vec![instrument("binance", "BTCUSDT")]),
        1
    );
    registry.record_refresh_success("bybit", NOW - INSTRUMENT_SPEC_FRESHNESS_MS - 1);
    registry.record_refresh_failure("gate", "gate probe failed");

    let diagnostic = registry.coverage_diagnostic("BTC-USDT", NOW);

    assert_eq!(diagnostic.canonical_symbol, "BTC");
    assert_eq!(diagnostic.executable_count, 1);
    assert_eq!(diagnostic.venue_count, SUPPORTED_VENUES.len());
    assert!(!diagnostic.constructible);
    assert!(diagnostic
        .diagnostics_text
        .contains("BINANCE 已挂牌 · 官方端点 · 规格就绪"));
    assert!(diagnostic.diagnostics_text.contains("BYBIT 证据过期"));
    assert!(diagnostic.diagnostics_text.contains("GATE 探测失败"));
}

#[test]
fn listed_but_incomplete_spec_is_visible_and_blocks_scanner() {
    let registry = InstrumentRegistry::default();
    let mut incomplete = instrument("binance", "BTCUSDT");
    incomplete.qty_step = None;
    assert_eq!(registry.replace_venue("binance", vec![incomplete]), 1);

    let coverage = registry.coverage("BTC", NOW);
    let binance = coverage
        .venues
        .iter()
        .find(|entry| entry.venue == "binance")
        .into_iter()
        .collect::<Vec<_>>();
    assert_eq!(binance.len(), 1, "binance coverage must be unique");
    let binance = binance[0];
    assert_eq!(binance.state, VenueListingState::Listed);
    assert!(!binance.execution_ready);
    assert!(!binance.is_executable_leg(NOW));
    assert!(registry
        .hedge_instrument_at_for_product("binance", "BTCUSDT", shared_types::FeeProduct::Perp, NOW,)
        .is_none());

    let mut rows = vec![opportunity()];
    registry.apply_listing_gate(&mut rows, NOW);
    assert!(!rows[0].execution_eligible);
    assert!(rows[0]
        .execution_blockers
        .iter()
        .any(|blocker| blocker.contains("BINANCE 官方已挂牌但执行规格未通过")));
}

#[test]
fn gate_decimal_contract_reports_public_data_ready_and_trade_protocol_pending() {
    let registry = InstrumentRegistry::default();
    let mut gate = instrument("gate", "BTC_USDT");
    gate.execution_supported = false;
    gate.qty_step = None;
    gate.min_qty = Some(0.1);
    assert_eq!(registry.replace_venue("gate", vec![gate]), 1);
    assert_eq!(
        registry.replace_venue("okx", vec![instrument("okx", "BTC-USDT-SWAP")]),
        1
    );
    let mut row = opportunity();
    row.long_exchange = "gate".into();
    let mut rows = vec![row];

    registry.apply_listing_gate(&mut rows, NOW);

    assert!(!rows[0].execution_eligible);
    assert!(rows[0].execution_blockers.iter().any(|blocker| blocker
        .contains("GATE 小数合约公共规格与盘口已核验；下单及私有订单/仓位小数协议尚未验收")));
}

#[test]
fn gate_native_unicode_contract_reports_symbol_protocol_pending() {
    let registry = InstrumentRegistry::default();
    let mut gate = instrument("gate", "BTC_USDT");
    gate.native_symbol = "龙虾_USDT".into();
    gate.display_symbol = "龙虾_USDT".into();
    gate.execution_supported = false;
    assert_eq!(registry.replace_venue("gate", vec![gate]), 1);
    assert_eq!(
        registry.replace_venue("okx", vec![instrument("okx", "BTC-USDT-SWAP")]),
        1
    );
    let mut row = opportunity();
    row.long_exchange = "gate".into();
    let mut rows = vec![row];

    registry.apply_listing_gate(&mut rows, NOW);

    assert!(!rows[0].execution_eligible);
    assert!(rows[0]
        .execution_blockers
        .iter()
        .any(|blocker| blocker.contains(
            "GATE 非 ASCII 原生合约公共挂牌与盘口已核验；下单及私有订单/仓位的符号编码协议尚未验收"
        )));
}

#[test]
fn binance_tradifi_contract_reports_specific_execution_evidence_boundary() {
    let registry = InstrumentRegistry::default();
    let mut binance = instrument("binance", "IWMUSDT");
    binance.asset_class = InstrumentAssetClass::Equity;
    binance.execution_supported = false;
    let mut bybit = instrument("bybit", "IWMUSDT");
    bybit.asset_class = InstrumentAssetClass::Equity;
    assert_eq!(registry.replace_venue("binance", vec![binance]), 1);
    assert_eq!(registry.replace_venue("bybit", vec![bybit]), 1);
    let mut row = opportunity();
    row.symbol = "IWM".into();
    row.short_exchange = "bybit".into();
    let mut rows = vec![row];

    registry.apply_listing_gate(&mut rows, NOW);

    assert!(!rows[0].execution_eligible);
    assert!(rows[0]
        .execution_blockers
        .iter()
        .any(|blocker| blocker.contains(
            "TradFi 永续已在官方 exchangeInfo 挂牌；专用下单、撤单与私有终态语义尚未获得官方证据"
        )));
}

#[test]
fn listing_gate_blocks_missing_leg_evidence() {
    let registry = InstrumentRegistry::default();
    assert_eq!(
        registry.replace_venue("binance", vec![instrument("binance", "BTCUSDT")]),
        1
    );
    let mut rows = vec![opportunity()];
    registry.apply_listing_gate(&mut rows, NOW);
    assert!(!rows[0].execution_eligible);
    assert!(rows[0]
        .execution_blockers
        .iter()
        .any(|blocker| blocker.contains("挂牌证据")
            && blocker.contains("OKX instrument registry 尚未刷新")));
}

fn state(coverage: &InstrumentCoverageEntry, venue: &str) -> VenueListingState {
    coverage
        .venues
        .iter()
        .find(|entry| entry.venue == venue)
        .map(|entry| entry.state)
        .unwrap_or(VenueListingState::Unknown)
}

fn instrument(venue: &str, native_symbol: &str) -> VenueInstrument {
    let evidence = super::super::instrument_metadata_evidence(venue);
    VenueInstrument {
        venue: venue.into(),
        native_symbol: native_symbol.into(),
        canonical_symbol: native_symbol
            .trim_end_matches("-USDT-SWAP")
            .trim_end_matches("_USDT")
            .trim_end_matches("USDT")
            .into(),
        display_symbol: native_symbol.into(),
        asset_class: InstrumentAssetClass::Crypto,
        product_type: Some("perpetual".into()),
        quote_asset: Some("USDT".into()),
        settle_asset: Some("USDT".into()),
        margin_asset: Some("USDT".into()),
        contract_size: Some(1.0),
        execution_supported: true,
        price_tick: Some(0.1),
        qty_step: Some(0.001),
        min_qty: Some(0.001),
        min_notional: None,
        listing_status: InstrumentListingStatus::Trading,
        funding_interval_ms: Some(28_800_000),
        builder_dex: None,
        source: InstrumentMetadataSource::OfficialEndpoint,
        source_url: evidence.map(|row| row.path.clone()),
        checked_at_ms: NOW,
        schema_version: evidence.map(|row| row.doc_version.clone()),
    }
}

fn opportunity() -> ArbitrageOpportunityDto {
    ArbitrageOpportunityDto {
        id: "coverage-test".into(),
        symbol: "BTC".into(),
        arb_type: shared_types::ArbitrageType::CrossExchange,
        type_label: "永续跨所".into(),
        long_exchange: "binance".into(),
        short_exchange: "okx".into(),
        spread_8h: 0.0,
        long_rate_8h: 0.0,
        short_rate_8h: 0.0,
        long_rate: 0.0,
        short_rate: 0.0,
        single_yield: 0.0,
        net_single_yield: 0.1,
        raw_single_yield: 0.0,
        settlement_interval: 8,
        risk_adjusted_yield: 0.0,
        trading_cost_rate: 0.0,
        min_holding_periods: 1,
        risk_level: shared_types::RiskLevel::Low,
        volatility: 0.0,
        sharpe_ratio: 0.0,
        score: 80.0,
        score_breakdown: None,
        ranking_key: None,
        recommendation: shared_types::Recommendation::Hold,
        optimal_position: 100.0,
        max_position: 100.0,
        liquidity_score: 1.0,
        volume_24h: 1.0,
        long_volume_24h: 1.0,
        short_volume_24h: 1.0,
        data_source: "test".into(),
        confidence: 1.0,
        updated_at: chrono::Utc::now(),
        long_funding_interval: 8,
        short_funding_interval: 8,
        settlement_time_diff: false,
        strategy_description: String::new(),
        long_action: "买入".into(),
        short_action: "卖出".into(),
        long_next_funding_time: 0,
        short_next_funding_time: 0,
        time_to_settlement_ms: 0,
        is_snipe_ready: false,
        long_price: Some(1.0),
        short_price: Some(1.1),
        long_leg_market_evidence: None,
        short_leg_market_evidence: None,
        quote_conversions: Vec::new(),
        price_deviation: None,
        basis_spread: None,
        basis_annual_cost: None,
        risk_warnings: Vec::new(),
        execution_eligible: true,
        execution_blockers: Vec::new(),
        execution_cost: None,
        index_composition: None,
        strategy_kind: Some(StrategyKind::PerpCross),
        strategy_category: Some(StrategyKind::PerpCross.category()),
        spot_leg_mode: None,
        basis_bps: None,
        annualized_funding_bps: None,
        triangular_path: None,
        onchain_metadata: None,
        predicted_next_funding: None,
        funding_diff_window: None,
        funding_diff_windows: Vec::new(),
        borrow_cost_bps_per_day: None,
        funding_window_alignment_minutes: None,
        funding_cap_distance_bps: None,
        min_hold_hours: None,
        settlement_countdown_seconds: None,
    }
}
