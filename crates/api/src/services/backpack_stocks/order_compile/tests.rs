use super::super::plans::tests::fixture_plan;
use super::*;

#[test]
fn stock_plan_compiler_builds_exact_spot_fok_with_stable_id_no_borrow_and_no_futures_fields() {
    let p = fixture_plan(10_000);
    let instruction = compile(&p.request, &p.terms).unwrap();
    assert_eq!(Some(&instruction), p.terms.cex_instruction.as_ref());
    let body = instruction.request_body();
    assert_eq!(instruction.path(), "/api/v1/order");
    assert_eq!(instruction.signing_instruction(), "orderExecute");
    for (key, value) in [
        ("symbol", "MU.US_USDC"),
        ("side", "Ask"),
        ("quantity", "0.02"),
        ("price", "600"),
        ("orderType", "Limit"),
        ("timeInForce", "FOK"),
        ("selfTradePrevention", "RejectTaker"),
    ] {
        assert_eq!(body[key], value, "{key}");
    }
    for key in [
        "autoBorrow",
        "autoBorrowRepay",
        "autoLend",
        "autoLendRedeem",
        "postOnly",
    ] {
        assert_eq!(body[key], false);
    }
    assert!(body.get("reduceOnly").is_none() && body.get("quoteQuantity").is_none());
    let client = body["clientId"].as_u64().unwrap();
    assert!(client > 0 && client <= u32::MAX as u64);
    assert_eq!(compile(&p.request, &p.terms).unwrap(), instruction);
    let mut r = p.request.clone();
    r.direction = StockChainDirection::Sell;
    let bid = compile(&r, &p.terms).unwrap().request_body();
    assert_eq!(bid["side"], "Bid");
    assert_ne!(bid["clientId"], body["clientId"]);
    for (quantity, notional) in [
        ("0.001", "0.6"),
        ("0.015", "9"),
        ("0.02", "12.00002"),
        ("NaN", "12"),
    ] {
        let mut t = p.terms.clone();
        t.cex_shares = quantity.into();
        t.cex_notional_usdc = notional.into();
        assert!(compile(&p.request, &t).is_err());
    }
}

#[test]
fn stock_plan_compiler_accepts_only_bound_original_rfq_ids_and_matching_terms() {
    let mut p = fixture_plan(10_000);
    p.terms.route.kind = StockRouteKind::Rfq;
    p.terms.route.symbol = Some(p.terms.security.rfq_symbol.clone());
    p.terms.route.session = Some(StockSession {
        name: "fixture".into(),
        min_quantity: "0.01".into(),
        max_quantity: Some("10".into()),
        step_size: "0.01".into(),
    });
    p.terms.rfq = Some(StockPlanRfq {
        request_id: "local-rfq-compiler-0001".into(),
        rfq_id: "9007199254740993".into(),
        candidate: StockRfqCandidate {
            quote_id: "9007199254740997".into(),
            taker_price: "600".into(),
            source_at_us: 10_000_000,
            received_at_ms: 10_000,
        },
        expiry_time_ms: 15_000,
    });
    let original = compile(&p.request, &p.terms).unwrap();
    assert_eq!(original.path(), "/api/v1/rfq/accept");
    assert_eq!(original.signing_instruction(), "quoteAccept");
    assert_eq!(
        original.request_body(),
        serde_json::json!({"rfqId":"9007199254740993","quoteId":"9007199254740997"})
    );
    for case in ["price", "symbol", "expiry", "id", "quantity"] {
        let mut t = p.terms.clone();
        match case {
            "price" => t.rfq.as_mut().unwrap().candidate.taker_price = "601".into(),
            "symbol" => t.route.symbol = Some("MU.US_USDC".into()),
            "expiry" => t.rfq.as_mut().unwrap().expiry_time_ms = 10_001,
            "id" => t.rfq.as_mut().unwrap().candidate.quote_id = "0".into(),
            _ => t.cex_shares = "0.025".into(),
        }
        assert!(compile(&p.request, &t).is_err(), "accepted {case}");
    }
}
