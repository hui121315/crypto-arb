use super::*;
use crate::services::market_data::MarketSource;
use shared_types::{InstrumentListingStatus, InstrumentMetadataSource, SpotTick};

#[test]
fn stock_identity_sandisk_peer_reuses_ws_and_keeps_issuer_and_quote_boundaries() {
    let now = common::time::now_ms();
    let mut snapshot = super::super::comparison::tests::snapshot();
    let s = snapshot.security.as_mut().unwrap();
    s.asset = "SNDK.US".into(); s.ticker = "SNDK".into(); s.name = "Sandisk Corporation".into();
    s.cusip = Some("80004C200".into()); s.rfq_symbol = "SNDK.US_USDC_RFQ".into(); s.order_books.clear();
    snapshot.tokens[0].contract_address = Some("SNDKbwMUQvZhnLnxLduradgLHG5KrPuKwpnrkkGRhfH".into());
    let mut row = spec(now);
    row.native_symbol = "SNDKx/USD".into(); row.canonical_symbol = "SNDKX".into(); row.display_symbol = "SNDKx/USD".into();
    let cache = Arc::new(MarketDataCache::default());
    let mut stock = tick("SNDKX/USD", now); stock.bid_size = Some("2.5".parse().unwrap());
    let mut fx = tick("USDC/USD",now); fx.bid="0.9998".parse().unwrap(); fx.ask="0.9999".parse().unwrap();
    fx.bid_size=Some(1000.into()); fx.ask_size=Some(1000.into());
    cache.store_spot_ticks(&[stock,fx],MarketSource::WsPush);
    let registry = Arc::new(InstrumentRegistry::default()); registry.upsert(row.clone()).unwrap();
    let service = BackpackStocks::new().unwrap().with_peer_markets(registry,cache);
    *service.snapshot.write()=snapshot.clone();
    let selection=StockPeerSelection {venue:"kraken".into(),product:StockPeerProduct::Spot,native_symbol:"SNDKx/USD".into()};
    let selected=service.watch_peer(StockPeerWatchRequest{asset:"SNDK.US".into(),selection:Some(selection)},&realtime::WsHub::new(8)).unwrap();
    let p=selected.peer.as_ref().unwrap();
    assert!(p.identity.underlying_verified && p.share_unit_verified);
    assert_eq!(p.identity.underlying_isin.as_deref(),Some("US80004C2008"));
    assert_eq!(p.identity.product_isin.as_deref(),Some("CH1500008748"));
    assert!(p.identity.reason.contains("不能直接互相充值"));
    assert!(p.quote.as_ref().unwrap().fresh_ws(now));
    assert_eq!(p.quote.as_ref().unwrap().symbol,"SNDKX/USD");
    assert_eq!(p.quote_conversion.as_ref().unwrap().bid,"0.9998");
    assert!(!p.instrument.as_ref().unwrap().execution_supported);
    assert!(selected.rfqs.is_empty() && selected.plans.is_empty() && selected.peer_preflight.is_none());
    let security=snapshot.security.as_ref().unwrap();
    for case in 0..5 {
        let mut wrong=row.clone();
        match case {
            0=>wrong.native_symbol="MUx/USD".into(),
            1=>wrong.canonical_symbol="MUX".into(),
            2=>wrong.product_type=Some("perp".into()),
            3=>wrong.quote_asset=Some("USDT".into()),
            _=>wrong.asset_class=InstrumentAssetClass::Crypto,
        }
        assert!(!identity(security,Some(&wrong)).underlying_verified,"case {case}");
    }
    if let Ok(path)=std::env::var("STOCK_IDENTITY_PEER_CAPTURE_PATH") {
        std::fs::write(path,serde_json::to_vec_pretty(&selected).unwrap()).unwrap();
    }
}

fn spec(now: i64) -> VenueInstrument {
    VenueInstrument {
        venue: "kraken".into(),
        native_symbol: "MUx/USD".into(),
        canonical_symbol: "MUX".into(),
        display_symbol: "MUx/USD".into(),
        asset_class: InstrumentAssetClass::Equity,
        product_type: Some("spot".into()),
        quote_asset: Some("USD".into()),
        settle_asset: Some("USD".into()),
        margin_asset: None,
        contract_size: Some(1.0),
        execution_supported: false,
        price_tick: Some(0.01),
        qty_step: Some(0.000001),
        min_qty: Some(0.01),
        min_notional: Some(0.5),
        listing_status: InstrumentListingStatus::Trading,
        funding_interval_ms: None,
        builder_dex: None,
        source: InstrumentMetadataSource::OfficialEndpoint,
        source_url: Some(
            "https://docs.kraken.com/exchange/api-reference/spot-websocket-v2/instrument".into(),
        ),
        checked_at_ms: now,
        schema_version: Some("kraken-spot-ws-v2-instrument-2026-08-06".into()),
    }
}

fn tick(symbol: &str, now: i64) -> SpotTick {
    SpotTick {
        venue: "kraken".into(),
        symbol: symbol.into(),
        bid: "110".parse().unwrap(),
        ask: "111".parse().unwrap(),
        last: "110".parse().unwrap(),
        bid_size: None,
        ask_size: Some("2.5".parse().unwrap()),
        volume_24h: 0.into(),
        exchange_ts_ms: Some(now),
        received_at_ms: now,
    }
}

#[tokio::test]
async fn stock_peer_selection_reuses_exact_cache_and_publishes_without_fund_actions() {
    let now = common::time::now_ms();
    let registry = Arc::new(InstrumentRegistry::default());
    registry.upsert(spec(now)).unwrap();
    let cache = Arc::new(MarketDataCache::default());
    let subscriptions =
        Arc::new(crate::services::market_subscriptions::MarketSubscriptions::load(None));
    cache.store_spot_ticks(
        &[tick("MUX/USD", now), tick("USDC/USD", now)],
        MarketSource::WsPush,
    );
    let service = BackpackStocks::new()
        .unwrap()
        .with_peer_markets(registry.clone(), cache.clone())
        .with_peer_feed(Arc::new(exchange::Aggregator::new()), subscriptions.clone());
    service.snapshot.write().security = Some(StockSecurity {
        asset: "MU.US".into(),
        ticker: "MU".into(),
        name: "Micron".into(),
        cusip: Some("595112103".into()),
        sessions: vec![],
        order_books: vec![],
        rfq_symbol: "MU.US_USDC_RFQ".into(),
    });
    let catalog = service
        .peer_catalog(StockPeerCatalogRequest {
            venue: "kraken".into(),
            product: StockPeerProduct::Spot,
            search: "mu".into(),
        })
        .unwrap();
    assert_eq!(catalog.rows.len(), 1);
    assert_eq!(catalog.rows[0].native_symbol, "MUx/USD");
    let hub = realtime::WsHub::new(8);
    let mut frames = hub.subscribe(realtime::channels::STOCKS);
    let selection = StockPeerSelection {
        venue: "kraken".into(),
        product: StockPeerProduct::Spot,
        native_symbol: "MUx/USD".into(),
    };
    let s = service
        .watch_peer(
            StockPeerWatchRequest {
                asset: "MU.US".into(),
                selection: Some(selection.clone()),
            },
            &hub,
        )
        .unwrap();
    let p = s.peer.unwrap();
    assert!(p.identity.underlying_verified);
    assert!(p.share_unit_verified);
    assert!(!p.instrument.as_ref().unwrap().execution_supported);
    assert!(p.quote_conversion.is_some());
    let q = p.quote.unwrap();
    assert_eq!(q.bid_quantity, None);
    assert_eq!(q.ask_quantity.as_deref(), Some("2.5"));
    assert_eq!(q.source, "ws_push");
    assert!(q.fresh_ws(now));
    subscriptions
        .update(&shared_types::MarketSubscriptionPatch {
            venue: "kraken".into(),
            spot_enabled: Some(false),
            perp_enabled: None,
            funding_enabled: None,
        })
        .unwrap();
    service.refresh_peer(now);
    assert!(service.snapshot().peer.unwrap().quote.is_none());
    subscriptions
        .update(&shared_types::MarketSubscriptionPatch {
            venue: "kraken".into(),
            spot_enabled: Some(true),
            perp_enabled: None,
            funding_enabled: None,
        })
        .unwrap();
    let frame = frames.recv().await.unwrap().payload_json().unwrap();
    assert_eq!(frame["peer"]["selection"]["nativeSymbol"], "MUx/USD");
    assert!(s.plans.is_empty() && s.rfqs.is_empty() && s.preflight.is_none());
    assert!(service
        .watch_peer(
            StockPeerWatchRequest {
                asset: "AAPL.US".into(),
                selection: Some(selection.clone())
            },
            &hub
        )
        .is_err());
    let mut wrong = selection.clone();
    wrong.native_symbol = "MUx/USDC".into();
    assert!(service
        .watch_peer(
            StockPeerWatchRequest {
                asset: "MU.US".into(),
                selection: Some(wrong)
            },
            &hub
        )
        .is_err());
    let mut newer = tick("MUX/USD", now + 1);
    newer.bid = "112".parse().unwrap();
    newer.ask = "113".parse().unwrap();
    cache.store_spot_ticks(&[newer], MarketSource::WsPush);
    service.refresh_peer(now + 1);
    assert_eq!(service.snapshot().peer.unwrap().quote.unwrap().bid, "112");
    service.snapshot.write().security.as_mut().unwrap().cusip = Some("wrong-security".into());
    service.refresh_peer(now + 2);
    assert!(
        !service
            .snapshot()
            .peer
            .unwrap()
            .identity
            .underlying_verified
    );
    service
        .watch_peer(
            StockPeerWatchRequest {
                asset: "MU.US".into(),
                selection: None,
            },
            &hub,
        )
        .unwrap();
    service.refresh_peer(now + 3);
    assert!(service.snapshot().peer.is_none());
    if let Ok(path) = std::env::var("STOCK_PEER_CAPTURE_PATH") {
        let capture: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let instrument: VenueInstrument =
            serde_json::from_value(capture["instrument"].clone()).unwrap();
        let stock: SpotTick = serde_json::from_value(capture["stockTick"].clone()).unwrap();
        let fx: SpotTick = serde_json::from_value(capture["fxTick"].clone()).unwrap();
        let captured_at = capture["capturedAtMs"].as_i64().unwrap();
        let registry = Arc::new(InstrumentRegistry::default());
        let cache = Arc::new(MarketDataCache::default());
        registry.upsert(instrument).unwrap();
        cache.store_spot_ticks(&[stock.clone(), fx], MarketSource::WsPush);
        let replay = BackpackStocks::new()
            .unwrap()
            .with_peer_markets(registry, cache);
        let mut security = service.snapshot().security.unwrap();
        security.cusip = Some("595112103".into());
        replay.snapshot.write().security = Some(security);
        replay
            .watch_peer(
                StockPeerWatchRequest {
                    asset: "MU.US".into(),
                    selection: Some(selection),
                },
                &hub,
            )
            .unwrap();
        replay.refresh_peer(captured_at);
        replay.snapshot.write().observed_at_ms = captured_at;
        let result = replay.snapshot();
        assert!(result.peer.as_ref().unwrap().identity.underlying_verified);
        let q = result.peer.as_ref().unwrap().quote.as_ref().unwrap();
        assert_eq!(q.bid, stock.bid.normalize().to_string());
        assert_eq!(q.received_at_ms, stock.received_at_ms);
        assert_eq!(q.source_at_ms, stock.exchange_ts_ms);
        assert!(!result.peer.as_ref().unwrap().share_unit_verified);
        // Replay at the recorded clock, never rewrite an older quote's source time.
        assert!(result.plans.is_empty() && result.rfqs.is_empty() && result.preflight.is_none());
        if let Ok(path) = std::env::var("STOCK_PEER_API_CAPTURE_PATH") {
            std::fs::write(path, serde_json::to_vec_pretty(&result).unwrap()).unwrap();
        }
    }
}

#[test]
fn stock_peer_catalog_is_bounded_and_crypto_name_is_not_equity_evidence() {
    let now = common::time::now_ms();
    let mut row = spec(now);
    let security = StockSecurity {
        asset: "MU.US".into(),
        ticker: "MU".into(),
        name: "Micron".into(),
        cusip: Some("595112103".into()),
        sessions: vec![],
        order_books: vec![],
        rfq_symbol: "MU.US_USDC_RFQ".into(),
    };
    row.asset_class = InstrumentAssetClass::Crypto;
    assert!(!identity(&security, Some(&row)).underlying_verified);
    let registry = Arc::new(InstrumentRegistry::default());
    for index in 0..100 {
        let mut row = spec(now);
        row.native_symbol = format!("MU{index}/USD");
        registry.upsert(row).unwrap();
    }
    let service = BackpackStocks::new()
        .unwrap()
        .with_peer_markets(registry, Arc::new(MarketDataCache::default()));
    let c = service
        .peer_catalog(StockPeerCatalogRequest {
            venue: "kraken".into(),
            product: StockPeerProduct::Spot,
            search: "MU".into(),
        })
        .unwrap();
    assert_eq!(c.matched, 100);
    assert_eq!(c.rows.len(), 80);
}
