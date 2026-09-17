use super::super::bitget_response::{BitgetObjectResponse, BitgetResponse};
use super::*;

#[test]
fn parse_ticker_uses_v3_field_names() {
    // Sample payload copied verbatim from a live
    // `GET /api/v3/market/tickers?category=USDT-FUTURES&symbol=BTCUSDT` response
    // (PR-DP-13 · B-2 manual capture).
    let raw = r#"{
        "category": "USDT-FUTURES",
        "symbol": "BTCUSDT",
        "ts": "1779511245529",
        "lastPrice": "75492.1",
        "openPrice24h": "77650.9",
        "highPrice24h": "77683",
        "lowPrice24h": "75188",
        "ask1Price": "75492.2",
        "bid1Price": "75492.1",
        "bid1Size": "1.4162",
        "ask1Size": "29.8745",
        "price24hPcnt": "-0.0278",
        "volume24h": "44545.8656",
        "turnover24h": "3404219299.89529",
        "indexPrice": "75525.1361651486510947",
        "markPrice": "75492.1",
        "fundingRate": "0.000078",
        "openInterest": "31601.0065"
    }"#;
    let item: UtaTickerItem = serde_json::from_str(raw).expect("parse ticker");
    let ticker = parse_ticker(&item).expect("ticker parses");
    assert_eq!(ticker.symbol, "BTC");
    assert_eq!(ticker.exchange, "bitget");
    assert!((ticker.bid - 75_492.1).abs() < 1e-6);
    assert!((ticker.ask - 75_492.2).abs() < 1e-6);
    assert!((ticker.last - 75_492.1).abs() < 1e-6);
    // turnover24h (quote volume) wins over base volume24h.
    assert!((ticker.volume_24h - 3_404_219_299.895_29).abs() < 1e-3);
    assert_eq!(ticker.timestamp, 1_779_511_245_529);
}

#[test]
fn parse_mark_index_uses_official_ticker_fields() {
    let raw = r#"{
        "category": "USDT-FUTURES",
        "symbol": "BTCUSDT",
        "ts": "1779511245529",
        "markPrice": "75492.1",
        "indexPrice": "75525.1361651486510947",
        "openInterest": "31601.0065"
    }"#;
    let item: UtaTickerItem = serde_json::from_str(raw).expect("parse ticker");
    let row = parse_mark_index(&item).expect("mark/index row parses");
    assert_eq!(row.symbol, "BTC");
    assert_eq!(row.exchange, "bitget");
    assert!((row.mark_price - 75_492.1).abs() < 1e-6);
    assert!((row.index_price.unwrap() - 75_525.136_165_148_65).abs() < 1e-9);
    assert!((row.open_interest.unwrap() - 31_601.006_5).abs() < 1e-9);
    assert_eq!(row.open_interest_value, None);
    assert_eq!(row.timestamp, 1_779_511_245_529);
}

#[test]
fn parse_ticker_falls_back_to_base_volume() {
    let raw = r#"{
        "symbol": "ETHUSDT",
        "lastPrice": "2000",
        "bid1Price": "1999",
        "ask1Price": "2001",
        "volume24h": "555.0",
        "ts": "0"
    }"#;
    let item: UtaTickerItem = serde_json::from_str(raw).expect("parse ticker");
    let ticker = parse_ticker(&item).expect("ticker parses");
    // quoteVolume24h missing -> fall back to volume24h.
    assert!((ticker.volume_24h - 555.0).abs() < 1e-9);
}

#[test]
fn parse_funding_uses_v3_fields_including_interval() {
    // Sample payload mirrors the response at
    // <https://www.bitget.com/api-doc/uta/public/Get-Current-Funding-Rate>.
    let raw = r#"{
        "symbol": "BTCUSDT",
        "fundingRate": "0.000071",
        "fundingRateInterval": "8",
        "nextUpdate": "1743062400000",
        "minFundingRate": "-0.003",
        "maxFundingRate": "0.003"
    }"#;
    let item: UtaFundingRateItem = serde_json::from_str(raw).expect("parse funding rate");
    let funding = parse_funding(&item, 1234.5).expect("funding parses");
    assert_eq!(funding.symbol, "BTC");
    assert_eq!(funding.exchange, "bitget");
    assert!((funding.rate - 0.000_071).abs() < 1e-9);
    assert_eq!(funding.funding_interval, 8);
    assert!((funding.rate_8h - 0.000_071).abs() < 1e-9);
    assert_eq!(funding.next_funding_time, 1_743_062_400_000);
    assert!((funding.volume_24h - 1234.5).abs() < 1e-9);
}

#[test]
fn parse_funding_from_ticker_uses_official_ticker_funding_field() {
    let fixture = include_str!("../../fixtures/bitget/uta_tickers_usdt_futures_spot_btcusdt.json");
    let body: serde_json::Value = serde_json::from_str(fixture).expect("bitget tickers fixture");
    let wrap: BitgetResponse<UtaTickerItem> =
        serde_json::from_value(body["futures"].clone()).expect("futures ticker response");
    let items = wrap.into_data("tickers").expect("futures ticker rows");
    let funding = parse_funding_from_ticker(&items[0], 8)
        .expect("funding schedule derives from ticker time and official interval");

    assert_eq!(funding.next_funding_time, 1_780_444_800_000);
}

#[test]
fn ticker_payload_accepts_nullable_optional_numeric_fields() {
    let raw = r#"{
        "symbol":"TRXUSDT",
        "lastPrice":"0.32661",
        "bid1Price":"0.32660",
        "ask1Price":"0.32661",
        "bid1Size":null,
        "ask1Size":null,
        "turnover24h":"985015.6595",
        "nextFundingTime":null,
        "ts":"1784517005016"
    }"#;
    let item: UtaTickerItem = serde_json::from_str(raw).expect("nullable ticker fields parse");
    assert!(item.bid1_size.is_empty());
    assert!(item.ask1_size.is_empty());
    assert!(item.next_funding_time.is_empty());
    assert!(parse_spot_tick(&item).is_some());
}

#[test]
fn parse_funding_from_ticker_requires_interval_and_next_funding_time() {
    let item = UtaTickerItem {
        symbol: "BTCUSDT".into(),
        last_price: "75492.1".into(),
        bid1_price: "75492.0".into(),
        ask1_price: "75492.2".into(),
        bid1_size: "1".into(),
        ask1_size: "2".into(),
        volume_24h: "44545.8656".into(),
        quote_volume_24h: "3404219299.89529".into(),
        mark_price: "75492.1".into(),
        index_price: "75525.13".into(),
        open_interest: "31601.0065".into(),
        funding_rate: "0.000078".into(),
        next_funding_time: "1779537600000".into(),
        ts: "1779511245529".into(),
    };
    assert!(parse_funding_from_ticker(&item, 3).is_none());
    let funding = parse_funding_from_ticker(&item, 8).expect("ticker funding parses");
    assert_eq!(funding.symbol, "BTC");
    assert_eq!(funding.funding_interval, 8);
    assert!((funding.rate - 0.000_078).abs() < 1e-12);
    assert!((funding.rate_8h - 0.000_078).abs() < 1e-12);
    assert!((funding.volume_24h - 3_404_219_299.895_29).abs() < 1e-4);
    assert_eq!(funding.next_funding_time, 1_779_537_600_000);
    assert_eq!(funding.timestamp, 1_779_511_245_529);
}

#[test]
fn parse_funding_normalizes_4h_to_rate_8h() {
    let item = UtaFundingRateItem {
        symbol: "BTCUSDT".into(),
        funding_rate: "0.0001".into(),
        next_update: "1743062400000".into(),
        funding_rate_interval: "4".into(),
    };
    let funding = parse_funding(&item, 0.0).expect("4h funding parses");
    assert_eq!(funding.funding_interval, 4);
    assert!((funding.rate_8h - 0.0002).abs() < 1e-12);
}

#[test]
fn parse_funding_rejects_missing_or_invalid_required_fields() {
    for item in [
        UtaFundingRateItem {
            symbol: "BTCUSDT".into(),
            funding_rate: String::new(),
            next_update: "1743062400000".into(),
            funding_rate_interval: "8".into(),
        },
        UtaFundingRateItem {
            symbol: "BTCUSDT".into(),
            funding_rate: "0.0001".into(),
            next_update: "0".into(),
            funding_rate_interval: "8".into(),
        },
        UtaFundingRateItem {
            symbol: "BTCUSDT".into(),
            funding_rate: "0.0001".into(),
            next_update: "1743062400000".into(),
            funding_rate_interval: String::new(),
        },
        UtaFundingRateItem {
            symbol: "BTCUSDT".into(),
            funding_rate: "0.0001".into(),
            next_update: "1743062400000".into(),
            funding_rate_interval: "3".into(),
        },
    ] {
        assert!(parse_funding(&item, 0.0).is_none());
    }
    let valid = UtaFundingRateItem {
        symbol: "BTCUSDT".into(),
        funding_rate: "0.0000".into(),
        next_update: "1743062400000".into(),
        funding_rate_interval: "8".into(),
    };
    assert!(parse_funding(&valid, f64::NAN).is_none());
}

#[test]
fn parse_orderbook_normalises_v3_short_keys_and_numeric_levels() {
    // Sample payload copied verbatim from a live
    // `GET /api/v3/market/orderbook?category=USDT-FUTURES&symbol=BTCUSDT&limit=5`
    // response (PR-DP-13 · B-2 manual capture). V3 uses short `a` / `b`
    // keys with numeric levels rather than V2 string-encoded `asks` / `bids`.
    let raw = r#"{
        "a": [[75492.2, 29.8745], [75492.3, 0.1325], [75492.5, 0]],
        "b": [[75492.1, 1.4162], [75492.0, 0.0001], [-1, 1]],
        "ts": "1779511245532"
    }"#;
    let item: UtaDepthItem = serde_json::from_str(raw).expect("parse orderbook");
    let (bids, asks, ts) = finalize_orderbook_levels(&item);
    // Zero / negative size or price levels are filtered out (parse_levels).
    assert_eq!(asks.len(), 2);
    assert_eq!(bids.len(), 2);
    assert!((asks[0][0] - 75_492.2).abs() < 1e-6);
    assert!((bids[0][1] - 1.4162).abs() < 1e-6);
    assert_eq!(ts, 1_779_511_245_532);
}

#[test]
fn bitget_uta_orderbook_parses_official_fixture_levels() {
    let fixture = include_str!("../../fixtures/bitget/uta_orderbook_usdt_futures_btcusdt.json");
    let wrap: BitgetObjectResponse<UtaDepthItem> =
        serde_json::from_str(fixture).expect("bitget orderbook fixture");
    let book = wrap.into_result("orderbook").expect("orderbook data");
    let (bids, asks, ts) = finalize_orderbook_levels(&book);

    assert_eq!(bids.len(), 5);
    assert_eq!(asks.len(), 5);
    assert_eq!(bids[0], [66766.9, 0.4599]);
    assert_eq!(asks[0], [66767.0, 6.3255]);
    assert_eq!(ts, 1_780_444_171_011);
}

#[test]
fn bitget_uta_instruments_parses_official_fixture_metadata() {
    let fixture = include_str!("../../fixtures/bitget/uta_instruments_usdt_futures_btcusdt.json");
    let official: serde_json::Value =
        serde_json::from_str(fixture).expect("bitget instruments fixture json");
    assert_eq!(official["data"][0]["category"], "USDT-FUTURES");
    assert_eq!(official["data"][0]["baseCoin"], "BTC");
    assert_eq!(official["data"][0]["quoteCoin"], "USDT");
    assert_eq!(official["data"][0]["isRwa"], "NO");
    assert_eq!(official["data"][0]["symbolType"], "crypto");
    assert_eq!(official["data"][0]["status"], "online");

    let wrap: BitgetResponse<UtaInstrumentItem> =
        serde_json::from_str(fixture).expect("bitget instruments fixture");
    let rows = wrap.into_data("instruments").expect("instrument rows");
    let row = &rows[0];
    let (symbol, interval) = instrument_funding_interval(row);

    assert_eq!(rows.len(), 1);
    assert_eq!(row.symbol, "BTCUSDT");
    assert_eq!(symbol, "BTCUSDT");
    assert_eq!(interval, Some(8));
}

#[test]
fn bitget_uta_current_funding_parses_official_fixture() {
    let fixture = include_str!("../../fixtures/bitget/uta_current_fund_rate_btcusdt.json");
    let wrap: BitgetResponse<UtaFundingRateItem> =
        serde_json::from_str(fixture).expect("bitget funding fixture");
    let rows = wrap.into_data("current-fund-rate").expect("funding rows");
    let funding =
        parse_funding(&rows[0], 6_289_983_502.276_38).expect("official funding fixture parses");

    assert_eq!(rows.len(), 1);
    assert_eq!(funding.symbol, "BTC");
    assert_eq!(funding.funding_interval, 8);
    assert!((funding.rate - 0.000_078).abs() < 1e-12);
    assert_eq!(funding.next_funding_time, 1_780_444_800_000);
}

#[test]
fn bitget_uta_tickers_parse_official_futures_and_spot_fixture() {
    let fixture = include_str!("../../fixtures/bitget/uta_tickers_usdt_futures_spot_btcusdt.json");
    let body: serde_json::Value = serde_json::from_str(fixture).expect("bitget tickers fixture");
    let futures: BitgetResponse<UtaTickerItem> =
        serde_json::from_value(body["futures"].clone()).expect("futures ticker response");
    let spot: BitgetResponse<UtaTickerItem> =
        serde_json::from_value(body["spot"].clone()).expect("spot ticker response");
    let futures = futures.into_data("futures tickers").expect("futures rows");
    let spot = spot.into_data("spot tickers").expect("spot rows");

    let perp = parse_ticker(&futures[0]).expect("ticker parses");
    let spot_tick = parse_spot_tick(&spot[0]).expect("spot tick parses");

    assert_eq!(perp.symbol, "BTC");
    assert!((perp.bid - 66_767.2).abs() < 1e-9);
    assert!((perp.ask - 66_767.3).abs() < 1e-9);
    assert!((perp.volume_24h - 6_289_983_502.276_38).abs() < 1e-4);
    assert_eq!(spot_tick.symbol, "BTC/USDT");
    assert_eq!(spot_tick.bid_size.unwrap().to_string(), "0.637802");
    assert_eq!(spot_tick.ask_size.unwrap().to_string(), "0.76768");
}

#[test]
fn parse_spot_tick_pair_symbol_uses_v3_field_names() {
    let item = UtaTickerItem {
        symbol: "ETHUSDT".into(),
        last_price: "2000.5".into(),
        bid1_price: "2000.0".into(),
        ask1_price: "2001.0".into(),
        bid1_size: "1.25".into(),
        ask1_size: "2.5".into(),
        volume_24h: "12345.0".into(),
        // V3 calls this `turnover24h`; the Rust field keeps the semantic name.
        quote_volume_24h: "98765.0".into(),
        mark_price: "2000.5".into(),
        index_price: "2000.0".into(),
        open_interest: "123.45".into(),
        funding_rate: String::new(),
        next_funding_time: String::new(),
        ts: "1700000000000".into(),
    };
    let tick = parse_spot_tick(&item).expect("spot tick parses");
    assert_eq!(tick.venue, "bitget");
    assert_eq!(tick.symbol, "ETH/USDT");
    assert_eq!(tick.exchange_ts_ms, Some(1_700_000_000_000));
    assert!(tick.received_at_ms > 0);
    // Spot tick volume uses quoteVolume24h, matching V2 behaviour.
    assert_eq!(tick.volume_24h.to_string(), "98765.0");
    assert_eq!(tick.bid_size.unwrap().to_string(), "1.25");
    assert_eq!(tick.ask_size.unwrap().to_string(), "2.5");
}

#[test]
fn parse_ticker_drops_tick_when_bid_missing() {
    let raw =
        r#"{"symbol":"BTCUSDT","lastPrice":"75492.1","ask1Price":"75492.2","ts":"1779511245529"}"#;
    let item: UtaTickerItem = serde_json::from_str(raw).expect("parse ticker");
    assert!(parse_ticker(&item).is_none());
}

#[test]
fn parse_ticker_drops_tick_with_unparseable_last() {
    let raw = r#"{"symbol":"BTCUSDT","lastPrice":"n/a","bid1Price":"75492.0","ask1Price":"75492.2","ts":"1779511245529"}"#;
    let item: UtaTickerItem = serde_json::from_str(raw).expect("parse ticker");
    assert!(parse_ticker(&item).is_none());
}
