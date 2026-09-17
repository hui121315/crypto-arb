use super::*;
use pretty_assertions::assert_eq;

#[test]
fn builder_dex_rows_use_clean_symbol_and_dex_venue() {
    let meta = UniverseWrapper {
        universe: vec![universe_entry("xyz:MU")],
    };
    let ctxs = vec![asset_ctx("0.0001", "700", "123")];
    let rows = ticker_rows("hyperliquid:xyz", &meta, &ctxs);
    assert_eq!(rows[0].symbol, "MU");
    assert_eq!(rows[0].exchange, "hyperliquid:xyz");
}

#[test]
fn hyperliquid_l2book_parses_official_fixture_levels() {
    let raw = include_str!("../../fixtures/hyperliquid/l2book_btc.json");
    let book: L2Book = serde_json::from_str(raw).unwrap();

    let bids = parse_levels(&book.levels[0]);
    let asks = parse_levels(&book.levels[1]);

    assert_eq!(book.time, 1_754_450_974_231);
    assert_eq!(bids[0], [113_377.0, 7.6699]);
    assert_eq!(bids[1], [113_376.0, 4.13714]);
    assert_eq!(asks[0], [113_397.0, 0.11543]);
}

#[test]
fn hyperliquid_meta_and_asset_ctxs_parses_official_fixture_metadata_ticker_funding() {
    let raw = include_str!("../../fixtures/hyperliquid/meta_and_asset_ctxs_btc_eth.json");
    let official: serde_json::Value = serde_json::from_str(raw).unwrap();
    assert_eq!(official[0]["universe"][0]["szDecimals"], 5);
    assert_eq!(official[0]["universe"][0]["maxLeverage"], 50);
    assert_eq!(official[0]["universe"][2]["onlyIsolated"], true);
    assert_eq!(official[0]["universe"][3]["isDelisted"], true);

    let (meta, ctxs): (UniverseWrapper, Vec<AssetCtx>) = serde_json::from_str(raw).unwrap();

    assert_eq!(meta.universe[0].name, "BTC");

    let ticker = ticker_rows("hyperliquid", &meta, &ctxs);
    assert_eq!(ticker[0].symbol, "BTC");
    assert_eq!(ticker[0].bid, 14.3047);
    assert_eq!(ticker[0].ask, 14.3444);
    assert_eq!(ticker[0].volume_24h, 1_169_046.294_06);

    let funding = funding_rows("hyperliquid", &meta, &ctxs, &HashMap::new());
    assert_eq!(funding[0].symbol, "BTC");
    assert_eq!(funding[0].funding_interval, 1);
    assert_eq!(funding[0].predicted_rate, None);
    assert!((funding[0].rate - 0.0000125).abs() < 1e-15);
    assert!((funding[0].rate_8h - 0.0001).abs() < 1e-12);
}

#[test]
fn parse_spot_tick_from_spot_meta_ctx() {
    let ctx = AssetCtx {
        coin: Some("BTC/USDC".into()),
        funding: String::new(),
        mark_px: "30000".into(),
        oracle_px: "30001".into(),
        open_interest: "1234".into(),
        mid_px: Some("29999".into()),
        day_ntl_vlm: "1000000".into(),
        impact_pxs: Some(["29998".into(), "30001".into()]),
    };
    let tick = parse_spot_tick("BTC", "USDC", &ctx).expect("spot tick parses");
    assert_eq!(tick.symbol, "BTC/USDC");
    assert_eq!(tick.bid.to_string(), "29998");
    assert_eq!(tick.ask.to_string(), "30001");
}

#[test]
fn hyperliquid_spot_meta_and_asset_ctxs_parses_official_fixture() {
    let raw = include_str!("../../fixtures/hyperliquid/spot_meta_and_asset_ctxs_purr_hfun.json");
    let (meta, ctxs): (SpotMetaWrapper, Vec<AssetCtx>) = serde_json::from_str(raw).unwrap();
    let token_names = spot_token_names(&meta.tokens);

    // Replicates the adapter's identity-aware spot context join.
    let contexts_by_coin = spot_contexts_by_coin(&ctxs);
    let ticks: Vec<SpotTick> = meta
        .universe
        .iter()
        .enumerate()
        .filter_map(|(idx, entry)| {
            let ctx = spot_context_for_entry(entry, idx, &ctxs, contexts_by_coin.as_ref())?;
            let (base, quote) = spot_pair(entry, &token_names)?;
            parse_spot_tick(&base, &quote, ctx)
        })
        .collect();

    assert_eq!(ticks.len(), 2);

    let purr = &ticks[0];
    assert_eq!(purr.venue, "hyperliquid");
    assert_eq!(purr.symbol, "PURR/USDC");
    // No impactPxs on the spot ctx, so BBO falls back to midPx; last is markPx.
    assert_eq!(purr.bid.to_string(), "0.209265");
    assert_eq!(purr.ask.to_string(), "0.209265");
    assert_eq!(purr.last.to_string(), "0.21");
    assert_eq!(purr.volume_24h.to_string(), "1234567.89");
    // Hyperliquid spot ctx carries neither size nor exchange timestamp.
    assert_eq!(purr.bid_size, None);
    assert_eq!(purr.ask_size, None);
    assert_eq!(purr.exchange_ts_ms, None);
    assert!(purr.received_at_ms > 0);

    // `@1` is resolved to its token pair via the spot token index map.
    assert_eq!(ticks[1].symbol, "HFUN/USDC");
    assert_eq!(ticks[1].last.to_string(), "12.5");
}

#[test]
fn spot_pair_uses_token_names_for_at_symbols() {
    let token_names = HashMap::from([(0, "PURR".to_owned()), (1, "USDC".to_owned())]);
    let entry = SpotUniverseEntry {
        name: "@0".into(),
        tokens: vec![0, 1],
    };
    assert_eq!(
        spot_pair(&entry, &token_names),
        Some(("PURR".to_owned(), "USDC".to_owned()))
    );
}

#[test]
fn predicted_fundings_maps_hl_perp_only() {
    let raw = r#"[
        ["BTC", [
            ["BinPerp", {"fundingRate":"0.0001","nextFundingTime":1700028800000,"fundingIntervalHours":4}],
            ["HlPerp", {"fundingRate":"-0.00003717","nextFundingTime":1700001000000,"fundingIntervalHours":1}]
        ]],
        ["ETH", [
            ["HlPerp", {"fundingRate":"0.000001","nextFundingTime":1700002000000,"fundingIntervalHours":1}]
        ]]
    ]"#;
    let parsed: PredictedFundingsResponse = serde_json::from_str(raw).unwrap();
    let map = build_predicted_map(&parsed);
    assert!((map["BTC"].rate + 0.00003717).abs() < 1e-12);
    assert_eq!(map["BTC"].next_funding_ms, 1_700_001_000_000);
    assert!((map["ETH"].rate - 0.000001).abs() < 1e-12);
}

#[test]
fn hyperliquid_predicted_fundings_parses_official_fixture_hl_perp() {
    let raw = include_str!("../../fixtures/hyperliquid/predicted_fundings_avax.json");
    let parsed: PredictedFundingsResponse = serde_json::from_str(raw).unwrap();
    let map = build_predicted_map(&parsed);

    assert_eq!(map.len(), 1);
    assert!((map["AVAX"].rate - 0.0000125).abs() < 1e-15);
    assert_eq!(map["AVAX"].next_funding_ms, 1_733_958_000_000);
}

#[test]
fn predicted_fundings_skips_null_venue_rows_from_live_schema() {
    let raw = r#"[
        ["BTC", [
            ["BinPerp", null],
            ["HlPerp", {"fundingRate":"0.0000125","nextFundingTime":1784512800000,"fundingIntervalHours":1}]
        ]],
        ["ETH", [["HlPerp", null]]]
    ]"#;
    let parsed: PredictedFundingsResponse = serde_json::from_str(raw).unwrap();
    let map = build_predicted_map(&parsed);

    assert_eq!(map.len(), 1);
    assert!((map["BTC"].rate - 0.0000125).abs() < 1e-15);
    assert!(!map.contains_key("ETH"));
}

#[test]
fn parse_mark_index_uses_active_asset_ctx_fields() {
    let ctx = asset_ctx("0.0001", "700", "123");
    let row = parse_mark_index_for_venue("hyperliquid:xyz", "MU", &ctx).expect("mark/index parses");
    assert_eq!(row.symbol, "MU");
    assert_eq!(row.exchange, "hyperliquid:xyz");
    assert_eq!(row.mark_price, 700.0);
    assert_eq!(row.index_price, Some(701.0));
    assert_eq!(row.open_interest, Some(10.0));
    assert_eq!(row.open_interest_value, None);
}

#[test]
fn ticker_rows_drop_tick_when_mark_nonpositive() {
    let meta = UniverseWrapper {
        universe: vec![universe_entry("MU")],
    };
    let ctxs = vec![asset_ctx("0.0001", "0", "123")];
    assert!(ticker_rows("hyperliquid", &meta, &ctxs).is_empty());
}

#[test]
fn ticker_rows_drop_tick_when_mark_unparseable() {
    let meta = UniverseWrapper {
        universe: vec![universe_entry("MU")],
    };
    let ctxs = vec![asset_ctx("0.0001", "n/a", "123")];
    assert!(ticker_rows("hyperliquid", &meta, &ctxs).is_empty());
}

fn universe_entry(name: &str) -> UniverseEntry {
    UniverseEntry { name: name.into() }
}

fn asset_ctx(funding: &str, mark_px: &str, volume: &str) -> AssetCtx {
    AssetCtx {
        coin: None,
        funding: funding.into(),
        mark_px: mark_px.into(),
        oracle_px: "701".into(),
        open_interest: "10".into(),
        mid_px: None,
        day_ntl_vlm: volume.into(),
        impact_pxs: None,
    }
}
