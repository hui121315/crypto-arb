use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize};

struct Io {
    path: std::path::PathBuf,
    wallet: StockWalletEvidence,
    sends: AtomicUsize,
    signs: AtomicUsize,
    failed: AtomicBool,
    missing: AtomicBool,
    low: AtomicBool,
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
            missing: AtomicBool::new(false),
            low: AtomicBool::new(false),
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
        if self.low.load(Ordering::SeqCst) {
            w.usdc_raw = Some("0".into());
        }
        Ok(w)
    }
    fn sign(&self, c: &StockChainCost) -> Result<String, String> {
        self.signs.fetch_add(1, Ordering::SeqCst);
        chain::tests::signed(c)
    }
    async fn send(&self, c: &StockChainCost, signed: &str) -> Result<Option<String>, String> {
        let disk = std::fs::read_to_string(&self.path).unwrap();
        let v: serde_json::Value = serde_json::from_str(disk.lines().last().unwrap()).unwrap();
        let p: StockPeerPlan = serde_json::from_value(v["plan"].clone()).unwrap();
        let r = p.native_topups.last().unwrap();
        assert!(r.terms.submission.is_some(), "intent persisted before send");
        assert_eq!(r.cost(&p).unwrap(), *c);
        assert_eq!(signed, chain::tests::signed(c).unwrap());
        self.sends.fetch_add(1, Ordering::SeqCst);
        self.entered.notify_one();
        if self.pause {
            self.release.notified().await;
        }
        Err("local reply lost".into())
    }
    async fn lookup(
        &self,
        c: &StockChainCost,
        s: &StockChainSubmission,
    ) -> Result<chain::Lookup, String> {
        if self.missing.load(Ordering::SeqCst) {
            return Ok(chain::Lookup {
                receipt: None,
                before: None,
            });
        }
        chain::tests::parsed_finalized_native(c, s, self.failed.load(Ordering::SeqCst))
    }
}
fn row(p: &StockPeerPlan, salt: u8) -> StockPeerNativeTopup {
    let now = common::time::now_ms();
    let (target, slot) = p.peer_native_target().unwrap();
    let mut v = super::super::native_topup::tests::valuation(
        &p.terms.basis.chain_cost,
        target,
        50_000,
        now,
        salt,
    );
    v.replenishment.as_mut().unwrap().simulation_slot = slot;
    let mut w = p.terms.basis.wallet.clone();
    w.checked_at_ms = now;
    StockPeerNativeTopup {
        terms: StockNativeTopup {
            source_revision: p.revision,
            prepared_at_ms: now,
            valuation: v,
            wallet: w,
            submission: None,
        },
        usdc_limit: "0.06".into(),
        cancelled_at_ms: None,
    }
}
async fn prepare(s: &BackpackStocks, p: &StockPeerPlan, salt: u8) -> StockPeerPlan {
    let r = StockPeerNativeTopupRequest {
        plan_id: p.plan_id.clone(),
        revision: p.revision,
        usdc_limit: "0.06".into(),
    };
    s.prepare_peer_native_topup_with(
        r.clone(),
        &realtime::WsHub::default(),
        move |p, _, _| async move {
            let r = row(&p, salt);
            Ok((r.terms.wallet, r.terms.valuation))
        },
    )
    .await
    .unwrap();
    s.prepare_peer_native_topup_with(r, &realtime::WsHub::default(), |_, _, _| async {
        panic!("idempotent prepare requoted")
    })
    .await
    .unwrap();
    s.peer_plan_store.get(&p.plan_id).unwrap()
}
fn action(p: &StockPeerPlan) -> StockRecoveryActionRequest {
    StockRecoveryActionRequest {
        plan_id: p.plan_id.clone(),
        revision: p.revision,
        index: p.native_topups.len() - 1,
    }
}
fn submit(p: &StockPeerPlan) -> StockPeerRecoverySubmitRequest {
    StockPeerRecoverySubmitRequest {
        plan_id: p.plan_id.clone(),
        revision: p.revision,
        index: p.native_topups.len() - 1,
        confirm_live: true,
    }
}
fn capture(s: &BackpackStocks, state: &str) {
    if let Ok(path) = std::env::var(format!("STOCK_PEER_NATIVE_{state}_CAPTURE_PATH")) {
        std::fs::write(path, serde_json::to_vec_pretty(&s.snapshot()).unwrap()).unwrap();
    }
}

#[tokio::test]
async fn stock_peer_native_topup_lost_reply_restart_failed_fee_and_success_are_exact() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("peer.jsonl");
    let (s, p) = peer_execution::tests::native_fixture(&path).await;
    let before = p.accounting();
    let ready = prepare(&s, &p, 63).await;
    capture(&s, "READY");
    let io = Io::new(
        path.clone(),
        ready.native_topups[0].terms.wallet.clone(),
        false,
    );
    let hub = realtime::WsHub::default();
    let mut no = submit(&ready);
    no.confirm_live = false;
    assert!(s
        .submit_peer_native_topup_with(no, hub.clone(), io.clone(), Arc::new(|| Ok(())))
        .await
        .is_err());
    io.low.store(true, Ordering::SeqCst);
    assert!(s
        .submit_peer_native_topup_with(submit(&ready), hub.clone(), io.clone(), Arc::new(|| Ok(())))
        .await
        .is_err());
    assert_eq!(io.signs.load(Ordering::SeqCst), 0);
    io.low.store(false, Ordering::SeqCst);
    s.submit_peer_native_topup_with(submit(&ready), hub.clone(), io.clone(), Arc::new(|| Ok(())))
        .await
        .unwrap();
    capture(&s, "PENDING");
    let pending = s.peer_plan_store.get(&p.plan_id).unwrap();
    assert!(pending.peer_native_target().is_err());
    assert!(!pending.peer_conversion_available(common::time::now_ms()));
    assert!(s.cancel_peer_native_topup(action(&pending), &hub).is_err());
    let agg = s.peer_feed.as_ref().unwrap().0.clone();
    drop(s);
    let restored = Arc::new(
        BackpackStocks::new()
            .unwrap()
            .with_peer_feed(
                agg,
                Arc::new(crate::services::market_subscriptions::MarketSubscriptions::load(None)),
            )
            .with_peer_plan_store(path.clone()),
    );
    assert!(
        restored.peer_plan_store.problem().is_none(),
        "{:?}",
        restored.peer_plan_store.problem()
    );
    restored
        .submit_peer_native_topup_with(submit(&ready), hub.clone(), io.clone(), Arc::new(|| Ok(())))
        .await
        .unwrap();
    assert_eq!(io.sends.load(Ordering::SeqCst), 1);
    assert_eq!(io.signs.load(Ordering::SeqCst), 1);
    assert!(restored
        .wallet_claims
        .check("solana", &p.request.wallet_address, i64::MAX)
        .is_err());
    io.failed.store(true, Ordering::SeqCst);
    restored
        .recheck_peer_native_topup_with(action(&pending), &hub, io.as_ref())
        .await
        .unwrap();
    let failed = restored.peer_plan_store.get(&p.plan_id).unwrap();
    assert_eq!(
        failed.peer_native_target().unwrap().0,
        14000,
        "failed retry adds its fee to actual debit"
    );
    assert_eq!(failed.accounting().cash_totals, before.cash_totals);
    let next = prepare(&restored, &failed, 64).await;
    io.failed.store(false, Ordering::SeqCst);
    restored
        .submit_peer_native_topup_with(submit(&next), hub.clone(), io.clone(), Arc::new(|| Ok(())))
        .await
        .unwrap();
    restored
        .recheck_peer_native_topup_with(action(&next), &hub, io.as_ref())
        .await
        .unwrap();
    let done = restored.peer_plan_store.get(&p.plan_id).unwrap();
    let a = done.accounting();
    assert_eq!(a.status, StockAccountingStatus::LegsReconciled, "{a:?}");
    assert_eq!(a.net_stock_shares, before.net_stock_shares);
    assert_eq!(a.wallet_sol_change.as_deref(), Some("0"));
    assert_eq!(a.network_fee_sol.as_deref(), Some("0.000021"));
    assert_eq!(a.cash_totals["USD"], before.cash_totals["USD"]);
    assert_eq!(
        stock_exact_decimal(&a.cash_totals["USDC"]).unwrap(),
        stock_exact_decimal(&before.cash_totals["USDC"]).unwrap()
            - rust_decimal::Decimal::new(5, 2)
    );
    assert!(done.peer_native_target().is_err());
    assert!(done.holds_funds(i64::MAX));
    let mut overspent = done.clone();
    let receipt = overspent
        .native_topups
        .last_mut()
        .unwrap()
        .terms
        .submission
        .as_mut()
        .unwrap()
        .receipt
        .as_mut()
        .unwrap();
    receipt
        .asset_changes
        .iter_mut()
        .find(|a| a.mint == shared_types::stocks::comparison::SOLANA_USDC)
        .unwrap()
        .raw_change = "-60000".into();
    receipt.within_plan = false;
    receipt.problems = vec!["actual amount differs".into()];
    let review = overspent.accounting();
    assert_eq!(review.status, StockAccountingStatus::NeedsReview);
    assert_eq!(
        stock_exact_decimal(&review.cash_totals["USDC"]).unwrap(),
        stock_exact_decimal(&before.cash_totals["USDC"]).unwrap()
            - rust_decimal::Decimal::new(6, 2)
    );
    assert!(
        overspent.peer_native_target().is_err(),
        "real overspend is preserved, not executable"
    );
    let mut missing = done.clone();
    missing
        .native_topups
        .last_mut()
        .unwrap()
        .terms
        .submission
        .as_mut()
        .unwrap()
        .receipt
        .as_mut()
        .unwrap()
        .asset_changes
        .retain(|a| a.mint != shared_types::stocks::comparison::SOLANA_USDC);
    assert_eq!(
        missing.accounting().status,
        StockAccountingStatus::NeedsReview
    );
    assert!(
        missing.accounting().wallet_sol_change.is_none(),
        "missing actual cash is not zero"
    );
    assert!(
        a.remaining.iter().any(|s| s.contains("不同发行方")),
        "SOL completion is not inventory restoration"
    );
    capture(&restored, "COMPLETED");
    restored
        .recheck_peer_native_topup_with(action(&done), &hub, io.as_ref())
        .await
        .unwrap();
    assert_eq!(restored.peer_plan_store.get(&p.plan_id).unwrap(), done);
    let mut conflict = done.cex_order.clone().unwrap();
    conflict.mark_conflict("late original conflict");
    restored
        .peer_plan_store
        .receipt(&p.plan_id, &conflict)
        .unwrap();
    drop(restored);
    let again = BackpackStocks::new().unwrap().with_peer_plan_store(path);
    assert!(
        again.peer_plan_store.problem().is_none(),
        "{:?}",
        again.peer_plan_store.problem()
    );
    assert_eq!(
        again.snapshot().peer_accounting[0].status,
        StockAccountingStatus::NeedsReview
    );
    assert_eq!(io.sends.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn stock_peer_native_topup_rejects_bad_budget_identity_and_survives_caller_cancellation() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("peer.jsonl");
    let (s, p) = peer_execution::tests::native_fixture(&path).await;
    let good = row(&p, 65);
    let mut later = good.clone();
    later
        .terms
        .valuation
        .replenishment
        .as_mut()
        .unwrap()
        .simulation_slot = 99;
    assert_eq!(
        later.cost(&p).unwrap().mint.slot,
        99,
        "pre-send wallet reads require the latest simulation slot"
    );
    assert!(good
        .validate(
            &p,
            good.terms
                .valuation
                .replenishment
                .as_ref()
                .unwrap()
                .valid_until_ms
        )
        .is_err());
    for case in 0..10 {
        let mut bad = good.clone();
        match case {
            0 => bad.usdc_limit = "0.01".into(),
            1 => bad.terms.valuation.native_lamports = "1".into(),
            2 => {
                bad.terms
                    .valuation
                    .replenishment
                    .as_mut()
                    .unwrap()
                    .minimum_credit_lamports = "1".into()
            }
            3 => bad.terms.wallet.usdc_raw = Some("1".into()),
            4 => bad.terms.wallet.sol_lamports = None,
            5 => {
                bad.terms
                    .valuation
                    .replenishment
                    .as_mut()
                    .unwrap()
                    .wallet_address = "wrong".into()
            }
            6 => {
                bad.terms
                    .valuation
                    .replenishment
                    .as_mut()
                    .unwrap()
                    .simulation_slot = 1
            }
            7 => {
                bad.terms
                    .valuation
                    .replenishment
                    .as_mut()
                    .unwrap()
                    .valid_until_ms = bad.terms.prepared_at_ms
            }
            8 => bad.terms.source_revision = 0,
            9 => {
                bad.terms
                    .valuation
                    .replenishment
                    .as_mut()
                    .unwrap()
                    .transaction_fingerprint = "wrong".into()
            }
            _ => unreachable!(),
        }
        assert!(
            s.peer_plan_store
                .prepare_native_topup(&p.plan_id, bad)
                .is_err(),
            "bad case {case}"
        );
    }
    let ready = prepare(&s, &p, 66).await;
    let hub = realtime::WsHub::default();
    let cancelled = s
        .cancel_peer_native_topup(action(&ready), &hub)
        .unwrap()
        .peer_plans[0]
        .clone();
    let io = Io::new(
        path.clone(),
        ready.native_topups[0].terms.wallet.clone(),
        true,
    );
    assert!(s
        .submit_peer_native_topup_with(
            submit(&cancelled),
            hub.clone(),
            io.clone(),
            Arc::new(|| Ok(()))
        )
        .await
        .is_err());
    let next = prepare(&s, &cancelled, 67).await;
    let caller = s.clone();
    let transport = io.clone();
    let r = submit(&next);
    let task = tokio::spawn(async move {
        caller
            .submit_peer_native_topup_with(
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
    s.submit_peer_native_topup_with(submit(&next), hub.clone(), io.clone(), Arc::new(|| Ok(())))
        .await
        .unwrap();
    assert_eq!(io.sends.load(Ordering::SeqCst), 1);
    io.missing.store(true, Ordering::SeqCst);
    s.recheck_peer_native_topup_with(action(&next), &hub, io.as_ref())
        .await
        .unwrap();
    assert!(s
        .recheck_peer_native_topup_with(action(&next), &hub, io.as_ref())
        .await
        .is_err());
    let pending = s.peer_plan_store.get(&p.plan_id).unwrap();
    assert!(pending.peer_native_target().is_err());
    drop(s);
    let reopened = BackpackStocks::new().unwrap().with_peer_plan_store(path);
    assert!(reopened.peer_plan_store.problem().is_none());
    assert!(reopened
        .recheck_peer_native_topup_with(action(&pending), &hub, io.as_ref())
        .await
        .is_err());
}
