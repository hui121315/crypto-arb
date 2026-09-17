use super::super::execution_gate::apply as apply_execution_gate;
use super::*;
use shared_types::instrument_registry::InstrumentAssetClass;
use shared_types::{
    IndexCompositionEvidence, IndexCompositionQuality, IndexCompositionRiskProfile,
    IndexCompositionStatus,
};

mod spot;
use spot::market_evidence;

#[test]
fn listing_gate_blocks_same_symbol_with_different_asset_classes() {
    let registry = InstrumentRegistry::default();
    let crypto = instrument("binance", "ONUSDT");
    let mut equity = instrument("bybit", "ONUSDT");
    equity.asset_class = InstrumentAssetClass::Equity;
    assert_eq!(registry.replace_venue("binance", vec![crypto]), 1);
    assert_eq!(registry.replace_venue("bybit", vec![equity]), 1);
    let mut row = opportunity();
    row.symbol = "ON".into();
    row.long_exchange = "bybit".into();
    row.short_exchange = "binance".into();
    let mut rows = vec![row];

    registry.apply_listing_gate(&mut rows, NOW);

    assert!(!rows[0].execution_eligible);
    assert!(rows[0].execution_blockers.iter().any(|blocker| {
        blocker.contains("经济标的身份未通过")
            && blocker.contains("BYBIT=股票")
            && blocker.contains("BINANCE=加密资产")
    }));
}

#[test]
fn listing_gate_blocks_matching_equity_classes_without_underlying_evidence() {
    let registry = InstrumentRegistry::default();
    let mut gate = instrument("gate", "ZHIPU_USDT");
    gate.asset_class = InstrumentAssetClass::Equity;
    let mut hyperliquid = instrument("hyperliquid:xyz", "xyz:ZHIPU");
    hyperliquid.canonical_symbol = "ZHIPU".into();
    hyperliquid.asset_class = InstrumentAssetClass::Equity;
    assert_eq!(registry.replace_venue("gate", vec![gate]), 1);
    assert_eq!(
        registry.replace_venue("hyperliquid:xyz", vec![hyperliquid]),
        1
    );
    let mut row = opportunity();
    row.symbol = "ZHIPU".into();
    row.long_exchange = "gate".into();
    row.short_exchange = "hyperliquid:xyz".into();
    row.long_price = Some(117.42);
    row.short_price = Some(116.49);
    let mut rows = vec![row];

    registry.apply_listing_gate(&mut rows, NOW);

    assert!(!rows[0].execution_eligible);
    assert!(rows[0]
        .execution_blockers
        .iter()
        .any(|blocker| blocker.contains("非加密资产缺少双边官方底层成分证据")));
}

#[test]
fn listing_gate_allows_matching_equity_with_verified_underlying_evidence() {
    let registry = InstrumentRegistry::default();
    let mut gate = instrument("gate", "ZHIPU_USDT");
    gate.asset_class = InstrumentAssetClass::Equity;
    let mut hyperliquid = instrument("hyperliquid:xyz", "xyz:ZHIPU");
    hyperliquid.canonical_symbol = "ZHIPU".into();
    hyperliquid.asset_class = InstrumentAssetClass::Equity;
    assert_eq!(registry.replace_venue("gate", vec![gate]), 1);
    assert_eq!(
        registry.replace_venue("hyperliquid:xyz", vec![hyperliquid]),
        1
    );
    let mut row = opportunity();
    row.symbol = "ZHIPU".into();
    row.long_exchange = "gate".into();
    row.short_exchange = "hyperliquid:xyz".into();
    row.long_price = Some(117.42);
    row.short_price = Some(116.49);
    row.index_composition = Some(verified_underlying_identity());
    let mut rows = vec![row];

    registry.apply_listing_gate(&mut rows, NOW);

    assert!(rows[0].execution_eligible);
    assert!(rows[0]
        .execution_blockers
        .iter()
        .all(|blocker| !blocker.contains("经济标的身份未通过")));
}

#[test]
fn perp_cross_blocks_cross_quote_contracts_without_fx_hedge() {
    let registry = InstrumentRegistry::default();
    assert_eq!(
        registry.replace_venue("binance", vec![instrument("binance", "BTCUSDT")]),
        1
    );
    let mut hyperliquid = instrument("hyperliquid:xyz", "xyz:BTC");
    hyperliquid.canonical_symbol = "BTC".into();
    hyperliquid.quote_asset = Some("USDC".into());
    hyperliquid.settle_asset = Some("USDC".into());
    hyperliquid.margin_asset = Some("USDC".into());
    assert_eq!(
        registry.replace_venue("hyperliquid:xyz", vec![hyperliquid]),
        1
    );
    let mut row = opportunity();
    row.long_exchange = "binance".into();
    row.short_exchange = "hyperliquid:xyz".into();
    row.long_price = Some(100.0);
    row.short_price = Some(100.01);
    let mut rows = vec![row];

    registry.apply_listing_gate(&mut rows, NOW);

    assert!(!rows[0].execution_eligible);
    assert!(rows[0].execution_blockers.iter().any(|blocker| {
        blocker.contains("BINANCE=USDT")
            && blocker.contains("HYPERLIQUID:XYZ=USDC")
            && blocker.contains("报价币风险对冲")
    }));
}

#[test]
fn perp_cross_base_discovery_prefers_the_verified_usdt_contract() {
    let registry = InstrumentRegistry::default();
    assert_eq!(
        registry.replace_venue("binance", vec![instrument("binance", "BTCUSDT")]),
        1
    );
    let usdt = instrument("bybit", "BTCUSDT");
    let mut usdc = instrument("bybit", "BTCUSDT");
    usdc.native_symbol = "BTCUSDC".into();
    usdc.display_symbol = "BTCUSDC".into();
    usdc.quote_asset = Some("USDC".into());
    usdc.settle_asset = Some("USDC".into());
    usdc.margin_asset = Some("USDC".into());
    assert_eq!(registry.replace_venue("bybit", vec![usdt, usdc]), 2);
    let mut row = opportunity();
    row.long_exchange = "binance".into();
    row.short_exchange = "bybit".into();
    row.long_leg_market_evidence = Some(market_evidence("binance", "BTC"));
    row.short_leg_market_evidence = Some(market_evidence("bybit", "BTC"));
    let mut rows = vec![row];

    registry.apply_listing_gate(&mut rows, NOW);

    assert!(rows[0].execution_eligible);
    assert!(rows[0].execution_blockers.is_empty());
}

#[test]
fn listing_gate_blocks_incompatible_price_units() {
    let registry = InstrumentRegistry::default();
    assert_eq!(
        registry.replace_venue("binance", vec![instrument("binance", "ONUSDT")]),
        1
    );
    assert_eq!(
        registry.replace_venue("bybit", vec![instrument("bybit", "ONUSDT")]),
        1
    );
    let mut row = opportunity();
    row.symbol = "ON".into();
    row.long_exchange = "bybit".into();
    row.short_exchange = "binance".into();
    row.long_price = Some(89.92);
    row.short_price = Some(0.13309);
    let mut rows = vec![row];

    registry.apply_listing_gate(&mut rows, NOW);

    assert!(!rows[0].execution_eligible);
    assert!(rows[0].execution_blockers.iter().any(|blocker| {
        blocker.contains("经济标的身份未通过")
            && blocker.contains("价格尺度")
            && blocker.contains("同名异资产")
    }));
}

#[test]
fn spot_strategy_requires_official_spot_execution_specs() {
    let registry = InstrumentRegistry::default();
    assert_eq!(
        registry.replace_venue("binance", vec![instrument("binance", "BTCUSDT")]),
        1
    );
    assert_eq!(
        registry.replace_venue("okx", vec![instrument("okx", "BTC-USDT-SWAP")]),
        1
    );
    let mut row = opportunity();
    row.strategy_kind = Some(StrategyKind::SpotCross);
    row.strategy_category = Some(StrategyKind::SpotCross.category());
    row.execution_blockers.push("缺盘口深度".into());
    let mut rows = vec![row];

    registry.apply_listing_gate(&mut rows, NOW);

    assert!(!rows[0].execution_eligible);
    assert!(rows[0].execution_blockers[0].contains("缺少官方现货执行规格"));
    assert!(rows[0].execution_blockers.iter().any(|blocker| {
        blocker.contains("BINANCE 缺少官方现货执行规格")
            && blocker.contains("OKX 缺少官方现货执行规格")
    }));
}

fn verified_underlying_identity() -> IndexCompositionRiskProfile {
    IndexCompositionRiskProfile {
        status: IndexCompositionStatus::Verified,
        overlap_score: 1.0,
        long_quality: IndexCompositionQuality::Verified,
        short_quality: IndexCompositionQuality::Verified,
        blocker: None,
        long_evidence: Some(composition_evidence("gate")),
        short_evidence: Some(composition_evidence("hyperliquid:xyz")),
    }
}

fn composition_evidence(venue: &str) -> IndexCompositionEvidence {
    IndexCompositionEvidence {
        source: format!("{venue} official index endpoint"),
        received_at_ms: NOW,
        freshness_ms: Some(0),
        source_url: Some(format!("https://example.com/{venue}/index")),
        payload_sha256: Some("sha256:test".into()),
        schema_version: Some("test-v1".into()),
    }
}
