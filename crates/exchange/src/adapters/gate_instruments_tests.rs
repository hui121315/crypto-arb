use super::*;
use pretty_assertions::assert_eq;

fn btc_row() -> GateContractRow {
    GateContractRow {
        name: "BTC_USDT".into(),
        quanto_multiplier: "0.0001".into(),
        order_price_round: "0.1".into(),
        order_size_min: 1,
        funding_interval: 28800,
        in_delisting: false,
        status: "trading".into(),
    }
}

#[test]
fn gate_instrument_rule_parses_official_contract_fixture() {
    let fixture = include_str!("../../fixtures/gate/futures_usdt_contracts_btc_usdt.json");
    let mut rows: Vec<GateContractRow> =
        serde_json::from_str(fixture).expect("gate contracts fixture");
    let rule = GateInstrumentRule::from_row(rows.pop().expect("contract row")).expect("rule");

    assert_eq!(rule.name, "BTC_USDT");
    assert_eq!(rule.base, "BTC");
    assert_eq!(rule.contract_size, 0.0001);
    assert_eq!(rule.price_tick, 0.1);
    assert_eq!(rule.min_qty, 1.0);
    assert_eq!(rule.funding_interval_ms, Some(28_800_000));
}

#[test]
fn into_venue_instrument_maps_quanto_and_min_qty() {
    let inst = GateInstrumentRule::from_row(btc_row())
        .expect("rule")
        .into_venue_instrument(1_700_000_000_000);

    assert_eq!(inst.venue, "gate");
    assert_eq!(inst.native_symbol, "BTC_USDT");
    assert_eq!(inst.canonical_symbol, "BTC");
    assert_eq!(inst.quote_asset.as_deref(), Some("USDT"));
    assert_eq!(inst.settle_asset.as_deref(), Some("USDT"));
    assert_eq!(inst.contract_size, Some(0.0001));
    assert_eq!(inst.price_tick, Some(0.1));
    // Gate 以整数张数下单：步进恒为 1 张，min_qty 来自 order_size_min。
    assert_eq!(inst.qty_step, Some(1.0));
    assert_eq!(inst.min_qty, Some(1.0));
    // Gate 官方不给最小名义。
    assert_eq!(inst.min_notional, None);
    assert_eq!(inst.funding_interval_ms, Some(28_800_000));
    assert_eq!(inst.listing_status, InstrumentListingStatus::Trading);
    assert_eq!(inst.source, InstrumentMetadataSource::OfficialEndpoint);
    assert!(inst.is_hedge_constructible());
}

#[test]
fn instrument_parser_rejects_non_usdt_settle() {
    let err = GateInstrumentRule::from_row(GateContractRow {
        name: "BTC_USD".into(),
        ..btc_row()
    })
    .expect_err("non-usdt settle");

    assert!(err.to_string().contains("is not USDT"));
}

#[test]
fn instrument_parser_rejects_non_positive_min_size() {
    let err = GateInstrumentRule::from_row(GateContractRow {
        order_size_min: 0,
        ..btc_row()
    })
    .expect_err("zero min size");

    assert!(err.to_string().contains("order_size_min"));
}

#[test]
fn instruments_from_rows_skips_invalid_and_keeps_usdt_perp() {
    let rows = vec![
        btc_row(),
        GateContractRow {
            name: "ETH_USD".into(),
            ..btc_row()
        },
        GateContractRow {
            name: "SOL_USDT".into(),
            quanto_multiplier: "bad".into(),
            ..btc_row()
        },
    ];
    let mapped = instruments_from_rows(rows, 1);

    assert_eq!(mapped.len(), 1);
    assert_eq!(mapped[0].native_symbol, "BTC_USDT");
}

#[test]
fn delisting_contract_maps_to_non_constructible() {
    let inst = GateInstrumentRule::from_row(GateContractRow {
        in_delisting: true,
        ..btc_row()
    })
    .expect("rule")
    .into_venue_instrument(1);

    assert_eq!(inst.listing_status, InstrumentListingStatus::Delisted);
    assert!(!inst.is_hedge_constructible());
}
