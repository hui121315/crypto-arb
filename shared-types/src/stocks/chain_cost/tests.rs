use super::*;

#[test]
fn stock_native_valuation_keeps_full_quote_cost_and_rejects_wrong_stale_or_short_evidence() {
    let evidence = StockNativeValuation {
        native_lamports: "2046280".into(),
        replenishment: None,
        quote: StockDexQuote {
            input_mint: comparison::SOLANA_USDC.into(),
            output_mint: STOCK_WRAPPED_SOL.into(),
            input_raw: "215000".into(),
            output_raw: "2200000".into(),
            minimum_output_raw: "2100000".into(),
            router: "metis".into(),
            fee_bps: Some(2),
            fee_mint: Some(STOCK_WRAPPED_SOL.into()),
            requested_at_ms: 1000,
            received_at_ms: 1100,
            expires_at_ms: Some(1500),
        },
    };
    assert_eq!(
        evidence.usdc_budget("2046280", 1200).as_deref(),
        Some("0.215")
    );
    assert!(evidence.usdc_budget("2046281", 1200).is_none());
    for time in [999, 1050, 1500, 1501] {
        assert!(evidence.usdc_budget("2046280", time).is_none());
    }
    for invalid in [
        "input",
        "output",
        "short",
        "zero",
        "malformed",
        "above_expected",
    ] {
        let mut e = evidence.clone();
        match invalid {
            "input" => e.quote.input_mint = "USDT".into(),
            "output" => e.quote.output_mint = comparison::SOLANA_USDC.into(),
            "short" => e.quote.minimum_output_raw = "2046279".into(),
            "zero" => e.quote.input_raw = "0".into(),
            "malformed" => e.native_lamports = "NaN".into(),
            _ => e.quote.minimum_output_raw = "2200001".into(),
        }
        assert!(e.usdc_budget("2046280", 1200).is_none(), "{invalid}");
    }
}

#[test]
fn stock_native_replenishment_checks_net_native_credit_owner_expiry_and_legacy_serialization() {
    let mut evidence = StockNativeValuation {
        native_lamports: "7000".into(),
        quote: StockDexQuote {
            input_mint: comparison::SOLANA_USDC.into(),
            output_mint: STOCK_WRAPPED_SOL.into(),
            input_raw: "1400".into(),
            output_raw: "15400".into(),
            minimum_output_raw: "14000".into(),
            router: "metis".into(),
            fee_bps: None,
            fee_mint: None,
            requested_at_ms: 1000,
            received_at_ms: 1010,
            expires_at_ms: Some(1500),
        },
        replenishment: None,
    };
    let legacy = serde_json::to_value(&evidence).unwrap();
    assert!(legacy.get("replenishment").is_none());
    assert_eq!(
        serde_json::to_value(
            serde_json::from_value::<StockNativeValuation>(legacy.clone()).unwrap()
        )
        .unwrap(),
        legacy
    );
    assert_eq!(
        evidence.usdc_budget("7000", 1200).as_deref(),
        Some("0.0014")
    );
    assert!(evidence.complete_budget("7000", "wallet", 1200).is_none());
    evidence.replenishment = Some(StockNativeReplenishment {
        wallet_address: "wallet".into(),
        transaction: crate::OnchainUnsignedTransaction::SolanaVersioned {
            transaction_base64: "unsigned".into(),
            request_id: "local-fixture".into(),
            router: "metis".into(),
            mode: "manual".into(),
            last_valid_block_height: Some(100),
            expire_at_ms: Some(1500),
        },
        transaction_fingerprint: "fingerprint".into(),
        network_fee_lamports: "7000".into(),
        wallet_outflow_lamports: "7000".into(),
        wallet_required_lamports: "897880".into(),
        minimum_credit_lamports: "7000".into(),
        simulation_slot: 12,
        checked_at_ms: 1100,
        valid_until_ms: 1500,
    });
    assert_eq!(
        evidence.complete_budget("7000", "wallet", 1200).as_deref(),
        Some("0.0014")
    );
    assert!(evidence.complete_budget("7000", "other", 1200).is_none());
    assert!(evidence.complete_budget("7000", "wallet", 1500).is_none());
    for invalid in ["cost", "credit", "funding", "slot", "time", "transaction"] {
        let mut e = evidence.clone();
        let p = e.replenishment.as_mut().unwrap();
        match invalid {
            "cost" => p.wallet_outflow_lamports = "7001".into(),
            "credit" => p.minimum_credit_lamports = "14000".into(),
            "funding" => p.wallet_required_lamports = "6999".into(),
            "slot" => p.simulation_slot = 0,
            "time" => p.checked_at_ms = 1201,
            _ => p.transaction_fingerprint.clear(),
        }
        assert!(
            e.complete_budget("7000", "wallet", 1200).is_none(),
            "{invalid}"
        );
    }
}
