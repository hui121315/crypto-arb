use super::*;

pub(in crate::panels::modules::stocks::view) fn fixture(
    now: i64,
) -> (StockMarketSnapshot, StockFundingPlan) {
    let security = StockSecurity {
        asset: "MU.US".into(),
        ticker: "MU".into(),
        name: "Micron Technology Inc.".into(),
        cusip: Some("595112103".into()),
        sessions: vec![],
        order_books: vec![],
        rfq_symbol: "MU.US_USDC_RFQ".into(),
    };
    let token = StockChainToken {
        blockchain: "Solana".into(),
        contract_address: Some(shared_types::stocks::comparison::SOLANA_USDC.into()),
        native_decimals: Some(6),
        deposit_enabled: Some(true),
        withdraw_enabled: Some(true),
        minimum_deposit: Some("0.5".into()),
        minimum_withdrawal: Some("1".into()),
        maximum_withdrawal: None,
        withdrawal_fee: Some("0.5".into()),
    };
    let need = StockFundingNeed {
        asset: "USDC".into(),
        target: "Solana".into(),
        source: "Backpack".into(),
        required: Some("10".into()),
        available: Some("0".into()),
        shortfall: Some("10".into()),
        source_available: Some("25".into()),
        source_spare: Some("25".into()),
        conservative_source_budget: Some("11".into()),
        source_sufficient: Some(true),
        token: Some(token.clone()),
        metadata_at_ms: Some(now),
        blockers: vec![],
    };
    let wallet = "Hc2D2As4vz9DZVYd3jJMCkiDEKjbUc1W8cf8vrGFfULz";
    let request = StockFundingPlanRequest {
        request_id: "local-visual-funding-0001".into(),
        security_asset: security.asset.clone(),
        funding_asset: need.asset.clone(),
        direction: StockChainDirection::Buy,
        target: StockFundingTarget::Solana,
        wallet_address: wallet.into(),
        preflight_at_ms: now,
    };
    let plan = StockFundingPlan {
        followup: None,
        transfer: None,
        withdrawal: None,
        plan_id: "stock-funding-local-visual-0001".into(),
        request,
        terms: StockFundingPlanTerms {
            account_fingerprint: "local-visual-funding".into(),
            security: security.clone(),
            mint: StockMintEvidence {
                address: "MUxEsUKSMACyw5fZf68wxf5FLnZVhtU9CwH8uNNGay1".into(),
                decimals: 6,
                ui_multiplier: "1.25".into(),
                slot: 123456,
                chain_time_ms: now,
                checked_at_ms: now,
                next_change_at_ms: None,
                extensions: vec![],
            },
            token,
            need: need.clone(),
            destination: wallet.into(),
            deposit_address: None,
            withdrawal_capacity: Some(StockWithdrawalCapacity {
                asset: "USDC".into(),
                quantity: "25".into(),
                checked_at_ms: now,
            }),
            quantity: "10.5".into(),
            minimum_credit_raw: "10000000".into(),
            source_budget: "11".into(),
            created_at_ms: now,
            valid_until_ms: now + 30_000,
        },
        phase: StockFundingPlanPhase::Reserved,
        revision: 1,
        updated_at_ms: now,
    };
    let mut snapshot = StockMarketSnapshot {
        security: Some(security),
        observed_at_ms: now,
        ..Default::default()
    };
    snapshot.preflight = Some(StockPreflight {
        asset: "MU.US".into(),
        wallet_address: Some(wallet.into()),
        checked_at_ms: now,
        valid_until_ms: now + 3000,
        price_basis: StockPriceBasis::from_snapshot(&snapshot),
        spot_taker_fee_pct: None,
        account_at_ms: Some(now),
        wallet_at_ms: Some(now),
        directions: vec![],
        funding: vec![StockFundingDirection {
            direction: StockChainDirection::Buy,
            needs: vec![need],
        }],
        problems: vec![],
    });
    (snapshot, plan)
}

#[test]
fn stock_funding_save_checks_current_wallet_inventory_and_shared_reservations() {
    let now = 10_000;
    let (s, p) = fixture(now);
    assert!(can_save(
        &s,
        &p.request,
        &p.terms.need,
        &p.request.wallet_address,
        now
    ));
    assert!(!can_save(&s, &p.request, &p.terms.need, "another", now));
    for phase in [
        StockFundingPlanPhase::Withdrawing,
        StockFundingPlanPhase::Received,
    ] {
        let mut other = s.clone();
        let mut held = p.clone();
        held.phase = phase;
        held.terms.valid_until_ms = now - 1;
        other.funding_plans = vec![held];
        assert!(
            !can_save(
                &other,
                &p.request,
                &p.terms.need,
                &p.request.wallet_address,
                now
            ),
            "submitted hold must not expire"
        );
    }
    assert!(!can_save(
        &s,
        &p.request,
        &p.terms.need,
        &p.request.wallet_address,
        now + 30_001
    ));
    for case in 0..6 {
        let mut s = s.clone();
        let mut n = p.terms.need.clone();
        match case {
            0 => s.funding_plans = vec![p.clone()],
            1 => s.funding_problem = Some("无法读取资金日志".into()),
            2 => s.preflight = None,
            3 => n.source_sufficient = Some(false),
            4 => n.token.as_mut().unwrap().withdraw_enabled = Some(false),
            _ => n.shortfall = None,
        }
        assert!(
            !can_save(&s, &p.request, &n, &p.request.wallet_address, now),
            "case {case}"
        );
    }
    let mut s = s;
    let mut cancelled = p.clone();
    cancelled.phase = StockFundingPlanPhase::Cancelled;
    s.funding_plans = vec![cancelled];
    assert!(can_save(
        &s,
        &p.request,
        &p.terms.need,
        &p.request.wallet_address,
        now
    ));
}
