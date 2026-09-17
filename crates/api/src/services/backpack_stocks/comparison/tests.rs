use super::*;

#[tokio::test]
#[ignore = "Explicit public read-only stock catalog and token identity verification; no account, WS or funds actions"]
async fn backpack_stock_identity_public_catalog() {
    let service=BackpackStocks::new().unwrap();
    let catalog=service.catalog().await.unwrap();
    let assets=service.read("/api/v1/assets").await.unwrap();
    let mut stocks=Vec::new();
    for asset in ["MU.US","SNDK.US","SPCX.US"] {
        let security=catalog.rows.iter().find(|s|s.asset==asset).unwrap().clone();
        let tokens=super::super::protocol::tokens(&assets,asset).unwrap();
        let snapshot=StockMarketSnapshot{security:Some(security),tokens,token_metadata_at_ms:Some(common::time::now_ms()),observed_at_ms:common::time::now_ms(),..Default::default()};
        let (mint,source,decimals)=issuer(&snapshot).unwrap();
        println!("{asset}: issuer mapping checked, mint={mint}, decimals={decimals}, source={source}");
        assert!(snapshot.rfqs.is_empty() && snapshot.plans.is_empty());
        stocks.push(snapshot);
    }
    let mapped=catalog.rows.iter().filter(|s|shared_types::stocks::identity::backpack_issuer(s).is_ok()).count();
    println!("{} official securities, {mapped} reviewed issuer relationships; not a trade-readiness assertion",catalog.rows.len());
    if let Ok(path)=std::env::var("STOCK_IDENTITY_PUBLIC_CAPTURE_PATH") {
        std::fs::write(path,serde_json::to_vec_pretty(&serde_json::json!({"catalog":catalog,"stocks":stocks})).unwrap()).unwrap();
    }
}

pub(crate) fn snapshot() -> StockMarketSnapshot {
    StockMarketSnapshot {
        security: Some(StockSecurity {
            asset: "MU.US".into(),
            ticker: "MU".into(),
            name: "Micron".into(),
            cusip: Some("595112103".into()),
            sessions: vec![],
            order_books: vec![StockOrderBookMarket {
                symbol: "MU.US_USDC".into(),
                quote: "USDC".into(),
                state: "Open".into(),
                tick_size: "0.01".into(),
                min_quantity: "0.01".into(),
                step_size: "0.01".into(),
            }],
            rfq_symbol: "MU.US_USDC_RFQ".into(),
        }),
        trading_route: Some(StockTradingRoute {
            kind: StockRouteKind::OrderBook,
            session: None,
            symbol: Some("MU.US_USDC".into()),
            reason: "fixture".into(),
            timezone: Some("America/New_York".into()),
            calendar_at_ms: Some(1000),
            valid_until_ms: i64::MAX,
        }),
        tokens: vec![StockChainToken {
            blockchain: "Solana".into(),
            contract_address: Some("MUxEsUKSMACyw5fZf68wxf5FLnZVhtU9CwH8uNNGay1".into()),
            native_decimals: Some(6),
            deposit_enabled: Some(true),
            withdraw_enabled: Some(true),
            minimum_deposit: None,
            minimum_withdrawal: None,
            maximum_withdrawal: None,
            withdrawal_fee: None,
        }],
        ..Default::default()
    }
}

pub(crate) fn comparison() -> StockComparison {
    let mint = StockMintEvidence {
        address: issuer(&snapshot()).unwrap().0.into(),
        decimals: 6,
        ui_multiplier: "1.25".into(),
        slot: 1,
        chain_time_ms: 1000,
        checked_at_ms: 1000,
        next_change_at_ms: None,
        extensions: vec![],
    };
    let buy = StockDexQuote {
        input_mint: SOLANA_USDC.into(),
        output_mint: mint.address.clone(),
        input_raw: "10000000".into(),
        output_raw: "19000".into(),
        minimum_output_raw: "17000".into(),
        router: "metis".into(),
        fee_bps: None,
        fee_mint: None,
        requested_at_ms: 1000,
        received_at_ms: 1100,
        expires_at_ms: None,
    };
    StockComparison {
        asset: "MU.US".into(),
        issuer_docs: issuer(&snapshot()).unwrap().1.into(),
        budget_usdc: "10".into(),
        keyed: false,
        mint,
        buy,
        sell: None,
        sell_problem: None,
        quantity_limit: None,
    }
}

#[test]
fn backpack_stock_comparison_maps_exact_issuer_and_hedges_whole_lots_after_rebase() {
    let mut s = snapshot();
    assert!(issuer(&s).is_ok());
    s.tokens[0].contract_address = Some("MUxEsUKSMACyw5fZf68wxf5FLnZVhtU9CwH8uNNGayl".into());
    assert!(issuer(&s).is_err());
    s = snapshot();
    s.security.as_mut().unwrap().cusip = None;
    assert!(issuer(&s).is_err());
    let c = comparison();
    // Minimum 0.017 native tokens * 1.25 = 0.02125 shares; hedge only 0.02 shares.
    assert_eq!(sell_raw(&snapshot(), &c.mint, &c.buy).unwrap(), "16000");
    let mut small = c.buy.clone();
    small.minimum_output_raw = "7000".into();
    assert!(sell_raw(&snapshot(), &c.mint, &small).is_err());
    let mut rfq = snapshot();
    rfq.trading_route.as_mut().unwrap().kind = StockRouteKind::Rfq;
    rfq.trading_route.as_mut().unwrap().session = Some(StockSession {
        name: "fixture".into(),
        min_quantity: "1".into(),
        max_quantity: Some("10".into()),
        step_size: "1".into(),
    });
    assert!(sell_raw(&rfq, &c.mint, &c.buy)
        .unwrap_err()
        .to_string()
        .contains("最小股数 1"));
    let SellSizeError::Quantity(_, limit) = sell_raw(&rfq, &c.mint, &c.buy).unwrap_err() else {
        panic!("quantity limits must not be classified as transport errors");
    };
    assert_eq!(limit.quoted_shares, "0.02125");
    assert_eq!(limit.min_quantity, "1");
    assert_eq!(limit.step_size, "1");
    let mut large = c.buy.clone();
    large.minimum_output_raw = "16000000".into();
    assert!(sell_raw(&rfq, &c.mint, &large)
        .unwrap_err()
        .to_string()
        .contains("最大股数"));
    for invalid in ["0", "-10", "100000.1", "1.0000001", "NaN"] {
        assert!(budget_raw(&StockQuoteRequest {
            asset: "MU.US".into(),
            budget_usdc: invalid.into(),
            keyed: false
        })
        .is_err());
    }
}

#[test]
fn backpack_stock_comparison_late_quote_cannot_restore_stop_or_a_reopened_same_asset() {
    let service = BackpackStocks::new().unwrap();
    *service.snapshot.write() = snapshot();
    let hub = realtime::WsHub::new(4);
    assert!(service.finish_comparison(0, comparison(), &hub).is_ok());
    service.generation.fetch_add(1, Ordering::SeqCst);
    *service.snapshot.write() = snapshot();
    assert!(service.finish_comparison(0, comparison(), &hub).is_err());
    assert!(service.snapshot().comparison.is_none());
}

#[tokio::test]
async fn backpack_stock_stop_advances_snapshot_even_in_same_clock_millisecond() {
    let service = Arc::new(BackpackStocks::new().unwrap());
    let future_version = common::time::now_ms() + 2;
    service.snapshot.write().observed_at_ms = future_version;
    let stopped = service
        .watch(StockWatchRequest { asset: None }, realtime::WsHub::new(4))
        .await
        .unwrap();
    assert!(stopped.observed_at_ms > future_version);
    assert!(stopped.security.is_none());
}

#[tokio::test]
#[ignore = "Explicit public read-only Backpack + Solana + Jupiter quote verification"]
async fn backpack_stocks_live_chain_comparison() {
    let service = Arc::new(BackpackStocks::new().unwrap());
    let hub = realtime::WsHub::new(32);
    let _viewer = hub.subscribe(realtime::channels::STOCKS);
    let budget = std::env::var("STOCK_PUBLIC_BUDGET_USDC").unwrap_or_else(|_| "10".into());
    let request = StockQuoteRequest {
        asset: "MU.US".into(),
        budget_usdc: budget,
        keyed: false,
    };
    let expected_raw = budget_raw(&request).unwrap();
    let result = tokio::time::timeout(Duration::from_secs(40), async {
        service
            .watch(
                StockWatchRequest {
                    asset: Some("MU.US".into()),
                },
                hub.clone(),
            )
            .await
            .unwrap();
        service.compare(request, hub.clone()).await
    })
    .await
    .unwrap()
    .unwrap();
    let c = result.comparison.as_ref().unwrap();
    if let Ok(path) = std::env::var("STOCK_PUBLIC_CAPTURE_PATH") {
        std::fs::write(path, serde_json::to_vec_pretty(&result).unwrap()).unwrap();
    }
    eprintln!(
        "public quote budget={} route={:?} limit={:?} buy_ms={} reverse_ms={:?}",
        c.budget_usdc,
        result.trading_route.as_ref().map(|r| r.kind),
        c.quantity_limit,
        c.buy.received_at_ms - c.buy.requested_at_ms,
        c.sell
            .as_ref()
            .map(|q| q.received_at_ms - q.requested_at_ms)
    );
    assert!(
        c.sell.is_some(),
        "reverse quote not verified: {:?}",
        c.sell_problem
    );
    assert_eq!(c.mint.decimals, 6);
    assert_eq!(c.buy.input_raw, expected_raw);
    assert!(result
        .trading_route
        .as_ref()
        .is_some_and(
            |r| r.valid_until_ms > common::time::now_ms() && r.kind != StockRouteKind::Unknown
        ));
    assert!(
        result.books.iter().any(|b| b.symbol == "MU.US_USDC"),
        "native Backpack BBO not received"
    );
    eprintln!(
        "public stock comparison: asset={} multiplier={} buy_min={} sell_min={} books={} route={:?}",
        c.asset,
        c.mint.ui_multiplier,
        c.buy.minimum_output_raw,
        c.sell.as_ref().unwrap().minimum_output_raw,
        result.books.len(),
        result.trading_route.as_ref().map(|r|r.kind)
    );
    drop(_viewer);
    drop(service);
}
