use super::*;
use crate::adapters::okx_response::OkxResponse;
use pretty_assertions::assert_eq;
use shared_types::{ExecutionMode, MarginMode, OrderSide, OrderSource, TimeInForce};

#[test]
fn sizing_converts_base_quantity_to_contract_count() {
    let rule = btc_rule();
    let sizing = sizing_from_instrument(&intent(OrderType::Limit, 0.02, Some(50_000.1)), &rule)
        .expect("sizing");

    assert_eq!(
        sizing,
        OkxOrderSizing {
            sz: "2".into(),
            px: Some("50000.1".into()),
        }
    );
}

#[test]
fn market_sizing_omits_price_but_checks_contract_count() {
    let rule = btc_rule();
    let sizing = sizing_from_instrument(&intent(OrderType::Market, 0.03, Some(50_000.1)), &rule)
        .expect("market sizing");

    assert_eq!(sizing.sz, "3");
    assert_eq!(sizing.px, None);
}

#[test]
fn sizing_rejects_fractional_contract_count() {
    let rule = btc_rule();
    let err = sizing_from_instrument(&intent(OrderType::Limit, 0.015, Some(50_000.1)), &rule)
        .expect_err("fractional contract");

    assert!(err.to_string().contains("lotSz"));
}

#[test]
fn sizing_rejects_price_not_aligned_to_tick() {
    let rule = btc_rule();
    let err = sizing_from_instrument(&intent(OrderType::Limit, 0.02, Some(50_000.15)), &rule)
        .expect_err("bad tick");

    assert!(err.to_string().contains("tickSz"));
}

#[test]
fn sizing_rejects_non_live_instrument_state() {
    let rule = OkxInstrumentRule::from_row(OkxInstrumentRow {
        state: "suspend".into(),
        ..btc_row()
    })
    .expect("rule parses");
    let err = sizing_from_instrument(&intent(OrderType::Limit, 0.02, Some(50_000.1)), &rule)
        .expect_err("not live");

    assert!(err.to_string().contains("not live"));
}

#[test]
fn instrument_parser_rejects_contract_currency_mismatch() {
    let err = OkxInstrumentRule::from_row(OkxInstrumentRow {
        contract_value_currency: "USDT".into(),
        ..btc_row()
    })
    .expect_err("ctValCcy mismatch");

    assert!(err.to_string().contains("ctValCcy"));
}

#[test]
fn okx_instrument_rule_parses_official_swap_fixture() {
    let fixture = include_str!("../../fixtures/okx/public_instruments_swap.json");
    let wrap: OkxResponse<OkxInstrumentRow> =
        serde_json::from_str(fixture).expect("okx instruments fixture");
    let mut rows = wrap.into_data("instruments").expect("instrument rows");
    let rule = OkxInstrumentRule::from_row(rows.pop().expect("instrument row")).expect("rule");

    assert_eq!(rule.inst_id, "BTC-USDT-SWAP");
    assert_eq!(rule.ws_inst_id_code().expect("instIdCode"), 123_456);
    assert_eq!(rule.contract_value, 0.01);
    assert_eq!(rule.contract_value_currency, "BTC");
    assert_eq!(rule.lot_size, 0.01);
    assert_eq!(rule.min_size, 0.01);
    assert_eq!(rule.tick_size, 0.1);
    assert_eq!(rule.state, "live");
}

fn btc_rule() -> OkxInstrumentRule {
    OkxInstrumentRule::from_row(btc_row()).expect("btc rule")
}

fn btc_row() -> OkxInstrumentRow {
    OkxInstrumentRow {
        inst_id: "BTC-USDT-SWAP".into(),
        inst_id_code: Some(123_456),
        contract_value: "0.01".into(),
        contract_value_currency: "BTC".into(),
        lot_size: "1".into(),
        min_size: "1".into(),
        tick_size: "0.1".into(),
        state: "live".into(),
    }
}

#[test]
fn ws_inst_id_code_fails_closed_when_missing() {
    let rule = OkxInstrumentRule::from_row(OkxInstrumentRow {
        inst_id_code: None,
        ..btc_row()
    })
    .expect("rule parses");
    let err = rule.ws_inst_id_code().expect_err("missing instIdCode");

    assert!(err.to_string().contains("instIdCode"));
}

#[test]
fn into_venue_instrument_maps_min_qty_from_min_sz() {
    let inst = btc_rule().into_venue_instrument(1_700_000_000_000);

    assert_eq!(inst.venue, "okx");
    assert_eq!(inst.native_symbol, "BTC-USDT-SWAP");
    assert_eq!(inst.canonical_symbol, "BTC");
    assert_eq!(inst.quote_asset.as_deref(), Some("USDT"));
    assert_eq!(inst.contract_size, Some(0.01));
    assert_eq!(inst.price_tick, Some(0.1));
    assert_eq!(inst.qty_step, Some(1.0));
    // OKX 官方不给最小名义：min_qty 来自 minSz，min_notional 必为空。
    assert_eq!(inst.min_qty, Some(1.0));
    assert_eq!(inst.min_notional, None);
    assert_eq!(inst.listing_status, InstrumentListingStatus::Trading);
    assert_eq!(inst.source, InstrumentMetadataSource::OfficialEndpoint);
    assert_eq!(
        inst.schema_version.as_deref(),
        Some(INSTRUMENT_SCHEMA_VERSION)
    );
    // 仅 min_qty 的 OKX 条目仍可构建对冲（泛化契约）。
    assert!(inst.is_hedge_constructible());
}

#[test]
fn instruments_from_rows_skips_contract_currency_mismatch() {
    let rows = vec![
        btc_row(),
        OkxInstrumentRow {
            inst_id: "USDC-USD-SWAP".into(),
            contract_value_currency: "USD".into(),
            ..btc_row()
        },
    ];
    let mapped = instruments_from_rows(rows, 1);

    // 币本位/不匹配 ctValCcy 的行 fail-closed 跳过，只留可安全 sizing 的 SWAP。
    assert_eq!(mapped.len(), 1);
    assert_eq!(mapped[0].native_symbol, "BTC-USDT-SWAP");
}

#[test]
fn suspended_instrument_maps_to_non_constructible() {
    let rule = OkxInstrumentRule::from_row(OkxInstrumentRow {
        state: "suspend".into(),
        ..btc_row()
    })
    .expect("rule parses");
    let inst = rule.into_venue_instrument(1);

    assert_eq!(inst.listing_status, InstrumentListingStatus::Suspended);
    // 非 Trading 状态绝不可下单：构建闸门 fail-closed。
    assert!(!inst.is_hedge_constructible());
}

fn intent(order_type: OrderType, quantity: f64, price: Option<f64>) -> OrderIntent {
    OrderIntent {
        id: "i1".into(),
        source: OrderSource::ArbitragePreview,
        strategy: None,
        mode: ExecutionMode::Live,
        exchange: "okx".into(),
        symbol: "BTC".into(),
        side: OrderSide::Buy,
        order_type,
        quantity,
        price,
        slippage_tolerance_bps: None,
        reduce_only: false,
        time_in_force: TimeInForce::Gtc,
        post_only: false,
        margin_mode: MarginMode::Cross,
        leverage: 1.0,
        client_order_id: "cid".into(),
        client_order_id_policy: None,
        created_at_ms: 1,
    }
}
