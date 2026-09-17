use super::*;
use pretty_assertions::assert_eq;
use serde::Deserialize;
use shared_types::FundingRateData;

#[test]
fn parse_funding_8h_seconds_interval() {
    let contract = ContractItem {
        name: "BTC_USDT".into(),
        funding_rate: "0.0001".into(),
        funding_rate_indicative: "0.00012".into(),
        funding_next_apply: 1_700_028_800,
        funding_interval: 28_800,
        in_delisting: false,
    };
    let funding = parse_funding(&contract, 1_000_000.0).expect("funding parses");
    assert_eq!(funding.symbol, "BTC");
    assert_eq!(funding.exchange, "gate");
    assert_eq!(funding.funding_interval, 8);
    assert_eq!(funding.next_funding_time, 1_700_028_800_000);
    assert!((funding.rate_8h - 0.0001).abs() < 1e-12);
    assert!((funding.predicted_rate.unwrap() - 0.00012).abs() < 1e-12);
}

#[test]
fn gate_contracts_parses_official_fixture_metadata_and_funding() {
    let fixture = include_str!("../../fixtures/gate/futures_usdt_contracts_btc_usdt.json");
    let mut contracts: Vec<ContractItem> =
        serde_json::from_str(fixture).expect("gate contracts fixture");
    let contract = contracts.pop().expect("contract row");

    assert_gate_contract_metadata(&contract);

    let funding = parse_funding(&contract, 1_000_000.0).expect("funding parses");
    assert_gate_contract_funding(&funding);

    let mut schema: Vec<OfficialContractSchema> =
        serde_json::from_str(fixture).expect("gate official contract schema");
    let row = schema.pop().expect("official contract row");
    assert_gate_contract_schema(&row);
}

fn assert_gate_contract_metadata(contract: &ContractItem) {
    assert_eq!(contract.name, "BTC_USDT");
    assert_eq!(contract.funding_interval, 28_800);
    assert_eq!(contract.funding_next_apply, 1_780_444_800);
    assert_eq!(contract.funding_rate, "0.000063");
    assert_eq!(contract.funding_rate_indicative, "0.000063");
    assert!(!contract.in_delisting);
}

fn assert_gate_contract_funding(funding: &FundingRateData) {
    assert_eq!(funding.symbol, "BTC");
    assert_eq!(funding.funding_interval, 8);
    assert_eq!(funding.next_funding_time, 1_780_444_800_000);
    assert!((funding.rate - 0.000063).abs() < 1e-12);
    assert!((funding.rate_8h - 0.000063).abs() < 1e-12);
    assert!((funding.predicted_rate.unwrap() - 0.000063).abs() < 1e-12);
}

fn assert_gate_contract_schema(row: &OfficialContractSchema) {
    assert_eq!(row.name, "BTC_USDT");
    assert_eq!(row.kind, "direct");
    assert_eq!(row.status, "trading");
    assert_eq!(row.order_price_round, "0.1");
    assert_eq!(row.order_size_min, 1);
    assert_eq!(row.order_size_max, 12_000_000);
    assert_eq!(row.market_order_size_max, "8000000");
    assert!(!row.enable_decimal);
    assert_eq!(row.leverage_min, "1");
    assert_eq!(row.leverage_max, "200");
    assert_eq!(row.maintenance_rate, "0.003");
    assert_eq!(row.maker_fee_rate, "-0.0001");
    assert_eq!(row.taker_fee_rate, "0.00075");
    assert_eq!(row.contract_type, "");
}

#[test]
fn parse_funding_4h_normalized() {
    let contract = ContractItem {
        name: "ETH_USDT".into(),
        funding_rate: "0.0001".into(),
        funding_rate_indicative: String::new(),
        funding_next_apply: 1_700_014_400,
        funding_interval: 14_400,
        in_delisting: false,
    };
    let funding = parse_funding(&contract, 0.0).expect("funding parses");
    assert_eq!(funding.funding_interval, 4);
    assert!((funding.rate_8h - 0.0002).abs() < 1e-12);
    assert!(funding.predicted_rate.is_none());
}

#[derive(Debug, Deserialize)]
struct OfficialContractSchema {
    name: String,
    #[serde(rename = "type")]
    kind: String,
    status: String,
    #[serde(rename = "order_price_round")]
    order_price_round: String,
    #[serde(rename = "order_size_min")]
    order_size_min: i64,
    #[serde(rename = "order_size_max")]
    order_size_max: i64,
    #[serde(rename = "market_order_size_max")]
    market_order_size_max: String,
    #[serde(rename = "enable_decimal")]
    enable_decimal: bool,
    #[serde(rename = "leverage_min")]
    leverage_min: String,
    #[serde(rename = "leverage_max")]
    leverage_max: String,
    #[serde(rename = "maintenance_rate")]
    maintenance_rate: String,
    #[serde(rename = "maker_fee_rate")]
    maker_fee_rate: String,
    #[serde(rename = "taker_fee_rate")]
    taker_fee_rate: String,
    #[serde(rename = "contract_type")]
    contract_type: String,
}

#[test]
fn parse_spot_tick_pair_symbol() {
    let ticker = SpotTickerItem {
        currency_pair: "BTC_USDT".into(),
        last: "30000.5".into(),
        highest_bid: "30000.0".into(),
        lowest_ask: "30001.0".into(),
        quote_volume: "1000000".into(),
    };
    let tick = parse_spot_tick(&ticker).expect("spot tick parses");
    assert_eq!(tick.venue, "gate");
    assert_eq!(tick.symbol, "BTC/USDT");
}

#[test]
fn gate_spot_tickers_parse_official_fixture_keeps_missing_sizes_none() {
    let fixture = include_str!("../../fixtures/gate/spot_tickers_btc_eth_usdt.json");
    let rows: Vec<SpotTickerItem> =
        serde_json::from_str(fixture).expect("gate spot tickers fixture");
    assert_eq!(rows.len(), 2);

    let btc = rows
        .iter()
        .find(|row| row.currency_pair == "BTC_USDT")
        .expect("BTC_USDT spot row");
    let tick = parse_spot_tick(btc).expect("spot tick parses");

    assert_eq!(tick.venue, "gate");
    assert_eq!(tick.symbol, "BTC/USDT");
    assert_eq!(tick.bid.to_string(), "66655.5");
    assert_eq!(tick.ask.to_string(), "66655.6");
    assert_eq!(tick.last.to_string(), "66655.5");
    assert_eq!(tick.volume_24h.to_string(), "822334455.66");
    // Gate's spot `/api/v4/spot/tickers` payload carries no bid/ask size and no
    // exchange timestamp, so those stay None rather than a fabricated 0.
    assert_eq!(tick.bid_size, None);
    assert_eq!(tick.ask_size, None);
    assert_eq!(tick.exchange_ts_ms, None);
    assert!(tick.received_at_ms > 0);
    assert_eq!(tick.best_timestamp_ms(), tick.received_at_ms);
}

#[test]
fn parse_ticker_prefers_quote_volume() {
    let ticker = ticker_row("BTC_USDT", "30000.5", "30000.0", "30001.0", "12345", "99");
    let parsed = parse_ticker(&ticker).expect("ticker parses");
    assert_eq!(parsed.symbol, "BTC");
    assert_eq!(parsed.volume_24h, 12345.0);
}

#[test]
fn gate_futures_tickers_parse_official_fixture_prices_and_mark() {
    let fixture = include_str!("../../fixtures/gate/futures_usdt_tickers_btc_eth_usdt.json");
    let rows: Vec<TickerItem> = serde_json::from_str(fixture).expect("gate tickers fixture");
    let btc = rows
        .iter()
        .find(|row| row.contract == "BTC_USDT")
        .expect("BTC_USDT ticker");

    let ticker = parse_ticker(btc).expect("ticker parses");
    let mark = parse_mark_index(btc, 1_780_442_009_894).expect("mark/index parses");

    assert_eq!(rows.len(), 2);
    assert_eq!(ticker.symbol, "BTC");
    assert_eq!(ticker.exchange, "gate");
    assert!((ticker.bid - 66655.5).abs() < 1e-12);
    assert!((ticker.ask - 66655.6).abs() < 1e-12);
    assert!((ticker.last - 66655.5).abs() < 1e-12);
    assert!((ticker.volume_24h - 10442549740.0).abs() < 1e-6);
    assert_eq!(mark.symbol, "BTC");
    assert_eq!(mark.mark_price, 66655.5);
    assert_eq!(mark.index_price, Some(66684.48));
    assert_eq!(mark.open_interest, Some(648667152.0));
}

#[test]
fn parse_mark_index_uses_futures_tickers_fields() {
    let mut ticker = ticker_row("BTC_USDT", "30000.5", "30000.0", "30001.0", "12345", "99");
    ticker.mark_price = "30000.4".into();
    ticker.index_price = "30002.0".into();
    ticker.total_size = "73648".into();
    let row = parse_mark_index(&ticker, 1_700_000_000_000).expect("mark/index parses");
    assert_eq!(row.symbol, "BTC");
    assert_eq!(row.exchange, "gate");
    assert_eq!(row.mark_price, 30000.4);
    assert_eq!(row.index_price, Some(30002.0));
    assert_eq!(row.open_interest, Some(73648.0));
    assert_eq!(row.open_interest_value, None);
    assert_eq!(row.timestamp, 1_700_000_000_000);
}

#[test]
fn parse_funding_from_ws_returns_some_when_rate_set() {
    let mut ticker = ticker_row(
        "BTC_USDT",
        "30000.5",
        "30000.0",
        "30001.0",
        "1_000_000",
        "33",
    );
    ticker.funding_rate = "0.0001".into();
    ticker.funding_rate_indicative = "0.00012".into();
    ticker.funding_next_apply = 1_700_028_800;
    let funding = parse_funding_from_ws_ticker(&ticker, 8).expect("ws funding parses");
    assert_eq!(funding.symbol, "BTC");
    assert_eq!(funding.exchange, "gate");
    assert!((funding.rate - 0.0001).abs() < 1e-12);
    assert!((funding.rate_8h - 0.0001).abs() < 1e-12);
    assert_eq!(funding.next_funding_time, 1_700_028_800_000);
    assert_eq!(funding.funding_interval, 8);
    assert!((funding.predicted_rate.unwrap() - 0.00012).abs() < 1e-12);
}

#[test]
fn parse_funding_from_ws_normalises_4h_to_rate_8h() {
    let mut ticker = ticker_row("ETH_USDT", "0", "0", "0", "0", "0");
    ticker.funding_rate = "0.0001".into();
    ticker.funding_next_apply = 1_700_014_400;
    let funding = parse_funding_from_ws_ticker(&ticker, 4).expect("ws funding parses");
    assert_eq!(funding.funding_interval, 4);
    assert!((funding.rate_8h - 0.0002).abs() < 1e-12);
}

#[test]
fn parse_funding_from_ws_returns_none_when_rate_missing() {
    let ticker = ticker_row("ETH_USDT", "0", "0", "0", "0", "0");
    assert!(parse_funding_from_ws_ticker(&ticker, 8).is_none());
}

#[test]
fn parse_funding_from_ws_returns_none_when_settlement_time_is_missing() {
    let mut ticker = ticker_row("ETH_USDT", "0", "0", "0", "0", "0");
    ticker.funding_rate = "0.0001".into();
    assert!(parse_funding_from_ws_ticker(&ticker, 8).is_none());
}

fn ticker_row(
    contract: &str,
    last: &str,
    highest_bid: &str,
    lowest_ask: &str,
    volume_24h_quote: &str,
    volume_24h_settle: &str,
) -> TickerItem {
    TickerItem {
        contract: contract.into(),
        last: last.into(),
        highest_bid: highest_bid.into(),
        lowest_ask: lowest_ask.into(),
        volume_24h_quote: volume_24h_quote.into(),
        volume_24h_settle: volume_24h_settle.into(),
        funding_rate: String::new(),
        funding_rate_indicative: String::new(),
        funding_next_apply: 0,
        mark_price: String::new(),
        index_price: String::new(),
        total_size: String::new(),
    }
}

#[test]
fn parse_levels_accepts_object_and_array_shapes() {
    let levels = parse_levels(vec![
        DepthLevel::Object {
            p: "30000".into(),
            s: serde_json::json!("2"),
        },
        DepthLevel::Array([serde_json::json!("30001"), serde_json::json!(3)]),
        DepthLevel::Array([serde_json::json!("bad"), serde_json::json!(3)]),
    ]);
    assert_eq!(levels, vec![[30000.0, 2.0], [30001.0, 3.0]]);
}

#[test]
fn gate_futures_orderbook_parses_official_fixture_levels() {
    let fixture = include_str!("../../fixtures/gate/futures_usdt_order_book_btc_usdt.json");
    let book: OrderBookResp = serde_json::from_str(fixture).expect("gate orderbook fixture");

    let bids = parse_levels(book.bids);
    let asks = parse_levels(book.asks);

    assert!((book.update - 1_780_442_009.885).abs() < 1e-9);
    assert_eq!(bids.len(), 5);
    assert_eq!(asks.len(), 5);
    assert_eq!(bids[0], [66655.5, 47081.0]);
    assert_eq!(asks[0], [66655.6, 11098.0]);
}

#[test]
fn snap_gate_depth_to_valid_levels() {
    assert_eq!(snap_gate_depth(0), 1);
    assert_eq!(snap_gate_depth(1), 1);
    assert_eq!(snap_gate_depth(3), 1);
    assert_eq!(snap_gate_depth(7), 5);
    assert_eq!(snap_gate_depth(15), 10);
    assert_eq!(snap_gate_depth(30), 20);
    assert_eq!(snap_gate_depth(70), 50);
    assert_eq!(snap_gate_depth(500), 100);
}

#[test]
fn parse_ticker_drops_tick_with_blank_required_price() {
    let ticker = ticker_row("BTC_USDT", "30000.5", "", "30001.0", "12345", "99");
    assert!(parse_ticker(&ticker).is_none());
}

#[test]
fn parse_ticker_drops_tick_with_unparseable_last() {
    let ticker = ticker_row("BTC_USDT", "n/a", "30000.0", "30001.0", "12345", "99");
    assert!(parse_ticker(&ticker).is_none());
}
