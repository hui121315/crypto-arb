use super::*;
use shared_types::{OpportunityLegMarketEvidence, SpotLegMode};

#[test]
fn spot_cross_binds_the_exact_ws_pair_when_multiple_quotes_exist() {
    let registry = InstrumentRegistry::default();
    for row in [
        spot_instrument("binance", "BTCUSDT", "USDT"),
        spot_instrument("binance", "BTCUSDC", "USDC"),
        spot_instrument("okx", "BTC-USDT", "USDT"),
        spot_instrument("okx", "BTC-USDC", "USDC"),
    ] {
        assert!(registry.upsert(row).is_ok());
    }
    let mut row = opportunity();
    row.strategy_kind = Some(StrategyKind::SpotCross);
    row.strategy_category = Some(StrategyKind::SpotCross.category());
    row.long_price = Some(100.0);
    row.short_price = Some(100.01);
    row.long_leg_market_evidence = Some(market_evidence("binance", "BTC/USDC"));
    row.short_leg_market_evidence = Some(market_evidence("okx", "BTC-USDC"));
    let mut rows = vec![row];

    apply_execution_gate(&registry, &mut rows, NOW);

    assert!(rows[0].execution_eligible);
    assert!(rows[0].execution_blockers.is_empty());
}

#[test]
fn spot_perp_listing_gate_follows_inventory_sale_orientation() {
    let registry = InstrumentRegistry::default();
    assert_eq!(
        registry.replace_venue("binance", vec![instrument("binance", "BTCUSDT")]),
        1
    );
    let mut okx_spot = instrument("okx", "BTC-USDT");
    okx_spot.canonical_symbol = "BTC".into();
    okx_spot.product_type = Some("spot".into());
    assert_eq!(registry.replace_venue("okx", vec![okx_spot]), 1);
    let mut row = opportunity();
    row.strategy_kind = Some(StrategyKind::SpotPerp);
    row.strategy_category = Some(StrategyKind::SpotPerp.category());
    row.spot_leg_mode = Some(SpotLegMode::SellInventory);
    row.long_exchange = "binance".into();
    row.short_exchange = "okx".into();
    let mut rows = vec![row];

    registry.apply_listing_gate(&mut rows, NOW);

    assert!(rows[0].execution_eligible);
    assert!(rows[0].execution_blockers.is_empty());
}

#[test]
fn spot_perp_listing_gate_rejects_missing_leg_orientation() {
    let registry = InstrumentRegistry::default();
    let mut row = opportunity();
    row.strategy_kind = Some(StrategyKind::CrossSpotPerp);
    row.strategy_category = Some(StrategyKind::CrossSpotPerp.category());
    row.spot_leg_mode = None;
    let mut rows = vec![row];

    registry.apply_listing_gate(&mut rows, NOW);

    assert!(!rows[0].execution_eligible);
    assert!(rows[0]
        .execution_blockers
        .iter()
        .any(|blocker| blocker.contains("现货腿方向证据缺失")));
}

fn spot_instrument(venue: &str, native_symbol: &str, quote: &str) -> VenueInstrument {
    let mut row = instrument(venue, native_symbol);
    row.canonical_symbol = "BTC".to_owned();
    row.display_symbol = format!("BTC/{quote} Spot");
    row.product_type = Some("spot".to_owned());
    row.quote_asset = Some(quote.to_owned());
    row.settle_asset = None;
    row.margin_asset = None;
    row
}

pub(super) fn market_evidence(venue: &str, symbol: &str) -> OpportunityLegMarketEvidence {
    OpportunityLegMarketEvidence {
        venue: venue.to_owned(),
        symbol: symbol.to_owned(),
        price: Some(100.0),
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
