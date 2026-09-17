use super::super::kucoin_response::KucoinResponse;
use super::*;
use pretty_assertions::assert_eq;

#[test]
fn symbol_xbt_btc_round_trip_helpers() {
    assert_eq!(normalized_to_kucoin("BTC"), "BTC");
    assert_eq!(normalized_to_kucoin("BTC-USDT"), "BTC-USDT");
    assert_eq!(normalized_to_kucoin("BTC-USDC"), "BTC-USDC");
    assert_eq!(normalized_to_kucoin("BTC-USD"), "BTC-USD");
    assert_eq!(kucoin_to_normalized("XBTUSDTM"), "BTC");
    assert_eq!(kucoin_to_normalized("XBTUSDCM"), "BTC");
    assert_eq!(normalized_to_kucoin("ETH"), "ETH");
    assert_eq!(normalized_to_kucoin("ETH-USDT"), "ETH-USDT");
    assert_eq!(kucoin_to_normalized("ETHUSDTM"), "ETH");
    assert_eq!(normalized_to_kucoin("XBTUSDTM"), "XBTUSDTM");
}

#[test]
fn to_kucoin_base_only_aliases_btc() {
    assert_eq!(to_kucoin_base("BTC"), Some("XBT"));
    assert_eq!(to_kucoin_base("btc"), Some("XBT"));
    assert_eq!(to_kucoin_base("ETH"), None);
    assert_eq!(to_kucoin_base("SOL"), None);
    assert_eq!(to_kucoin_base("DOGE"), None);
    assert_eq!(to_kucoin_base(""), None);
}

#[test]
fn snap_kucoin_depth_endpoint_two_buckets() {
    assert_eq!(snap_kucoin_depth_endpoint(0), "depth100");
    assert_eq!(snap_kucoin_depth_endpoint(1), "depth20");
    assert_eq!(snap_kucoin_depth_endpoint(10), "depth20");
    assert_eq!(snap_kucoin_depth_endpoint(20), "depth20");
    assert_eq!(snap_kucoin_depth_endpoint(21), "depth100");
    assert_eq!(snap_kucoin_depth_endpoint(50), "depth100");
    assert_eq!(snap_kucoin_depth_endpoint(100), "depth100");
    assert_eq!(snap_kucoin_depth_endpoint(500), "depth100");
}

#[test]
fn parse_funding_8h_from_granularity() {
    let json = serde_json::json!({
        "symbol": "XBTUSDTM",
        "fundingFeeRate": 0.0001,
        "predictedFundingFeeRate": 0.00012,
        "fundingRateGranularity": 28_800_000_i64,
        "nextFundingRateDateTime": 1_700_028_800_000_i64,
        "lastTradePrice": 30000.0,
        "turnoverOf24h": 1_500_000_000.0
    });
    let contract: ContractActive = serde_json::from_value(json).unwrap();
    let funding = parse_funding(&contract).expect("funding parses");
    assert_eq!(funding.symbol, "BTC");
    assert_eq!(funding.exchange, "kucoin");
    assert_eq!(funding.funding_interval, 8);
    assert!((funding.rate - 0.0001).abs() < 1e-12);
    assert!((funding.rate_8h - 0.0001).abs() < 1e-12);
    assert!((funding.predicted_rate.unwrap() - 0.00012).abs() < 1e-12);
    assert_eq!(funding.next_funding_time, 1_700_028_800_000);
}

#[test]
fn parse_funding_4h_normalized() {
    let json = serde_json::json!({
        "symbol": "ETHUSDTM",
        "fundingFeeRate": 0.0001,
        "predictedFundingFeeRate": null,
        "fundingRateGranularity": 14_400_000_i64,
        "nextFundingRateDateTime": 1_700_014_400_000_i64,
        "lastTradePrice": 0.0,
        "turnoverOf24h": 0.0
    });
    let contract: ContractActive = serde_json::from_value(json).unwrap();
    let funding = parse_funding(&contract).expect("4h funding parses");
    assert_eq!(funding.funding_interval, 4);
    assert!((funding.rate_8h - 0.0002).abs() < 1e-12);
    assert!(funding.predicted_rate.is_none());
}

#[test]
fn parse_funding_rejects_missing_required_fields() {
    let base = serde_json::json!({
        "symbol": "NEWUSDTM",
        "fundingFeeRate": 0.0001,
        "predictedFundingFeeRate": null,
        "fundingRateGranularity": 28_800_000_i64,
        "nextFundingRateDateTime": 1_700_028_800_000_i64,
        "turnoverOf24h": 10_000.0
    });

    for patch in [
        serde_json::json!({"fundingFeeRate": null}),
        serde_json::json!({"fundingRateGranularity": null}),
        serde_json::json!({"fundingRateGranularity": 0}),
        serde_json::json!({"fundingRateGranularity": 3_600_001_i64}),
        serde_json::json!({"nextFundingRateDateTime": null}),
        serde_json::json!({"nextFundingRateDateTime": 0}),
        serde_json::json!({"turnoverOf24h": null}),
        serde_json::json!({"turnoverOf24h": -1.0}),
    ] {
        let mut row = base.clone();
        merge_object(&mut row, &patch);
        let contract: ContractActive = serde_json::from_value(row).unwrap();
        assert!(parse_funding(&contract).is_none());
    }
}

#[test]
fn parse_funding_allows_explicit_zero_rate_and_volume() {
    let json = serde_json::json!({
        "symbol": "ZEROUSDTM",
        "fundingFeeRate": 0.0,
        "predictedFundingFeeRate": null,
        "fundingRateGranularity": 28_800_000_i64,
        "nextFundingRateDateTime": 1_700_028_800_000_i64,
        "turnoverOf24h": 0.0
    });
    let contract: ContractActive = serde_json::from_value(json).unwrap();
    let funding = parse_funding(&contract).expect("explicit zeros are valid");
    assert_eq!(funding.symbol, "ZERO");
    assert_eq!(funding.rate, 0.0);
    assert_eq!(funding.volume_24h, 0.0);
    assert_eq!(funding.funding_interval, 8);
}

#[test]
fn parse_ticker_uses_best_bid_ask_when_available() {
    let contract_json = serde_json::json!({
        "symbol": "XAGUSDTM",
        "lastTradePrice": 75.87,
        "turnoverOf24h": 289750.05
    });
    let quote_json = serde_json::json!({
        "symbol": "XAGUSDTM",
        "price": "75.87",
        "bestBidPrice": "75.82",
        "bestAskPrice": "75.86"
    });
    let contract: ContractActive = serde_json::from_value(contract_json).unwrap();
    let quote: FuturesTickerItem = serde_json::from_value(quote_json).unwrap();
    let ticker = parse_ticker(&contract, Some(&quote)).expect("ticker parses");
    assert_eq!(ticker.symbol, "XAG");
    assert!((ticker.bid - 75.82).abs() < 1e-12);
    assert!((ticker.ask - 75.86).abs() < 1e-12);
}

#[test]
fn kucoin_futures_all_tickers_parses_official_fixture_quotes() {
    let fixture = include_str!("../../fixtures/kucoin/futures_all_tickers_xbt_eth_usdtm.json");
    let wrap: KucoinResponse<Vec<FuturesTickerItem>> =
        serde_json::from_str(fixture).expect("kucoin futures allTickers fixture");
    let quotes = wrap.into_data("allTickers").expect("kucoin futures quotes");
    let xbt = quotes
        .iter()
        .find(|quote| quote.symbol == "XBTUSDTM")
        .expect("XBTUSDTM quote");
    let contract: ContractActive = serde_json::from_value(serde_json::json!({
        "symbol": "XBTUSDTM",
        "lastTradePrice": 66306.0,
        "turnoverOf24h": 12345.0
    }))
    .unwrap();

    let ticker = parse_ticker(&contract, Some(xbt)).expect("ticker parses");

    assert_eq!(quotes.len(), 2);
    assert_eq!(ticker.symbol, "BTC");
    assert!((ticker.bid - 66306.7).abs() < 1e-12);
    assert!((ticker.ask - 66306.8).abs() < 1e-12);
    assert!((ticker.last - 66306.8).abs() < 1e-12);
    assert!((ticker.volume_24h - 12345.0).abs() < 1e-12);
}

#[test]
fn kucoin_depth20_parses_official_fixture_levels() {
    let fixture = include_str!("../../fixtures/kucoin/futures_depth20_xbtusdtm.json");
    let wrap: KucoinResponse<DepthData> =
        serde_json::from_str(fixture).expect("kucoin futures depth20 fixture");
    let book = wrap.into_data("level2/depth20").expect("kucoin depth20");

    assert_eq!(book.bids.len(), 20);
    assert_eq!(book.asks.len(), 20);
    assert_eq!(book.bids[0], [66665.6, 1856.0]);
    assert_eq!(book.asks[0], [66665.7, 52.0]);
    assert_eq!(book.ts, 1_780_443_856_339_000_000);
}

#[test]
fn parse_mark_index_uses_contracts_active_fields() {
    let json = serde_json::json!({
        "symbol": "XBTUSDTM",
        "markPrice": 89131.36,
        "indexPrice": 89148.12,
        "openInterest": "4955514"
    });
    let contract: ContractActive = serde_json::from_value(json).unwrap();
    let row = parse_mark_index(&contract).expect("mark/index parses");
    assert_eq!(row.symbol, "BTC");
    assert_eq!(row.mark_price, 89_131.36);
    assert_eq!(row.index_price, Some(89_148.12));
    assert_eq!(row.open_interest, Some(4_955_514.0));
    assert_eq!(row.open_interest_value, None);
}

#[test]
fn parse_mark_index_rejects_missing_mark() {
    let json = serde_json::json!({
        "symbol": "ETHUSDTM",
        "markPrice": null,
        "indexPrice": 3000.0
    });
    let contract: ContractActive = serde_json::from_value(json).unwrap();
    assert!(parse_mark_index(&contract).is_none());
}

#[test]
fn contract_spec_reads_official_contract_metadata() {
    let json = serde_json::json!({
        "symbol": "XBTUSDTM",
        "baseCurrency": "XBT",
        "quoteCurrency": "USDT",
        "settleCurrency": "USDT",
        "multiplier": "0.001",
        "lotSize": 1,
        "tickSize": "0.1",
        "maxOrderQty": 1000000,
        "marketMaxOrderQty": 500000,
        "maxLeverage": 125,
        "makerFeeRate": "0.0002",
        "takerFeeRate": "0.0006",
        "status": "Open"
    });
    let contract: ContractActive = serde_json::from_value(json).unwrap();
    let spec = contract_spec(&contract).expect("open contract spec parses");
    assert_eq!(spec.native_symbol, "XBTUSDTM");
    let identity = contract_identity(&contract).expect("contract identity");
    assert_eq!(identity.normalized_symbol, "BTC");
    assert_eq!(identity.quote_currency, "USDT");
    assert_eq!(identity.settle_currency, "USDT");
    assert_eq!(spec.order_unit, 0.001);
    assert_eq!(spec.price_tick, 0.1);
    assert_eq!(spec.lot_size, 1.0);
}

#[test]
fn contract_spec_rejects_closed_or_zero_multiplier_contracts() {
    let closed: ContractActive = serde_json::from_value(serde_json::json!({
        "symbol": "OLDUSDTM",
        "multiplier": "0.01",
        "status": "Closed"
    }))
    .unwrap();
    let zero: ContractActive = serde_json::from_value(serde_json::json!({
        "symbol": "ZEROUSDTM",
        "multiplier": 0,
        "status": "Open"
    }))
    .unwrap();
    assert!(contract_spec(&closed).is_none());
    assert!(contract_spec(&zero).is_none());
}

#[test]
fn parse_spot_tick_pair_symbol() {
    let ticker = SpotTickerItem {
        symbol: "BTC-USDT".into(),
        buy: "30000.0".into(),
        best_bid_size: "0.25".into(),
        sell: "30001.0".into(),
        best_ask_size: "0.5".into(),
        last: "30000.5".into(),
        vol_value: "1000000".into(),
    };
    let tick = parse_spot_tick(&ticker, Some(1_700_000_000_000)).expect("spot tick parses");
    assert_eq!(tick.venue, "kucoin");
    assert_eq!(tick.symbol, "BTC/USDT");
    assert_eq!(tick.bid_size.unwrap().to_string(), "0.25");
    assert_eq!(tick.ask_size.unwrap().to_string(), "0.5");
    assert_eq!(tick.exchange_ts_ms, Some(1_700_000_000_000));
    assert!(tick.received_at_ms > 0);
}

#[test]
fn kucoin_spot_all_tickers_parses_official_fixture_quotes() {
    let fixture = include_str!("../../fixtures/kucoin/spot_market_all_tickers_btc_eth_usdt.json");
    let wrap: KucoinResponse<SpotTickerData> =
        serde_json::from_str(fixture).expect("kucoin spot allTickers fixture");
    let data = wrap
        .into_data("market/allTickers")
        .expect("kucoin spot quotes");
    let btc = data
        .ticker
        .iter()
        .find(|ticker| ticker.symbol == "BTC-USDT")
        .expect("BTC-USDT ticker");

    let exchange_ts_ms = (data.time > 0).then_some(data.time);
    let tick = parse_spot_tick(btc, exchange_ts_ms).expect("BTC-USDT spot tick parses");

    assert_eq!(data.ticker.len(), 2);
    assert_eq!(tick.venue, "kucoin");
    assert_eq!(tick.symbol, "BTC/USDT");
    assert_eq!(tick.bid.to_string(), "66234.8");
    assert_eq!(tick.ask.to_string(), "66234.9");
    assert_eq!(tick.last.to_string(), "66241.4");
    assert_eq!(tick.bid_size.unwrap().to_string(), "0.15866");
    assert_eq!(tick.ask_size.unwrap().to_string(), "0.15548318");
    assert_eq!(
        tick.volume_24h.to_string(),
        "458653195.80618393535818391866"
    );
}

#[test]
fn kucoin_spot_all_tickers_isolates_nullable_quote_rows() {
    let data: SpotTickerData = serde_json::from_value(serde_json::json!({
        "time": 1_700_000_000_000_i64,
        "ticker": [
            {
                "symbol": "BTC-USDT",
                "buy": "30000",
                "bestBidSize": "1",
                "sell": "30001",
                "bestAskSize": "2",
                "last": "30000.5",
                "volValue": "1000000"
            },
            {
                "symbol": "EMPTY-USDT",
                "buy": null,
                "bestBidSize": null,
                "sell": null,
                "bestAskSize": null,
                "last": null,
                "volValue": "0"
            }
        ]
    }))
    .expect("nullable KuCoin quote fields must not reject the full snapshot");

    assert_eq!(data.ticker.len(), 2);
    assert!(parse_spot_tick(&data.ticker[0], Some(data.time)).is_some());
    assert!(parse_spot_tick(&data.ticker[1], Some(data.time)).is_none());
}

#[test]
fn parse_spot_depth_levels_filters_invalid_values() {
    let levels = parse_spot_depth_levels(vec![
        [serde_json::json!("30000"), serde_json::json!("2")],
        [serde_json::json!(30001), serde_json::json!(3)],
        [serde_json::json!("bad"), serde_json::json!(3)],
        [serde_json::json!(30002), serde_json::json!(0)],
    ]);
    assert_eq!(levels, vec![[30000.0, 2.0], [30001.0, 3.0]]);
}

fn merge_object(target: &mut serde_json::Value, patch: &serde_json::Value) {
    let Some(target) = target.as_object_mut() else {
        return;
    };
    if let Some(patch) = patch.as_object() {
        for (key, value) in patch {
            target.insert(key.clone(), value.clone());
        }
    }
}

#[test]
fn parse_ticker_drops_tick_when_no_positive_price() {
    let contract: ContractActive = serde_json::from_value(serde_json::json!({
        "symbol": "XAGUSDTM",
        "lastTradePrice": 0,
        "turnoverOf24h": 289750.05
    }))
    .unwrap();
    assert!(parse_ticker(&contract, None).is_none());
}

#[test]
fn parse_ticker_drops_tick_when_last_unparseable() {
    let contract: ContractActive = serde_json::from_value(serde_json::json!({
        "symbol": "XAGUSDTM",
        "lastTradePrice": "n/a",
        "turnoverOf24h": 289750.05
    }))
    .unwrap();
    assert!(parse_ticker(&contract, None).is_none());
}
