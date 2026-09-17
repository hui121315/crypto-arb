use super::*;
use crate::adapters::bybit_response::BybitResponse;
use serde::Deserialize;

fn ticker(symbol: &str) -> MarketTickerItem {
    MarketTickerItem {
        symbol: symbol.into(),
        last_price: "30000".into(),
        bid1_price: "29999".into(),
        bid1_size: "1.0".into(),
        ask1_price: "30001".into(),
        ask1_size: "1.2".into(),
        turnover24h: "1000000".into(),
        funding_rate: "0.0001".into(),
        next_funding_time: "1700028800000".into(),
        funding_interval_hour: "8".into(),
        mark_price: "30001".into(),
        index_price: "30000".into(),
        open_interest: "1234".into(),
        open_interest_value: "37020000".into(),
    }
}

#[test]
fn funding_interval_minutes_to_hours_handles_common_cases() {
    assert_eq!(funding_interval_minutes_to_hours(60), 1);
    assert_eq!(funding_interval_minutes_to_hours(240), 4);
    assert_eq!(funding_interval_minutes_to_hours(480), 8);
}

#[test]
fn funding_interval_minutes_to_hours_rounds_non_60_multiples() {
    assert_eq!(funding_interval_minutes_to_hours(30), 1);
    assert_eq!(funding_interval_minutes_to_hours(90), 2);
    assert_eq!(funding_interval_minutes_to_hours(150), 3);
}

#[test]
fn funding_interval_minutes_to_hours_handles_negative_and_zero() {
    assert_eq!(funding_interval_minutes_to_hours(0), 8);
    assert_eq!(funding_interval_minutes_to_hours(-1), 8);
    assert_eq!(funding_interval_minutes_to_hours(-9999), 8);
}

#[test]
fn funding_interval_minutes_to_hours_clamps_to_24h() {
    assert_eq!(funding_interval_minutes_to_hours(1500), 24);
}

#[test]
fn is_usdm_perp_accepts_usdt_and_usdc() {
    assert!(is_usdm_perp("BTCUSDT"));
    assert!(is_usdm_perp("ETHUSDC"));
    assert!(is_usdm_perp("BTCPERP"));
    assert!(!is_usdm_perp("BTCBUSD"));
    assert!(!is_usdm_perp("BTC"));
}

#[test]
fn linear_stream_symbol_keeps_official_usdc_native_forms() {
    assert_eq!(linear_stream_symbol("BTC"), "BTCUSDT");
    assert_eq!(linear_stream_symbol("BTC-USDT"), "BTCUSDT");
    assert_eq!(linear_stream_symbol("BTCUSDC"), "BTCUSDC");
    assert_eq!(linear_stream_symbol("BTC-USDC-SWAP"), "BTCUSDC");
    assert_eq!(linear_stream_symbol("BTCPERP"), "BTCPERP");
}

#[test]
fn clamp_orderbook_limit_respects_category_caps() {
    assert_eq!(clamp_orderbook_limit("linear", 500), 500);
    assert_eq!(clamp_orderbook_limit("linear", 1000), 1000);
    assert_eq!(clamp_orderbook_limit("spot", 1000), 1000);
    assert_eq!(clamp_orderbook_limit("option", 100), 25);
    assert_eq!(clamp_orderbook_limit("inverse", 1000), 1000);
    assert_eq!(clamp_orderbook_limit("unknown", 1000), 1000);
    assert_eq!(clamp_orderbook_limit("linear", 0), 1);
}

#[test]
fn parse_funding_8h_no_op() {
    let funding = parse_funding(&ticker("BTCUSDT"), 8, 1_000_000.0, 1_700_000_000_000)
        .expect("funding parses");
    assert_eq!(funding.symbol, "BTC");
    assert_eq!(funding.exchange, "bybit");
    assert!((funding.rate - 0.0001).abs() < 1e-12);
    assert!((funding.rate_8h - 0.0001).abs() < 1e-12);
    assert_eq!(funding.funding_interval, 8);
    assert_eq!(funding.timestamp, 1_700_000_000_000);
}

#[test]
fn parse_funding_4h_normalized() {
    let funding = parse_funding(&ticker("ETHUSDT"), 4, 0.0, 0).expect("funding parses");
    assert!((funding.rate_8h - 0.0002).abs() < 1e-12);
    assert_eq!(funding.funding_interval, 4);
    assert!(funding.timestamp > 0);
}

#[test]
fn parse_spot_tick_pair_symbol() {
    let tick = parse_spot_tick(&ticker("SOLUSDT"), 1_700_000_000_000).expect("spot tick parses");
    assert_eq!(tick.venue, "bybit");
    assert_eq!(tick.symbol, "SOL/USDT");
    assert_eq!(tick.exchange_ts_ms, Some(1_700_000_000_000));
    assert!(tick.received_at_ms > 0);
}

#[test]
fn parse_mark_index_uses_official_ticker_fields() {
    let row = parse_mark_index(&ticker("BTCUSDT"), 1_700_000_000_000).unwrap();
    assert_eq!(row.symbol, "BTC");
    assert_eq!(row.exchange, "bybit");
    assert_eq!(row.mark_price, 30001.0);
    assert_eq!(row.index_price, Some(30000.0));
    assert_eq!(row.open_interest, Some(1234.0));
    assert_eq!(row.open_interest_value, Some(37020000.0));
    assert_eq!(row.timestamp, 1_700_000_000_000);
}

#[test]
fn bybit_market_tickers_official_fixture_closes_perp_funding_spot_debt() {
    let fixture = include_str!("../../fixtures/bybit/market_tickers_linear_spot_btcusdt.json");
    let fixture: BybitMarketTickersFixture =
        serde_json::from_str(fixture).expect("bybit market tickers fixture");

    let (linear_rows, server_time_ms) = fixture
        .linear
        .into_list_with_time("linear market tickers")
        .expect("linear ticker list");
    let linear = linear_rows.first().expect("linear ticker row");

    let volume_24h = assert_bybit_perp_ticker(linear);
    assert_bybit_funding(linear, volume_24h, server_time_ms);

    let (spot_rows, spot_time_ms) = fixture
        .spot
        .into_list_with_time("spot tickers")
        .expect("spot ticker list");
    assert_bybit_spot_ticker(spot_rows.first().expect("spot ticker row"), spot_time_ms);
}

fn assert_bybit_perp_ticker(linear: &MarketTickerItem) -> f64 {
    let perp = parse_ticker(linear).expect("ticker parses");
    assert_eq!(perp.symbol, "BTC");
    assert_eq!(perp.exchange, "bybit");
    assert_eq!(perp.bid, 66931.20);
    assert_eq!(perp.ask, 66931.30);
    assert_eq!(perp.last, 66931.30);
    assert_eq!(perp.volume_24h, 10198855851.0186);
    perp.volume_24h
}

fn assert_bybit_funding(linear: &MarketTickerItem, volume_24h: f64, server_time_ms: i64) {
    let funding = parse_ws_funding(linear, volume_24h, server_time_ms).expect("funding parses");
    assert_eq!(funding.symbol, "BTC");
    assert_eq!(funding.exchange, "bybit");
    assert!((funding.rate - -0.00000183).abs() < 1e-12);
    assert!((funding.rate_8h - -0.00000183).abs() < 1e-12);
    assert_eq!(funding.next_funding_time, 1_780_444_800_000);
    assert_eq!(funding.funding_interval, 8);
    assert_eq!(funding.timestamp, 1_780_440_271_825);
}

fn assert_bybit_spot_ticker(row: &MarketTickerItem, server_time_ms: i64) {
    let spot = parse_spot_tick(row, server_time_ms).expect("spot tick parses");
    assert_eq!(spot.venue, "bybit");
    assert_eq!(spot.symbol, "BTC/USDT");
    assert_eq!(spot.bid.to_string(), "66955.6");
    assert_eq!(spot.ask.to_string(), "66955.7");
    assert_eq!(spot.last.to_string(), "66955.7");
    assert_eq!(spot.bid_size.unwrap().to_string(), "0.891");
    assert_eq!(spot.ask_size.unwrap().to_string(), "0.455514");
    assert_eq!(spot.volume_24h.to_string(), "1149023986.52650607");
    assert_eq!(spot.exchange_ts_ms, Some(1_780_440_271_831));
    assert!(spot.received_at_ms > 0);
}

#[test]
fn bybit_instruments_info_parses_official_linear_fixture() {
    let fixture = include_str!("../../fixtures/bybit/instruments_info_linear_btcusdt.json");
    let response: BybitResponse<InstrumentInfoItem> =
        serde_json::from_str(fixture).expect("bybit instruments fixture");
    let mut rows = response
        .into_list("instruments-info")
        .expect("bybit instruments list");
    let item = rows.pop().expect("instrument row");

    assert_bybit_instrument_item(&item);

    let schema: BybitResponse<OfficialInstrumentInfoSchema> =
        serde_json::from_str(fixture).expect("bybit official instrument schema");
    let mut rows = schema
        .into_list("instruments-info")
        .expect("official instruments list");
    let item = rows.pop().expect("official instrument row");
    assert_bybit_official_instrument_schema(&item);
}

fn assert_bybit_instrument_item(item: &InstrumentInfoItem) {
    assert_eq!(item.symbol, "BTCUSDT");
    assert_eq!(item.settle_coin, "USDT");
    assert_eq!(item.funding_interval, 480);
}

fn assert_bybit_official_instrument_schema(item: &OfficialInstrumentInfoSchema) {
    assert_eq!(item.symbol, "BTCUSDT");
    assert_eq!(item.contract_type, "LinearPerpetual");
    assert_eq!(item.status, "Trading");
    assert_eq!(item.base_coin, "BTC");
    assert_eq!(item.quote_coin, "USDT");
    assert_eq!(item.settle_coin, "USDT");
    assert_eq!(item.price_scale, "2");
    assert_eq!(item.price_filter.tick_size, "0.10");
    assert_eq!(item.lot_size_filter.min_order_qty, "0.001");
    assert_eq!(item.lot_size_filter.qty_step, "0.001");
    assert_eq!(item.lot_size_filter.min_notional_value, "5");
    assert_eq!(item.leverage_filter.max_leverage, "100.00");
    assert_eq!(item.funding_interval, 480);
}

#[derive(Debug, Deserialize)]
struct BybitMarketTickersFixture {
    linear: BybitResponse<MarketTickerItem>,
    spot: BybitResponse<MarketTickerItem>,
}

#[derive(Debug, Deserialize)]
struct OfficialInstrumentInfoSchema {
    symbol: String,
    #[serde(rename = "contractType")]
    contract_type: String,
    status: String,
    #[serde(rename = "baseCoin")]
    base_coin: String,
    #[serde(rename = "quoteCoin")]
    quote_coin: String,
    #[serde(rename = "settleCoin")]
    settle_coin: String,
    #[serde(rename = "priceScale")]
    price_scale: String,
    #[serde(rename = "leverageFilter")]
    leverage_filter: OfficialLeverageFilter,
    #[serde(rename = "priceFilter")]
    price_filter: OfficialPriceFilter,
    #[serde(rename = "lotSizeFilter")]
    lot_size_filter: OfficialLotSizeFilter,
    #[serde(rename = "fundingInterval")]
    funding_interval: i64,
}

#[derive(Debug, Deserialize)]
struct OfficialLeverageFilter {
    #[serde(rename = "maxLeverage")]
    max_leverage: String,
}

#[derive(Debug, Deserialize)]
struct OfficialPriceFilter {
    #[serde(rename = "tickSize")]
    tick_size: String,
}

#[derive(Debug, Deserialize)]
struct OfficialLotSizeFilter {
    #[serde(rename = "minNotionalValue")]
    min_notional_value: String,
    #[serde(rename = "minOrderQty")]
    min_order_qty: String,
    #[serde(rename = "qtyStep")]
    qty_step: String,
}

#[test]
fn parse_ticker_drops_tick_with_blank_required_price() {
    let fixture = include_str!("../../fixtures/bybit/market_tickers_linear_spot_btcusdt.json");
    let fixture: BybitMarketTickersFixture =
        serde_json::from_str(fixture).expect("bybit market tickers fixture");
    let (rows, _) = fixture
        .linear
        .into_list_with_time("linear market tickers")
        .expect("linear ticker list");
    let mut raw = rows.into_iter().next().expect("linear ticker row");
    raw.bid1_price = String::new();
    assert!(parse_ticker(&raw).is_none());
}

#[test]
fn parse_ticker_drops_tick_with_unparseable_last() {
    let fixture = include_str!("../../fixtures/bybit/market_tickers_linear_spot_btcusdt.json");
    let fixture: BybitMarketTickersFixture =
        serde_json::from_str(fixture).expect("bybit market tickers fixture");
    let (rows, _) = fixture
        .linear
        .into_list_with_time("linear market tickers")
        .expect("linear ticker list");
    let mut raw = rows.into_iter().next().expect("linear ticker row");
    raw.last_price = "n/a".into();
    assert!(parse_ticker(&raw).is_none());
}
