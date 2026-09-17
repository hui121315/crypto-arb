use super::*;

pub(in crate::stocks) fn fixture() -> StockMarketSnapshot {
    let mint = StockMintEvidence {
        address: "MU".into(),
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
        output_mint: "MU".into(),
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
    let sell = StockDexQuote {
        input_mint: "MU".into(),
        output_mint: SOLANA_USDC.into(),
        input_raw: "16000".into(),
        output_raw: "10300000".into(),
        minimum_output_raw: "10100000".into(),
        ..buy.clone()
    };
    StockMarketSnapshot {
        security: Some(StockSecurity {
            asset: "MU.US".into(),
            ticker: "MU".into(),
            name: "Micron".into(),
            cusip: None,
            sessions: vec![],
            rfq_symbol: "RFQ".into(),
            order_books: vec![StockOrderBookMarket {
                symbol: "MU.US_USDC".into(),
                quote: "USDC".into(),
                state: "Open".into(),
                tick_size: "0.01".into(),
                step_size: "0.01".into(),
                min_quantity: "0.01".into(),
            }],
        }),
        connected: true,
        trading_route: Some(StockTradingRoute {
            kind: StockRouteKind::OrderBook,
            session: None,
            symbol: Some("MU.US_USDC".into()),
            reason: "fixture".into(),
            timezone: Some("America/New_York".into()),
            calendar_at_ms: Some(1000),
            valid_until_ms: 100_000,
        }),
        books: vec![StockBookQuote {
            symbol: "MU.US_USDC".into(),
            bid: Some("501".into()),
            ask: Some("502".into()),
            bid_quantity: Some("1".into()),
            ask_quantity: Some("1".into()),
            update_id: 1,
            source_at_ms: 1100,
            received_at_ms: 1100,
        }],
        comparison: Some(StockComparison {
            asset: "MU.US".into(),
            issuer_docs: "fixture".into(),
            budget_usdc: "10".into(),
            keyed: false,
            mint,
            buy,
            sell: Some(sell),
            sell_problem: None,
            quantity_limit: None,
        }),
        ..Default::default()
    }
}

#[test]
fn stock_comparison_uses_minimum_scaled_shares_lot_rounding_and_never_counts_dust_as_profit() {
    let s = fixture();
    let rows = evaluate(&s, 1200);
    assert_eq!(rows[0].shares.as_deref(), Some("0.02"));
    assert_eq!(rows[0].remainder_shares.as_deref(), Some("0.00125"));
    assert_eq!(rows[0].gross_usdc.as_deref(), Some("0.02"));
    assert_eq!(rows[1].gross_usdc.as_deref(), Some("0.06"));
    assert!(rows
        .iter()
        .all(|r| r.blockers.iter().any(|b| b.contains("不是可执行净利润"))));
    let mut remainder = s.clone();
    remainder
        .comparison
        .as_mut()
        .unwrap()
        .sell
        .as_mut()
        .unwrap()
        .input_raw = "16001".into();
    let row = &evaluate(&remainder, 1200)[1];
    assert_eq!(row.shares.as_deref(), Some("0.03"));
    assert_eq!(row.gross_usdc.as_deref(), Some("-4.96"));
}

#[test]
fn stock_comparison_rejects_stale_empty_shallow_and_mismatched_quotes() {
    for scenario in 0..10 {
        let mut s = fixture();
        match scenario {
            0 => s.connected = false,
            1 => s.books[0].bid = None,
            2 => s.books[0].bid_quantity = Some("0.001".into()),
            3 => s.comparison.as_mut().unwrap().buy.output_mint = "mu".into(),
            4 => s.comparison.as_mut().unwrap().buy.expires_at_ms = Some(1100),
            5 => s.comparison.as_mut().unwrap().mint.next_change_at_ms = Some(1100),
            6 => s.security.as_mut().unwrap().order_books[0].quote = "USD".into(),
            7 => s.trading_route.as_mut().unwrap().kind = StockRouteKind::Rfq,
            8 => s.trading_route.as_mut().unwrap().valid_until_ms = 1100,
            _ => s.trading_route = None,
        }
        assert!(
            evaluate(&s, 1200)[0].gross_usdc.is_none(),
            "scenario {scenario}"
        );
    }
    assert!(evaluate(&fixture(), 12_000)
        .iter()
        .all(|r| r.gross_usdc.is_none()));
}

#[test]
fn stock_comparison_rfq_uses_taker_fee_price_exact_side_and_conservative_inventory() {
    let mut s = fixture();
    let route = s.trading_route.as_mut().unwrap();
    route.kind = StockRouteKind::Rfq;
    route.symbol = Some("RFQ".into());
    route.session = Some(StockSession {
        name: "regular".into(),
        min_quantity: "0.01".into(),
        max_quantity: Some("100".into()),
        step_size: "0.01".into(),
    });
    s.rfq_connected = true;
    let quote = |side| StockRfq {
        request: StockRfqRequest {
            request_id: format!("fixture-request-{side:?}"),
            asset: "MU.US".into(),
            side,
            quantity: "0.02".into(),
        },
        client_id: 1,
        account_fingerprint: "fixture".into(),
        symbol: "RFQ".into(),
        rfq_id: Some("123".into()),
        phase: StockRfqPhase::Candidate,
        candidate: Some(StockRfqCandidate {
            quote_id: "456".into(),
            taker_price: "501.05".into(),
            source_at_us: 1_100_000,
            received_at_ms: 1100,
        }),
        submission_time_ms: Some(1100),
        expiry_time_ms: Some(2000),
        source_at_us: Some(1_100_000),
        fill_price: None,
        executed_quantity: None,
        executed_quote_quantity: None,
        fills: vec![],
        settlement: Default::default(),
        acceptance: None,
        needs_recheck: false,
        cancel_requested: false,
        created_at_ms: 1000,
        updated_at_ms: 1100,
        problem: None,
    };
    s.rfqs = vec![quote(StockRfqSide::Ask), quote(StockRfqSide::Bid)];
    let rows = evaluate(&s, 1200);
    assert_eq!(rows[0].gross_usdc.as_deref(), Some("0.021"));
    assert_eq!(rows[1].gross_usdc.as_deref(), Some("0.079"));
    assert_eq!(rows[0].remainder_shares.as_deref(), Some("0.00125"));
    assert!(rows[0].blockers.iter().any(|p| p.contains("已含报价费")));
    for scenario in 0..8 {
        let mut bad = s.clone();
        match scenario {
            0 => bad.rfq_connected = false,
            1 => bad.rfqs[0].needs_recheck = true,
            2 => bad.rfqs[0].phase = StockRfqPhase::AcceptedBinding,
            3 => bad.rfqs[0].expiry_time_ms = Some(1200),
            4 => bad.rfqs[0].submission_time_ms = Some(1201),
            5 => bad.rfqs[0].request.quantity = "0.03".into(),
            6 => bad.rfqs[0].request.side = StockRfqSide::Bid,
            _ => bad.rfqs[0].cancel_requested = true,
        }
        assert!(
            evaluate(&bad, 1200)[0].gross_usdc.is_none(),
            "scenario {scenario}"
        );
    }
    s.rfqs[1].request.quantity = "0.01".into();
    assert!(evaluate(&s, 1200)[1].gross_usdc.is_none());
    s.rfqs[1].request.quantity = "0.03".into();
    assert_eq!(evaluate(&s, 1200)[1].gross_usdc.as_deref(), Some("-4.9315"));
}
