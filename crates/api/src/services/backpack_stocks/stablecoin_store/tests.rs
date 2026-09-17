use super::*;

// Synthetic HTTP/RPC capture: fee=0 is explicit fixture data, not a live fee assumption.
pub(in crate::services::backpack_stocks) fn fixture(now: i64) -> StockStablecoinPreview {
    let mut p: StockStablecoinPreview =
        serde_json::from_str(include_str!("fixtures/preview.json")).unwrap();
    p.request.target_usdc = "9.5".into();
    p.wallet.checked_at_ms = now;
    p.quote.requested_at_ms = now;
    p.quote.received_at_ms = now;
    let c = p.cost.as_mut().unwrap();
    c.quote = p.quote.clone();
    c.checked_at_ms = now;
    c.valid_until_ms = now + 10_000;
    c.mint.checked_at_ms = now;
    c.mint.chain_time_ms = now;
    stablecoin_preview(p.request, p.wallet, p.quote, p.cost, vec![], now).unwrap()
}

pub(in crate::services::backpack_stocks) fn request(
    p: &StockStablecoinPreview,
) -> StockStablecoinPlanRequest {
    StockStablecoinPlanRequest {
        request_id: "local-stock-stablecoin-0001".into(),
        conversion: p.request.clone(),
        preview_at_ms: p.checked_at_ms,
        transaction_fingerprint: p.cost.as_ref().unwrap().transaction_fingerprint.clone(),
    }
}

#[test]
fn stock_stablecoin_store_reserve_restart_cancel_and_expiry_never_recreate_funds() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("stablecoin.jsonl");
    let now = 100_000;
    let claims = Arc::new(WalletClaims::default());
    let store = StablecoinStore::load(Some(path.clone()), claims.clone());
    let p = fixture(now);
    assert!(p.can_reserve(now));
    let r = request(&p);
    let plan = store.reserve(r.clone(), p.clone(), now).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(store.reserve(r.clone(), p.clone(), now).unwrap(), plan);
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert!(claims
        .check("solana", &r.conversion.wallet_address, now)
        .is_err());
    let mut wrong = r.clone();
    wrong.conversion.input_usdt = "11".into();
    assert!(store.previous(&wrong).is_err());
    assert!(store.reserve(wrong, p.clone(), now).is_err());
    drop(store);
    drop(claims);

    let claims = Arc::new(WalletClaims::default());
    let store = StablecoinStore::load(Some(path.clone()), claims.clone());
    assert_eq!(store.records(), vec![plan.clone()]);
    assert!(claims
        .check("solana", &r.conversion.wallet_address, now)
        .is_err());
    assert!(store
        .cancel(
            &StockPlanRevisionRequest {
                plan_id: plan.plan_id.clone(),
                revision: 0
            },
            now
        )
        .is_err());
    let cancel = StockPlanRevisionRequest {
        plan_id: plan.plan_id,
        revision: 1,
    };
    let cancelled = store.cancel(&cancel, now + 1).unwrap();
    assert_eq!(cancelled.phase, StockStablecoinPlanPhase::Cancelled);
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(store.cancel(&cancel, now + 2).unwrap(), cancelled);
    assert_eq!(
        store.reserve(r.clone(), p.clone(), now + 2).unwrap(),
        cancelled
    );
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert!(claims
        .check("solana", &r.conversion.wallet_address, now)
        .is_ok());

    let mut next = r.clone();
    next.request_id = "local-stock-stablecoin-0002".into();
    let second = store.reserve(next.clone(), p.clone(), now + 2).unwrap();
    assert_eq!(
        second.phase_at(p.valid_until_ms),
        StockStablecoinPlanPhase::Expired
    );
    assert!(claims
        .check("solana", &r.conversion.wallet_address, p.valid_until_ms)
        .is_ok());
    let mut expired = next.clone();
    expired.request_id = "local-stock-stablecoin-expired".into();
    assert!(store.reserve(expired, p.clone(), p.valid_until_ms).is_err());
    drop(store);
    drop(claims);
    let claims = Arc::new(WalletClaims::default());
    let restored = StablecoinStore::load(Some(path), claims.clone());
    assert!(restored.problem().is_none());
    assert_eq!(restored.previous(&r).unwrap(), Some(cancelled));
    assert_eq!(restored.previous(&next).unwrap(), Some(second));
    assert!(claims
        .check("solana", &r.conversion.wallet_address, p.valid_until_ms)
        .is_ok());
}

#[test]
fn stock_stablecoin_store_respects_other_modules_and_preserves_corrupt_journals() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("stablecoin.jsonl");
    let now = 100_000;
    let claims = Arc::new(WalletClaims::default());
    let store = StablecoinStore::load(Some(path.clone()), claims.clone());
    let p = fixture(now);
    let r = request(&p);
    let owner = Owner::new(Module::CrossChain, "other-local-plan");
    claims
        .commit(
            owner.clone(),
            Some(Hold::wallet("solana", &r.conversion.wallet_address, None).unwrap()),
            now,
            || Ok(()),
        )
        .unwrap();
    assert!(store
        .reserve(r.clone(), p.clone(), now)
        .unwrap_err()
        .contains("占用"));
    assert!(!path.exists());
    claims.commit(owner, None, now, || Ok(())).unwrap();
    let plan = store.reserve(r.clone(), p.clone(), now).unwrap();
    let second = StablecoinStore::load(Some(path.clone()), Arc::new(WalletClaims::default()));
    assert!(second.problem().unwrap().contains("其他实例"));
    drop(second);
    drop(store);
    drop(claims);
    let bytes = std::fs::read(&path).unwrap();
    plan_store::options(true)
        .open(&path)
        .unwrap()
        .write_all(b"{")
        .unwrap();
    let damaged = std::fs::read(&path).unwrap();
    let claims = Arc::new(WalletClaims::default());
    let store = StablecoinStore::load(Some(path.clone()), claims.clone());
    assert!(store.problem().unwrap().contains("尾部"));
    assert!(store.previous(&r).is_err());
    assert!(store.reserve(r.clone(), p.clone(), now).is_err());
    assert!(claims
        .check("solana", &r.conversion.wallet_address, now)
        .is_err());
    assert_eq!(std::fs::read(&path).unwrap(), damaged);
    drop(store);

    let mut entry: Entry = serde_json::from_slice(&bytes).unwrap();
    entry.plan.preview.minimum_usdc = "1000".into();
    let tampered = tmp.path().join("tampered.jsonl");
    let mut bytes = serde_json::to_vec(&entry).unwrap();
    bytes.push(b'\n');
    std::fs::write(&tampered, &bytes).unwrap();
    assert!(
        StablecoinStore::load(Some(tampered), Arc::new(WalletClaims::default()))
            .problem()
            .is_some()
    );
    let orphan = tmp.path().join("orphan.jsonl");
    let mut cancelled = plan;
    cancelled.phase = StockStablecoinPlanPhase::Cancelled;
    cancelled.revision = 2;
    append(&orphan, &cancelled).unwrap();
    assert!(
        StablecoinStore::load(Some(orphan), Arc::new(WalletClaims::default()))
            .problem()
            .unwrap()
            .contains("初始计划")
    );
}

#[test]
fn stock_stablecoin_reservation_requires_known_fees_and_net_target_after_sol_budget() {
    let now = 100_000;
    let p = fixture(now);
    let mut c = p.cost.clone().unwrap();
    c.network_fee_lamports = None;
    let unknown = stablecoin_preview(
        p.request.clone(),
        p.wallet.clone(),
        p.quote.clone(),
        Some(c),
        vec![],
        now,
    )
    .unwrap();
    assert!(!unknown.can_reserve(now));
    let mut c = p.cost.clone().unwrap();
    c.wallet_debit_lamports = Some("5000".into());
    let mut q = p.quote.clone();
    q.input_mint = shared_types::stocks::comparison::SOLANA_USDC.into();
    q.output_mint = STOCK_WRAPPED_SOL.into();
    q.input_raw = "20000".into();
    q.output_raw = "10000".into();
    q.minimum_output_raw = "10000".into();
    c.native_valuation = Some(StockNativeValuation {
        native_lamports: "5000".into(),
        quote: q,
        replenishment: Some(StockNativeReplenishment {
            wallet_address: p.request.wallet_address.clone(),
            transaction: c.transaction.clone().unwrap(),
            transaction_fingerprint: "synthetic-budget-proof".into(),
            network_fee_lamports: "0".into(),
            wallet_outflow_lamports: "0".into(),
            wallet_required_lamports: "0".into(),
            minimum_credit_lamports: "10000".into(),
            simulation_slot: 12,
            checked_at_ms: now,
            valid_until_ms: now + 10_000,
        }),
    });
    let mut r = p.request.clone();
    r.target_usdc = "9.89".into();
    let net = stablecoin_preview(r, p.wallet, p.quote, Some(c), vec![], now).unwrap();
    assert_eq!(net.after_native_cost_usdc.as_deref(), Some("9.88"));
    assert_eq!(net.shortfall_usdc, "0.01");
    assert!(!net.can_reserve(now));
    let mut c = net.cost.unwrap();
    c.native_valuation
        .as_mut()
        .unwrap()
        .replenishment
        .as_mut()
        .unwrap()
        .valid_until_ms = now + 500;
    let mut r = net.request;
    r.target_usdc = "9".into();
    let bounded = stablecoin_preview(r, net.wallet, net.quote, Some(c), vec![], now).unwrap();
    assert_eq!(bounded.valid_until_ms, now + 500);
    assert!(bounded.can_reserve(now));
    assert!(
        !bounded.can_reserve(now + 500),
        "SOL replacement expiry also ends the reservation window"
    );
}
