use super::*;
use pretty_assertions::assert_eq;
use serde::Deserialize;
use shared_types::{FundingRateData, MarkIndexInfo};

#[test]
fn parse_mark_index_uses_premium_index_fields() {
    let item = PremiumIndexItem {
        symbol: "BTCUSDT".into(),
        mark_price: "11793.63104562".into(),
        index_price: "11781.80495970".into(),
        last_funding_rate: "0.00038246".into(),
        next_funding_time: 1_597_392_000_000,
        time: 1_597_370_495_002,
    };

    let row = parse_mark_index(&item).expect("mark/index parses");
    assert_eq!(row.symbol, "BTC");
    assert_eq!(row.exchange, "binance");
    assert!((row.mark_price - 11_793.631_045_62).abs() < 1e-9);
    assert_eq!(row.index_price, Some(11_781.804_959_70));
    assert_eq!(row.open_interest, None);
    assert_eq!(row.open_interest_value, None);
    assert_eq!(row.timestamp, 1_597_370_495_002);
}

#[test]
fn parse_mark_index_rejects_zero_mark_price() {
    let item = PremiumIndexItem {
        symbol: "BTCUSDT".into(),
        mark_price: "0".into(),
        index_price: "11781.80495970".into(),
        last_funding_rate: "0.00038246".into(),
        next_funding_time: 1_597_392_000_000,
        time: 0,
    };

    assert!(parse_mark_index(&item).is_none());
}

#[test]
fn parse_open_interest_uses_official_fields() {
    let item = OpenInterestItem {
        symbol: "BTCUSDT".into(),
        open_interest: "10659.509".into(),
        time: 1_589_437_530_011,
    };

    let (symbol, value, time) = parse_open_interest(&item).expect("open interest parses");
    assert_eq!(symbol, "BTCUSDT");
    assert_eq!(value, 10659.509);
    assert_eq!(time, 1_589_437_530_011);
}

#[test]
fn open_interest_parses_official_fixture() {
    let fixture = include_str!("../../fixtures/binance/usdm_open_interest_btcusdt.json");
    let row: OpenInterestItem =
        serde_json::from_str(fixture).expect("binance open interest fixture");

    let (symbol, value, time) = parse_open_interest(&row).expect("open interest parses");
    assert_eq!(symbol, "BTCUSDT");
    assert_eq!(value, 10659.509);
    assert_eq!(time, 1_589_437_530_011);

    let official: OfficialOpenInterestRow =
        serde_json::from_str(fixture).expect("official open interest schema");
    assert_eq!(official.symbol, "BTCUSDT");
    assert_eq!(official.open_interest, "10659.509");
    assert_eq!(official.time, 1_589_437_530_011);
}

#[test]
fn premium_index_parses_official_fixture_funding_and_mark_index() {
    let fixture = include_str!("../../fixtures/binance/usdm_premium_index_btcusdt.json");
    let rows: Vec<PremiumIndexItem> =
        serde_json::from_str(fixture).expect("binance premium index fixture");
    assert_eq!(rows.len(), 1);

    let funding = parse_funding(&rows[0], 123_456.0, 8).expect("funding parses");
    assert_binance_premium_funding(&funding);

    let mark = parse_mark_index(&rows[0]).expect("mark/index");
    assert_binance_premium_mark(&mark);

    let official: Vec<OfficialPremiumIndexRow> =
        serde_json::from_str(fixture).expect("official premium index schema");
    assert_binance_premium_schema(&official[0]);
}

fn assert_binance_premium_funding(funding: &FundingRateData) {
    assert_eq!(funding.symbol, "BTC");
    assert_eq!(funding.exchange, "binance");
    assert_eq!(funding.next_funding_time, 1_597_392_000_000);
    assert_eq!(funding.funding_interval, 8);
    assert_eq!(funding.timestamp, 1_597_370_495_002);
    assert!((funding.rate - 0.000_382_46).abs() < 1e-12);
    assert!((funding.rate_8h - 0.000_382_46).abs() < 1e-12);
    assert!((funding.volume_24h - 123_456.0).abs() < f64::EPSILON);
}

fn assert_binance_premium_mark(mark: &MarkIndexInfo) {
    assert_eq!(mark.symbol, "BTC");
    assert_eq!(mark.exchange, "binance");
    assert!((mark.mark_price - 11_793.631_045_62).abs() < 1e-9);
    assert_eq!(mark.index_price, Some(11_781.804_959_70));
    assert_eq!(mark.timestamp, 1_597_370_495_002);
}

fn assert_binance_premium_schema(row: &OfficialPremiumIndexRow) {
    assert_eq!(row.symbol, "BTCUSDT");
    assert_eq!(row.estimated_settle_price, "11781.16138815");
    assert_eq!(row.interest_rate, "0.00010000");
}

#[test]
fn binance_spot_ticker_24hr_parses_official_fixture() {
    let fixture = include_str!("../../fixtures/binance/spot_ticker_24hr_full.json");
    let rows: Vec<Ticker24hItem> =
        serde_json::from_str(fixture).expect("binance spot ticker fixture");
    assert_eq!(rows.len(), 1);

    let tick = parse_spot_tick(&rows[0]).expect("spot tick");
    assert_binance_spot_tick(&tick);

    let official: Vec<OfficialSpotTicker24hRow> =
        serde_json::from_str(fixture).expect("official spot ticker schema");
    assert_eq!(official[0].symbol, "BNBBTC");
    assert_eq!(official[0].bid_qty, "100.00000000");
    assert_eq!(official[0].ask_qty, "100.00000000");
    assert_eq!(official[0].count, 76);
}

fn assert_binance_spot_tick(tick: &shared_types::SpotTick) {
    assert_eq!(tick.venue, "binance");
    assert_eq!(tick.symbol, "BNB/BTC");
    assert_eq!(tick.bid.to_string(), "4.00000000");
    assert_eq!(tick.ask.to_string(), "4.00000200");
    assert_eq!(tick.last.to_string(), "4.00000200");
    assert_eq!(tick.bid_size, None);
    assert_eq!(tick.ask_size, None);
    assert_eq!(tick.volume_24h.to_string(), "15.30000000");
    assert_eq!(tick.exchange_ts_ms, Some(1_499_869_899_040));
    assert!(tick.received_at_ms > 0);
}

#[test]
fn binance_usdm_ticker_24hr_parses_official_fixture() {
    let fixture = include_str!("../../fixtures/binance/usdm_ticker_24hr_btcusdt.json");
    let rows: Vec<Ticker24hItem> =
        serde_json::from_str(fixture).expect("binance usdm ticker fixture");
    assert_eq!(rows.len(), 1);

    let book = BookTickerItem {
        symbol: "BTCUSDT".into(),
        bid_price: "3.99000000".into(),
        ask_price: "4.01000000".into(),
        time: 0,
    };
    let tick = parse_ticker(&rows[0], "BTC", Some(&book)).expect("ticker parses");
    assert_eq!(tick.exchange, "binance");
    assert_eq!(tick.symbol, "BTC");
    assert!((tick.last - 4.000_002).abs() < 1e-12);
    assert!((tick.volume_24h - 15.3).abs() < 1e-12);
    assert_eq!(tick.timestamp, 1_499_869_899_040);

    let official: Vec<OfficialUsdMFuturesTicker24hRow> =
        serde_json::from_str(fixture).expect("official usdm ticker schema");
    assert_eq!(official[0].symbol, "BTCUSDT");
    assert_eq!(official[0].weighted_avg_price, "0.29628482");
    assert_eq!(official[0].last_qty, "200.00000000");
    assert_eq!(official[0].volume, "8913.30000000");
    assert_eq!(official[0].count, 76);
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OfficialSpotTicker24hRow {
    symbol: String,
    bid_qty: String,
    ask_qty: String,
    count: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OfficialPremiumIndexRow {
    symbol: String,
    estimated_settle_price: String,
    interest_rate: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OfficialOpenInterestRow {
    symbol: String,
    open_interest: String,
    time: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OfficialUsdMFuturesTicker24hRow {
    symbol: String,
    weighted_avg_price: String,
    last_qty: String,
    volume: String,
    count: u64,
}
