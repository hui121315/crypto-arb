use super::*;
use pretty_assertions::assert_eq;

fn fixture() -> (SpotMetaWrapper, Vec<AssetCtx>) {
    serde_json::from_str(include_str!(
        "../../fixtures/hyperliquid/spot_meta_and_asset_ctxs_purr_hfun.json"
    ))
    .expect("official spot metadata fixture")
}

#[test]
fn spot_routes_preserve_official_api_coin_identity() {
    let (meta, _) = fixture();
    let routes = spot_routes(&meta, None).collect::<Vec<_>>();

    assert_eq!(routes[0].coin, "PURR/USDC");
    assert_eq!(routes[0].base, "PURR");
    assert_eq!(routes[1].coin, "@1");
    assert_eq!(routes[1].base, "HFUN");
}

#[test]
fn spot_context_builds_tick_without_l2_subscription() {
    let (meta, ctxs) = fixture();
    let requested = ["PURR/USDC".to_owned()];
    let tick = spot_ticks_from_context(&meta, &ctxs, Some(&requested))
        .into_iter()
        .next()
        .expect("spot tick");
    assert_eq!(tick.symbol, "PURR/USDC");
    assert_eq!(tick.bid.to_string(), "0.209265");
    assert_eq!(tick.ask.to_string(), "0.209265");
    assert_eq!(tick.last.to_string(), "0.21");
    assert_eq!(tick.bid_size, None);
    assert_eq!(tick.ask_size, None);
    assert_eq!(tick.exchange_ts_ms, None);
}

#[test]
fn all_mids_builds_full_market_spot_tick_without_per_symbol_subscription() {
    let (meta, _) = fixture();
    let routes = spot_routes(&meta, None).collect::<Vec<_>>();
    let mids = vec![
        ("PURR/USDC".into(), "0.2095".into()),
        ("@1".into(), "12.5".into()),
    ];

    let ticks = spot_ticks_from_mids("hyperliquid", &routes, &mids);

    assert_eq!(ticks.len(), 2);
    assert_eq!(ticks[0].symbol, "PURR/USDC");
    assert_eq!(ticks[0].last.to_string(), "0.2095");
    assert_eq!(ticks[1].symbol, "HFUN/USDC");
    assert_eq!(ticks[1].last.to_string(), "12.5");
}

#[test]
fn rest_spot_context_joins_by_coin_when_live_rows_are_not_index_aligned() {
    let (meta, mut contexts) = fixture();
    contexts.reverse();
    contexts.insert(
        0,
        AssetCtx {
            coin: Some("@999".into()),
            funding: String::new(),
            mark_px: "999999".into(),
            oracle_px: String::new(),
            open_interest: String::new(),
            mid_px: Some("999999".into()),
            day_ntl_vlm: String::new(),
            impact_pxs: None,
        },
    );

    let ticks = spot_ticks_from_context(&meta, &contexts, None);

    assert_eq!(ticks.len(), 2);
    assert_eq!(ticks[0].symbol, "PURR/USDC");
    assert_eq!(ticks[0].last.to_string(), "0.21");
    assert_eq!(ticks[1].symbol, "HFUN/USDC");
    assert_eq!(ticks[1].last.to_string(), "12.5");
}
