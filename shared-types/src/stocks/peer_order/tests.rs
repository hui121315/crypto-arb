use super::*;

fn snapshot() -> StockMarketSnapshot {
    let mut s = super::super::peers::tests::fixture();
    let issuer = identity::backpack_issuer(s.security.as_ref().unwrap()).unwrap();
    s.tokens=serde_json::from_value(serde_json::json!([{"blockchain":"Solana","contractAddress":issuer.solana_mint,"nativeDecimals":issuer.decimals}])).unwrap();
    let c = s.comparison.as_mut().unwrap();
    c.mint.address = issuer.solana_mint.into();
    c.buy.output_mint = issuer.solana_mint.into();
    c.sell.as_mut().unwrap().input_mint = issuer.solana_mint.into();
    s
}

fn request(s: &StockMarketSnapshot, direction: StockChainDirection) -> StockPeerOrderCheckRequest {
    StockPeerOrderCheckRequest {
        asset: "MU.US".into(),
        selection: s.peer.as_ref().unwrap().selection.clone(),
        direction,
    }
}

#[test]
fn stock_peer_order_compiler_uses_exact_shares_and_non_trading_flags() {
    let s = snapshot();
    for direction in [StockChainDirection::Buy, StockChainDirection::Sell] {
        let mut d = prepare_peer_order_check(&s, request(&s, direction), 1100).unwrap();
        assert_eq!(d.quantity, "1.2");
        assert_eq!(
            d.limit_price,
            if direction == StockChainDirection::Buy {
                "110"
            } else {
                "111"
            }
        );
        let frame = d.kraken_validation("fixture-token", 7, 1200).unwrap();
        let p = &frame["params"];
        assert_eq!(
            p["side"],
            if direction == StockChainDirection::Buy {
                "sell"
            } else {
                "buy"
            }
        );
        assert_eq!(p["symbol"], "MUx/USD");
        assert_eq!(p["order_type"], "limit");
        assert_eq!(p["time_in_force"], "fok");
        assert_eq!(p["validate"], true);
        assert_eq!(p["margin"], false);
        assert_eq!(p["fee_preference"], "quote");
        assert_eq!(p["cl_ord_id"], "sv0000000000000007");
        assert_eq!(p["deadline"], "1970-01-01T00:00:03.200Z");
        d.quantity = "0.123456789012345678".into();
        d.limit_price = "999999.123456789012".into();
        let frame = d.kraken_validation("fixture-token", 8, 1200).unwrap();
        assert_eq!(frame["params"]["order_qty"].to_string(), d.quantity);
        assert_eq!(frame["params"]["limit_price"].to_string(), d.limit_price);
        assert!(d.kraken_validation("fixture-token", 8, 11_101).is_err());
    }
    let mut injected = serde_json::to_value(request(&s, StockChainDirection::Buy)).unwrap();
    injected["validate"] = serde_json::json!(false);
    assert!(serde_json::from_value::<StockPeerOrderCheckRequest>(injected).is_err());
}

#[test]
fn stock_peer_order_compiler_rejects_wrong_identity_tick_and_stale_quotes() {
    let base = snapshot();
    for case in 0..8 {
        let mut s = base.clone();
        match case {
            0 => s.security.as_mut().unwrap().cusip = Some("wrong".into()),
            1 => s.comparison.as_mut().unwrap().mint.address = "other-stock".into(),
            2 => s.peer.as_mut().unwrap().quote.as_mut().unwrap().bid = "110.001".into(),
            3 => {
                s.peer
                    .as_mut()
                    .unwrap()
                    .quote
                    .as_mut()
                    .unwrap()
                    .source_at_ms = Some(-3000)
            }
            4 => s.peer.as_mut().unwrap().quote_conversion = None,
            5 => {
                s.peer
                    .as_mut()
                    .unwrap()
                    .instrument
                    .as_mut()
                    .unwrap()
                    .schema_version = None
            }
            6 => s.tokens.clear(),
            _ => s.peer.as_mut().unwrap().selection.native_symbol = "MUx/USDT".into(),
        }
        assert!(
            prepare_peer_order_check(&s, request(&s, StockChainDirection::Buy), 1100).is_err(),
            "case {case}"
        );
    }
}
