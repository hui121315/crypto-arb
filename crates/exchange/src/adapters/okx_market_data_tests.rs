use super::*;
use pretty_assertions::assert_eq;

#[derive(serde::Deserialize)]
struct MarketTickersFixture {
    swap: crate::adapters::okx_response::OkxResponse<TickerItem>,
    spot: crate::adapters::okx_response::OkxResponse<TickerItem>,
}

#[test]
fn swap_inst_ids_keeps_usdt_swaps() {
    let rows = vec![
        InstrumentItem {
            inst_id: "BTC-USDT-SWAP".into(),
        },
        InstrumentItem {
            inst_id: "BTC-USDC-SWAP".into(),
        },
        InstrumentItem {
            inst_id: "ETH-USDT".into(),
        },
    ];
    assert_eq!(swap_inst_ids(rows), vec!["BTC-USDT-SWAP"]);
}

#[test]
fn funding_volume_map_uses_ccy_volume() {
    let rows = vec![ticker("BTC-USDT-SWAP", "1.5")];
    let map = funding_volume_map(rows);
    assert_eq!(map.get("BTC-USDT-SWAP"), Some(&1.5));
}

#[test]
fn parse_ticker_strips_swap_suffix() {
    let tick = parse_ticker(&ticker("BTC-USDT-SWAP", "1000000")).expect("ticker parses");
    assert_eq!(tick.exchange, "okx");
    assert_eq!(tick.symbol, "BTC");
    assert_eq!(tick.bid, 2000.0);
    assert_eq!(tick.ask, 2001.0);
}

#[test]
fn parse_spot_tick_pair_symbol() {
    let raw = ticker("ETH-USDT", "1000000");
    let tick = parse_spot_tick(&raw).expect("spot tick parses");
    assert_eq!(tick.venue, "okx");
    assert_eq!(tick.symbol, "ETH/USDT");
    assert_eq!(tick.exchange_ts_ms, Some(1_700_000_000_000));
    assert!(tick.received_at_ms > 0);
}

#[test]
fn orderbook_levels_take_first_two_columns() {
    let raw = OrderBookItem {
        bids: vec![
            vec!["30001".into(), "1.5".into(), "0".into(), "1".into()],
            vec!["30002".into(), "2.0".into(), "0".into(), "2".into()],
        ],
        asks: vec![vec!["30003".into(), "0.7".into(), "0".into(), "1".into()]],
        ts: "1700000000000".into(),
    };
    let parsed = orderbook_info("BTC".into(), raw);
    assert_eq!(parsed.bids.len(), 2);
    assert!((parsed.bids[0][0] - 30001.0).abs() < 1e-9);
    assert!((parsed.bids[0][1] - 1.5).abs() < 1e-9);
    assert_eq!(parsed.timestamp, 1_700_000_000_000);
}

#[test]
fn okx_orderbook_parses_official_fixture_levels() {
    let fixture = include_str!("../../fixtures/okx/market_books_btc_usdt_swap.json");
    let response: crate::adapters::okx_response::OkxResponse<OrderBookItem> =
        serde_json::from_str(fixture).expect("okx orderbook fixture");
    let mut rows = response.into_data("books").expect("okx orderbook data");
    let parsed = orderbook_info("BTC".into(), rows.pop().expect("orderbook row"));

    assert_eq!(parsed.exchange, "okx");
    assert_eq!(parsed.symbol, "BTC");
    assert_eq!(parsed.timestamp, 1_780_436_122_556);
    assert_eq!(parsed.asks.len(), 5);
    assert_eq!(parsed.bids.len(), 5);
    assert!((parsed.asks[0][0] - 67_870.4).abs() < 1e-9);
    assert!((parsed.asks[0][1] - 40.66).abs() < 1e-9);
    assert!((parsed.bids[0][0] - 67_870.3).abs() < 1e-9);
    assert!((parsed.bids[0][1] - 428.54).abs() < 1e-9);
}

#[test]
fn okx_market_tickers_parse_official_swap_and_spot_fixture() {
    let fixture = include_str!("../../fixtures/okx/market_tickers_swap_spot_btc_eth_usdt.json");
    let response: MarketTickersFixture =
        serde_json::from_str(fixture).expect("okx market tickers fixture");
    let swap_rows = response.swap.into_data("swap tickers").expect("swap rows");
    let spot_rows = response.spot.into_data("spot tickers").expect("spot rows");

    let btc_swap = swap_rows
        .iter()
        .find(|row| row.inst_id == "BTC-USDT-SWAP")
        .expect("btc swap ticker");
    assert_okx_swap_ticker(btc_swap);

    let btc_spot = spot_rows
        .iter()
        .find(|row| row.inst_id == "BTC-USDT")
        .expect("btc spot ticker");
    assert_okx_spot_ticker(btc_spot);
}

fn assert_okx_swap_ticker(btc_swap: &TickerItem) {
    let parsed_swap = parse_ticker(btc_swap).expect("swap ticker parses");
    assert_eq!(btc_swap.inst_type(), Some("SWAP"));
    assert_eq!(parsed_swap.exchange, "okx");
    assert_eq!(parsed_swap.symbol, "BTC");
    assert_eq!(parsed_swap.bid, 66_554.4);
    assert_eq!(parsed_swap.ask, 66_554.5);
    assert_eq!(parsed_swap.last, 66_554.5);
    assert_eq!(parsed_swap.volume_24h, 201_680.534_4);
    assert_eq!(parsed_swap.timestamp, 1_780_442_471_010);
}

fn assert_okx_spot_ticker(btc_spot: &TickerItem) {
    let parsed_spot = parse_spot_tick(btc_spot).expect("spot tick parses");
    assert_eq!(btc_spot.inst_type(), Some("SPOT"));
    assert_eq!(parsed_spot.venue, "okx");
    assert_eq!(parsed_spot.symbol, "BTC/USDT");
    assert_eq!(parsed_spot.bid.to_string(), "66585");
    assert_eq!(parsed_spot.ask.to_string(), "66585.1");
    assert_eq!(parsed_spot.last.to_string(), "66585");
    assert_eq!(parsed_spot.bid_size.unwrap().to_string(), "0.31232584");
    assert_eq!(parsed_spot.ask_size.unwrap().to_string(), "3.05304999");
    assert_eq!(parsed_spot.volume_24h.to_string(), "1035288267.64312691");
    assert_eq!(parsed_spot.exchange_ts_ms, Some(1_780_442_470_511));
    assert!(parsed_spot.received_at_ms > 0);
}

#[test]
fn parse_ticker_drops_tick_with_blank_required_price() {
    let mut raw = ticker("BTC-USDT-SWAP", "1000000");
    raw.bid_px = String::new();
    assert!(parse_ticker(&raw).is_none());
}

#[test]
fn parse_ticker_drops_tick_with_unparseable_last() {
    let mut raw = ticker("BTC-USDT-SWAP", "1000000");
    raw.last = "n/a".into();
    assert!(parse_ticker(&raw).is_none());
}

fn ticker(inst_id: &str, volume: &str) -> TickerItem {
    TickerItem {
        inst_type: if inst_id.ends_with("-SWAP") {
            "SWAP".into()
        } else {
            "SPOT".into()
        },
        inst_id: inst_id.into(),
        last: "2000.5".into(),
        bid_px: "2000.0".into(),
        ask_px: "2001.0".into(),
        bid_sz: "1.5".into(),
        ask_sz: "2.5".into(),
        vol_ccy_24h: volume.into(),
        ts: "1700000000000".into(),
    }
}
