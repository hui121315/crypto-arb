use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize};

fn quote(p: &StockPeerPlan, salt: u8) -> (StockWalletEvidence, StockChainCost) {
    let now = common::time::now_ms();
    let t = p.peer_recovery_target().unwrap();
    let mut c = p.terms.basis.chain_cost.clone();
    c.direction = t.direction;
    c.mint.slot = p.peer_minimum_slot().unwrap();
    c.mint.checked_at_ms = now;
    c.mint.chain_time_ms = now;
    c.checked_at_ms = now;
    c.valid_until_ms = now + 5000;
    c.quote.requested_at_ms = now;
    c.quote.received_at_ms = now;
    c.quote.expires_at_ms = None;
    if t.direction == StockChainDirection::Buy {
        c.quote.input_mint = shared_types::stocks::comparison::SOLANA_USDC.into();
        c.quote.output_mint = c.mint.address.clone();
        c.quote.input_raw = "11000000".into();
        c.quote.output_raw = t.stock_raw.clone();
        c.quote.minimum_output_raw = t.stock_raw;
    } else {
        c.quote.input_mint = c.mint.address.clone();
        c.quote.output_mint = shared_types::stocks::comparison::SOLANA_USDC.into();
        c.quote.input_raw = t.stock_raw;
        c.quote.output_raw = "11000000".into();
        c.quote.minimum_output_raw = "10000000".into();
    }
    chain::tests::attach_sponsored_variant(&mut c, salt);
    c.simulation_slot = Some(c.mint.slot);
    let w = StockWalletEvidence {
        owner: p.request.wallet_address.clone(),
        mint: c.mint.address.clone(),
        stock_raw: Some("1000000".into()),
        usdc_raw: Some("100000000".into()),
        sol_lamports: Some("1000000000".into()),
        checked_at_ms: now,
        problems: vec![],
    };
    (w, c)
}

struct Io {
    path: std::path::PathBuf,
    wallet: StockWalletEvidence,
    sends: AtomicUsize,
    signs: AtomicUsize,
    failed: AtomicBool,
    low_balance: AtomicBool,
    pause: bool,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
impl Io {
    fn new(path: std::path::PathBuf, wallet: StockWalletEvidence, pause: bool) -> Arc<Self> {
        Arc::new(Self {
            path,
            wallet,
            sends: AtomicUsize::new(0),
            signs: AtomicUsize::new(0),
            failed: AtomicBool::new(false),
            low_balance: AtomicBool::new(false),
            pause,
            entered: tokio::sync::Notify::new(),
            release: tokio::sync::Notify::new(),
        })
    }
}
#[async_trait::async_trait]
impl ChainTransport for Io {
    async fn check(&self, _: &StockChainCost) -> Result<(), String> {
        Ok(())
    }
    async fn wallet(&self, _: &StockChainCost) -> Result<StockWalletEvidence, String> {
        let mut w = self.wallet.clone();
        w.checked_at_ms = common::time::now_ms();
        if self.low_balance.load(Ordering::SeqCst) {
            w.stock_raw = Some("0".into());
            w.usdc_raw = Some("0".into());
            w.sol_lamports = Some("0".into());
        }
        Ok(w)
    }
    fn sign(&self, c: &StockChainCost) -> Result<String, String> {
        self.signs.fetch_add(1, Ordering::SeqCst);
        chain::tests::signed(c)
    }
    async fn send(&self, c: &StockChainCost, signed: &str) -> Result<Option<String>, String> {
        let bytes = std::fs::read_to_string(&self.path).unwrap();
        let v: serde_json::Value = serde_json::from_str(bytes.lines().last().unwrap()).unwrap();
        let p: StockPeerPlan = serde_json::from_value(v["plan"].clone()).unwrap();
        let row = p.recoveries.last().unwrap();
        assert_eq!(&row.cost, c);
        assert!(
            row.submission.is_some(),
            "intent must be on disk before send"
        );
        assert_eq!(signed, chain::tests::signed(c).unwrap());
        self.sends.fetch_add(1, Ordering::SeqCst);
        self.entered.notify_one();
        if self.pause {
            self.release.notified().await;
        }
        Err("local reply lost after submission".into())
    }
    async fn lookup(
        &self,
        c: &StockChainCost,
        s: &StockChainSubmission,
    ) -> Result<chain::Lookup, String> {
        chain::tests::parsed_finalized(c, s, self.failed.load(Ordering::SeqCst))
    }
}

fn submit(p: &StockPeerPlan) -> StockPeerRecoverySubmitRequest {
    StockPeerRecoverySubmitRequest {
        plan_id: p.plan_id.clone(),
        revision: p.revision,
        index: p.recoveries.len() - 1,
        confirm_live: true,
    }
}
fn action(p: &StockPeerPlan) -> StockRecoveryActionRequest {
    StockRecoveryActionRequest {
        plan_id: p.plan_id.clone(),
        revision: p.revision,
        index: p.recoveries.len() - 1,
    }
}
fn capture(s: &BackpackStocks, name: &str) {
    if let Ok(file) = std::env::var(name) {
        std::fs::write(file, serde_json::to_vec_pretty(&s.snapshot()).unwrap()).unwrap();
    }
}
async fn prepare(s: &Arc<BackpackStocks>, p: &StockPeerPlan, salt: u8) -> StockPeerPlan {
    let limit = if p.peer_recovery_target().unwrap().direction == StockChainDirection::Buy {
        "12"
    } else {
        "9"
    };
    let r = StockPeerRecoveryRequest {
        plan_id: p.plan_id.clone(),
        revision: p.revision,
        usdc_limit: limit.into(),
    };
    s.prepare_peer_recovery_with(
        r.clone(),
        &realtime::WsHub::default(),
        move |p, _| async move { Ok(quote(&p, salt)) },
    )
    .await
    .unwrap();
    s.prepare_peer_recovery_with(r, &realtime::WsHub::default(), |_, _| async {
        panic!("same original build must not requote")
    })
    .await
    .unwrap();
    s.peer_plan_store.get(&p.plan_id).unwrap()
}

pub(in crate::services::backpack_stocks) async fn restore_inventory_chain(
    s: &Arc<BackpackStocks>,
    p: &StockPeerPlan,
    path: &std::path::Path,
) -> StockPeerPlan {
    let ready = prepare(s, p, 77).await;
    let io = Io::new(
        path.into(),
        ready.recoveries.last().unwrap().wallet.clone(),
        false,
    );
    let hub = realtime::WsHub::default();
    s.submit_peer_recovery_with(submit(&ready), hub.clone(), io.clone(), Arc::new(|| Ok(())))
        .await
        .unwrap();
    let sent = s.peer_plan_store.get(&p.plan_id).unwrap();
    s.recheck_peer_recovery_with(action(&sent), &hub, io.as_ref())
        .await
        .unwrap();
    assert_eq!(io.sends.load(Ordering::SeqCst), 1);
    s.peer_plan_store.get(&p.plan_id).unwrap()
}

#[tokio::test]
async fn stock_peer_recovery_both_directions_lost_reply_restart_failure_retry_and_accounting() {
    for (direction, reject) in [
        (StockChainDirection::Buy, true),
        (StockChainDirection::Sell, true),
        (StockChainDirection::Buy, false),
        (StockChainDirection::Sell, false),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("peer.jsonl");
        let (s, original) = peer_execution::tests::recovery_fixture(&path, direction, reject).await;
        let p = prepare(&s, &original, 31).await;
        let io = Io::new(path.clone(), p.recoveries[0].wallet.clone(), false);
        let hub = realtime::WsHub::default();
        capture(&s, "STOCK_PEER_RECOVERY_READY_CAPTURE_PATH");
        let mut unconfirmed = submit(&p);
        unconfirmed.confirm_live = false;
        assert!(s
            .submit_peer_recovery_with(unconfirmed, hub.clone(), io.clone(), Arc::new(|| Ok(())))
            .await
            .is_err());
        assert!(s
            .submit_peer_recovery_with(
                submit(&p),
                hub.clone(),
                io.clone(),
                Arc::new(|| Err("local live guard".into()))
            )
            .await
            .is_err());
        io.low_balance.store(true, Ordering::SeqCst);
        assert!(s
            .submit_peer_recovery_with(submit(&p), hub.clone(), io.clone(), Arc::new(|| Ok(())))
            .await
            .is_err());
        assert_eq!(io.signs.load(Ordering::SeqCst), 0);
        io.low_balance.store(false, Ordering::SeqCst);
        s.submit_peer_recovery_with(submit(&p), hub.clone(), io.clone(), Arc::new(|| Ok(())))
            .await
            .unwrap();
        let pending = s.peer_plan_store.get(&p.plan_id).unwrap();
        assert!(pending.accounting().net_stock_shares.is_none());
        assert!(pending.peer_recovery_target().is_err());
        assert!(s.cancel_peer_recovery(action(&pending), &hub).is_err());
        assert!(!pending.peer_recovery_available(common::time::now_ms()));
        capture(&s, "STOCK_PEER_RECOVERY_PENDING_CAPTURE_PATH");
        let aggregator = s.peer_feed.as_ref().unwrap().0.clone();
        drop(s);
        let restored = Arc::new(
            BackpackStocks::new()
                .unwrap()
                .with_peer_feed(
                    aggregator,
                    Arc::new(
                        crate::services::market_subscriptions::MarketSubscriptions::load(None),
                    ),
                )
                .with_peer_plan_store(path.clone()),
        );
        assert!(
            restored.peer_plan_store.problem().is_none(),
            "{:?}",
            restored.peer_plan_store.problem()
        );
        restored
            .submit_peer_recovery_with(submit(&p), hub.clone(), io.clone(), Arc::new(|| Ok(())))
            .await
            .unwrap();
        assert_eq!(io.signs.load(Ordering::SeqCst), 1);
        assert_eq!(io.sends.load(Ordering::SeqCst), 1);
        assert!(restored
            .wallet_claims
            .check("solana", &p.request.wallet_address, i64::MAX)
            .is_err());
        io.failed.store(true, Ordering::SeqCst);
        restored
            .recheck_peer_recovery_with(action(&pending), &hub, io.as_ref())
            .await
            .unwrap();
        let failed = restored.peer_plan_store.get(&p.plan_id).unwrap();
        assert_eq!(
            failed.accounting().net_stock_shares,
            original.accounting().net_stock_shares
        );
        assert!(failed.peer_recovery_target().is_ok());
        let next = prepare(&restored, &failed, 32).await;
        io.failed.store(false, Ordering::SeqCst);
        restored
            .submit_peer_recovery_with(submit(&next), hub.clone(), io.clone(), Arc::new(|| Ok(())))
            .await
            .unwrap();
        restored
            .recheck_peer_recovery_with(action(&next), &hub, io.as_ref())
            .await
            .unwrap();
        let final_p = restored.peer_plan_store.get(&p.plan_id).unwrap();
        let report = final_p.accounting();
        assert_eq!(report.net_stock_shares.as_deref(), Some("0"), "{report:?}");
        assert_eq!(
            report.status,
            StockAccountingStatus::LegsReconciled,
            "{report:?}"
        );
        assert!(final_p.peer_recovery_target().is_err());
        assert_eq!(
            report.cash_totals["USD"],
            original.accounting().cash_totals["USD"]
        );
        let delta = if next.recoveries.last().unwrap().target.direction == StockChainDirection::Buy
        {
            Decimal::from(-11)
        } else {
            Decimal::from(11)
        };
        let initial = stock_exact_decimal(&original.accounting().cash_totals["USDC"]).unwrap();
        assert_eq!(
            report.cash_totals["USDC"],
            (initial + delta).normalize().to_string()
        );
        assert_eq!(report.network_fee_sol.as_deref(), Some("0.000021"));
        assert_eq!(report.wallet_sol_change.as_deref(), Some("0"));
        assert!(final_p.holds_funds(i64::MAX));
        assert_eq!(io.sends.load(Ordering::SeqCst), 2);
        capture(&restored, "STOCK_PEER_RECOVERY_COMPLETED_CAPTURE_PATH");
        restored
            .recheck_peer_recovery_with(action(&final_p), &hub, io.as_ref())
            .await
            .unwrap();
        assert_eq!(restored.peer_plan_store.get(&p.plan_id).unwrap(), final_p);
        let mut conflict = final_p.cex_order.clone().unwrap();
        conflict.mark_conflict("late original conflict");
        restored
            .peer_plan_store
            .receipt(&p.plan_id, &conflict)
            .unwrap();
        assert_eq!(
            restored
                .peer_plan_store
                .get(&p.plan_id)
                .unwrap()
                .accounting()
                .status,
            StockAccountingStatus::NeedsReview
        );
        drop(restored);
        let again = BackpackStocks::new().unwrap().with_peer_plan_store(path);
        assert!(
            again.peer_plan_store.problem().is_none(),
            "{:?}",
            again.peer_plan_store.problem()
        );
        assert!(again.snapshot().peer_accounting[0]
            .recovery_target
            .is_none());
    }
}

#[tokio::test]
async fn stock_peer_recovery_cancellation_bad_quotes_and_lost_http_caller_do_not_duplicate() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("peer.jsonl");
    let (s, p) =
        peer_execution::tests::recovery_fixture(&path, StockChainDirection::Sell, false).await;
    let (w, c) = quote(&p, 40);
    let row = StockPeerRecovery {
        source_revision: p.revision,
        prepared_at_ms: common::time::now_ms(),
        usdc_limit: "9".into(),
        target: p.peer_recovery_target().unwrap(),
        cost: c,
        wallet: w,
        cancelled_at_ms: None,
        submission: None,
    };
    for case in 0..10 {
        let mut bad = row.clone();
        match case {
            0 => bad.usdc_limit = "11".into(),
            1 => bad.cost.mint.ui_multiplier = "2".into(),
            2 => bad.cost.mint.slot = 1,
            3 => bad.cost.quote.input_raw = "1".into(),
            4 => bad.cost.quote.output_mint = "wrong".into(),
            5 => bad.wallet.stock_raw = Some("0".into()),
            6 => bad.wallet.sol_lamports = None,
            7 => bad.cost.valid_until_ms = bad.prepared_at_ms,
            8 => bad.cost.transaction = p.terms.basis.chain_cost.transaction.clone(),
            9 => bad.source_revision = 0,
            _ => unreachable!(),
        }
        assert!(
            s.peer_plan_store.prepare_recovery(&p.plan_id, bad).is_err(),
            "case {case}"
        );
    }
    let ready = prepare(&s, &p, 41).await;
    let hub = realtime::WsHub::default();
    let cancelled = s
        .cancel_peer_recovery(action(&ready), &hub)
        .unwrap()
        .peer_plans[0]
        .clone();
    let io = Io::new(path.clone(), ready.recoveries[0].wallet.clone(), true);
    assert!(s
        .submit_peer_recovery_with(
            submit(&cancelled),
            hub.clone(),
            io.clone(),
            Arc::new(|| Ok(()))
        )
        .await
        .is_err());
    assert_eq!(io.signs.load(Ordering::SeqCst), 0);
    let ready = prepare(&s, &cancelled, 42).await;
    let caller = s.clone();
    let r = submit(&ready);
    let transport = io.clone();
    let task = tokio::spawn(async move {
        caller
            .submit_peer_recovery_with(
                r,
                realtime::WsHub::default(),
                transport,
                Arc::new(|| Ok(())),
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), io.entered.notified())
        .await
        .unwrap();
    task.abort();
    let _ = task.await;
    io.release.notify_one();
    let owner = tokio::time::timeout(
        Duration::from_secs(2),
        s.submission_lock.clone().lock_owned(),
    )
    .await
    .unwrap();
    drop(owner);
    s.submit_peer_recovery_with(submit(&ready), hub.clone(), io.clone(), Arc::new(|| Ok(())))
        .await
        .unwrap();
    assert_eq!(io.signs.load(Ordering::SeqCst), 1);
    assert_eq!(io.sends.load(Ordering::SeqCst), 1);
    let submitted = s.peer_plan_store.get(&p.plan_id).unwrap();
    assert!(s.cancel_peer_recovery(action(&submitted), &hub).is_err());
    s.recheck_peer_recovery_with(action(&submitted), &hub, io.as_ref())
        .await
        .unwrap();
    assert_eq!(
        s.peer_plan_store
            .get(&p.plan_id)
            .unwrap()
            .accounting()
            .net_stock_shares
            .as_deref(),
        Some("0")
    );
}
