use super::*;

fn inputs() -> (StockStablecoinRequest, StockWalletEvidence, StockDexQuote) {
    (
        StockStablecoinRequest {
            asset: "MU.US".into(),
            wallet_address: "fixture-wallet".into(),
            input_usdt: "10.123456".into(),
            target_usdc: "10".into(),
            keyed: false,
        },
        StockWalletEvidence {
            owner: "fixture-wallet".into(),
            mint: STOCK_SOLANA_USDT.into(),
            stock_raw: Some("10123456".into()),
            usdc_raw: Some("0".into()),
            sol_lamports: Some("1000000".into()),
            checked_at_ms: 1000,
            problems: vec![],
        },
        StockDexQuote {
            input_mint: STOCK_SOLANA_USDT.into(),
            output_mint: comparison::SOLANA_USDC.into(),
            input_raw: "10123456".into(),
            output_raw: "9900000".into(),
            minimum_output_raw: "9800000".into(),
            router: "metis".into(),
            fee_bps: None,
            fee_mint: None,
            requested_at_ms: 1000,
            received_at_ms: 1100,
            expires_at_ms: None,
        },
    )
}

#[test]
fn stock_stablecoin_preview_keeps_actual_ratio_and_never_increases_input() {
    let (r, w, q) = inputs();
    assert_eq!(r.amounts_raw().unwrap(), (10_123_456, 10_000_000));
    let p = stablecoin_preview(r, w, q, None, vec![], 1200).unwrap();
    assert_eq!(p.minimum_usdc, "9.8");
    assert_eq!(p.shortfall_usdc, "0.2");
    assert_eq!(p.quote.input_raw, "10123456");
    assert_eq!(p.input_sufficient, Some(true));
    assert_eq!(p.after_native_cost_usdc, None);
    assert!(p.blockers.iter().any(|b| b.contains("路由费用")));
    assert!(p.current(1200));
    assert!(!p.current(p.valid_until_ms));
    let mut w = p.wallet.clone();
    w.stock_raw = None;
    let unknown =
        stablecoin_preview(p.request.clone(), w, p.quote.clone(), None, vec![], 1200).unwrap();
    assert_eq!(unknown.input_sufficient, None);
    let mut w = p.wallet;
    w.stock_raw = Some("1".into());
    assert_eq!(
        stablecoin_preview(p.request, w, p.quote, None, vec![], 1200)
            .unwrap()
            .input_sufficient,
        Some(false)
    );
}

#[test]
fn stock_stablecoin_preview_rejects_mismatched_identity_and_fractional_raw_units() {
    for amount in ["0", "-1", "1.0000001", "1e2", "NaN", "1000001"] {
        let (mut r, _, _) = inputs();
        r.input_usdt = amount.into();
        assert!(r.amounts_raw().is_err(), "{amount}");
    }
    for change in 0..5 {
        let (r, mut w, mut q) = inputs();
        match change {
            0 => q.output_mint = STOCK_SOLANA_USDT.into(),
            1 => q.input_raw = "10123457".into(),
            2 => w.owner = "other".into(),
            3 => q.minimum_output_raw = "99900000".into(),
            _ => q.received_at_ms = 1300,
        }
        assert!(stablecoin_preview(r, w, q, None, vec![], 1200).is_err());
    }
    let (r, w, q) = inputs();
    let stale = stablecoin_preview(r, w, q, None, vec![], 40_000).unwrap();
    assert_eq!(stale.input_sufficient, None);
    assert!(!stale.current(40_000));
    assert!(stale.blockers.iter().any(|b| b.contains("报价已过期")));
}
