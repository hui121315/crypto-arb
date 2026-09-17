use super::*;
use crate::{InstrumentAssetClass, InstrumentMetadataSource};

pub(crate) fn fixture() -> StockMarketSnapshot {
    let spec = VenueInstrument {
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
        qty_step: Some(0.1),
        min_qty: Some(0.1),
        min_notional: Some(0.5),
        listing_status: InstrumentListingStatus::Trading,
        funding_interval_ms: None,
        builder_dex: None,
        source: InstrumentMetadataSource::OfficialEndpoint,
        source_url: Some(
            "https://docs.kraken.com/exchange/api-reference/spot-websocket-v2/instrument".into(),
        ),
        checked_at_ms: 1000,
        schema_version: Some("kraken-spot-ws-v2-instrument-2026-08-06".into()),
    };
    let quote = StockPeerQuote {
        symbol: "MUX/USD".into(),
        bid: "110".into(),
        ask: "111".into(),
        bid_quantity: Some("10".into()),
        ask_quantity: Some("10".into()),
        source: "ws_push".into(),
        source_at_ms: Some(1000),
        received_at_ms: 1000,
    };
    let mut fx = quote.clone();
    fx.symbol = "USDC/USD".into();
    fx.bid = "1".into();
    fx.ask = "2".into();
    fx.bid_quantity = Some("10000".into());
    fx.ask_quantity = Some("10000".into());
    let buy = StockDexQuote {
        input_mint: SOLANA_USDC.into(),
        output_mint: "verified-stock".into(),
        input_raw: "100000000".into(),
        output_raw: "1000000".into(),
        minimum_output_raw: "1000000".into(),
        router: "local-fixture".into(),
        fee_bps: None,
        fee_mint: None,
        requested_at_ms: 1000,
        received_at_ms: 1000,
        expires_at_ms: None,
    };
    let mut sell = buy.clone();
    sell.input_mint = "verified-stock".into();
    sell.output_mint = SOLANA_USDC.into();
    sell.input_raw = "1000000".into();
    sell.minimum_output_raw = "120000000".into();
    StockMarketSnapshot {
        security: Some(StockSecurity {
            asset: "MU.US".into(),
            ticker: "MU".into(),
            name: "Micron".into(),
            cusip: Some("595112103".into()),
            sessions: vec![],
            order_books: vec![],
            rfq_symbol: "MU.US_USDC_RFQ".into(),
        }),
        peer: Some(StockPeerComparison {
            selection: StockPeerSelection {
                venue: "kraken".into(),
                product: StockPeerProduct::Spot,
                native_symbol: "MUx/USD".into(),
            },
            instrument: Some(spec),
            identity: StockPeerIdentity {
                underlying_verified: true,
                underlying_isin: Some("US5951121038".into()),
                product_isin: Some("CH1473121320".into()),
                issuer: Some("Backed Assets (JE) Limited".into()),
                sources: vec![],
                reason: "different issuer".into(),
            },
            quote: Some(quote),
            // Arithmetic fixture only, not evidence about Kraken's live API units.
            share_unit_verified: true,
            quote_conversion: Some(fx),
            problem: None,
        }),
        comparison: Some(StockComparison {
            asset: "MU.US".into(),
            issuer_docs: "fixture".into(),
            budget_usdc: "100".into(),
            keyed: false,
            mint: StockMintEvidence {
                address: "verified-stock".into(),
                decimals: 6,
                ui_multiplier: "1.2".into(),
                slot: 1,
                chain_time_ms: 1000,
                checked_at_ms: 1000,
                next_change_at_ms: None,
                extensions: vec![],
            },
            buy,
            sell: Some(sell),
            sell_problem: None,
            quantity_limit: None,
        }),
        ..Default::default()
    }
}

#[test]
fn stock_peer_cashflows_use_rebased_shares_and_opposite_fx_sides() {
    let s = fixture();
    let rows = evaluate_peer(&s, 1100);
    assert_eq!(rows[0].shares.as_deref(), Some("1.2"));
    assert_eq!(rows[0].gross_usdc.as_deref(), Some("-34"));
    assert_eq!(rows[1].gross_usdc.as_deref(), Some("-13.2"));
    assert!(rows
        .iter()
        .all(|r| r.blockers.iter().any(|b| b.contains("不是净利润"))));
    assert!(rows
        .iter()
        .all(|r| r.blockers.iter().any(|b| b.contains("不能直接互转"))));
}

#[test]
fn stock_peer_unknown_identity_stale_quotes_and_missing_fx_never_make_profit() {
    let base = fixture();
    for case in 0..15 {
        let mut s = base.clone();
        let p = s.peer.as_mut().unwrap();
        match case {
            0 => p.identity.underlying_verified = false,
            1 => p.quote.as_mut().unwrap().source = "rest_baseline".into(),
            2 => p.quote.as_mut().unwrap().source_at_ms = Some(1101),
            3 => p.quote_conversion = None,
            4 => p.quote_conversion.as_mut().unwrap().received_at_ms = -3000,
            5 => {
                p.quote.as_mut().unwrap().bid_quantity = None;
                p.quote.as_mut().unwrap().ask_quantity = None;
            }
            6 => p.selection.product = StockPeerProduct::Perpetual,
            7 => s.comparison.as_mut().unwrap().mint.next_change_at_ms = Some(1099),
            9 => p.share_unit_verified = false,
            10 => p.quote.as_mut().unwrap().symbol = "MUX/USDT".into(),
            11 => p.quote_conversion.as_mut().unwrap().symbol = "USDC/USDT".into(),
            12 => p.quote.as_mut().unwrap().source_at_ms = None,
            13 => p.problem = Some("subscriptions disabled".into()),
            14 => p.instrument.as_mut().unwrap().venue = "other".into(),
            _ => {
                p.quote_conversion.as_mut().unwrap().bid_quantity = Some("0.1".into());
                p.quote_conversion.as_mut().unwrap().ask_quantity = Some("0.1".into());
            }
        }
        assert!(
            evaluate_peer(&s, 1100)
                .iter()
                .all(|r| r.gross_usdc.is_none()),
            "case {case}"
        );
    }
    assert!(evaluate_peer(&base, 4001)
        .iter()
        .all(|r| r.gross_usdc.is_none()));
}
