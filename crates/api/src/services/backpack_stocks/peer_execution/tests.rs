use super::*;
use exchange::{ExchangeError, ExchangeResult};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicBool, AtomicUsize},
};
mod accounting;

struct Peer {
    path: PathBuf,
    row: Mutex<Option<StockPeerOrderReceipt>>,
    events: broadcast::Sender<StockPeerOrderReceipt>,
    sends: AtomicUsize,
    history_reads: AtomicUsize,
    history_available: AtomicBool,
    reject: bool,
    account: Mutex<StockPeerAccount>,
}
impl Peer {
    fn new(path: PathBuf, reject: bool, account: StockPeerAccount) -> Self {
        Self {
            path,
            row: Mutex::new(None),
            events: broadcast::channel(64).0,
            sends: AtomicUsize::new(0),
            history_reads: AtomicUsize::new(0),
            history_available: AtomicBool::new(false),
            reject,
            account: Mutex::new(account),
        }
    }
    fn emit_fill(&self, known_fee: bool) {
        let mut rows = self.row.lock();
        let r = rows.as_mut().unwrap();
        let cost = stock_exact_decimal(&r.draft.quantity).unwrap()
            * stock_exact_decimal(&r.draft.limit_price).unwrap();
        let fee = cost / rust_decimal::Decimal::from(1000);
        let at = r.draft.prepared_at_ms + 1;
        r.apply(StockPeerExecutionPatch {
            order_id: "local-stock-order".into(),
            client_order_id: Some(r.client_order_id.clone()),
            native_symbol: Some(r.draft.request.selection.native_symbol.clone()),
            side: None,
            order_quantity: Some(r.draft.quantity.clone()),
            phase: Some(StockCexOrderPhase::Filled),
            cumulative_quantity: Some(r.draft.quantity.clone()),
            cumulative_cost: Some(cost.normalize().to_string()),
            fill: Some(StockPeerFill {
                execution_id: "local-exec-1".into(),
                trade_id: Some(1),
                quantity: r.draft.quantity.clone(),
                price: r.draft.limit_price.clone(),
                cost: Some(cost.normalize().to_string()),
                fees: known_fee.then(|| {
                    vec![StockTradeFee {
                        asset: "USD".into(),
                        quantity: fee.normalize().to_string(),
                    }]
                }),
                occurred_at_ms: at,
            }),
            occurred_at_ms: at,
        })
        .unwrap();
        self.events.send(r.clone()).ok();
    }
}
fn disk_intent(path: &std::path::Path) -> StockPeerPlan {
    let text = std::fs::read_to_string(path).unwrap();
    let v: serde_json::Value = serde_json::from_str(text.lines().last().unwrap()).unwrap();
    let p: StockPeerPlan = serde_json::from_value(v["plan"].clone()).unwrap();
    assert_eq!(p.phase, StockPeerPlanPhase::SubmissionUnknown);
    assert!(p.cex_order.is_some() && p.chain_submission.is_some());
    p
}
#[async_trait::async_trait]
impl ExchangeAdapter for Peer {
    async fn stock_cash_account(&self, _: &str) -> ExchangeResult<StockPeerAccount> {
        let mut row = self.account.lock().clone();
        row.observed_at_ms = common::time::now_ms();
        Ok(row)
    }
    fn name(&self) -> &'static str {
        "kraken"
    }
    fn normalize_symbol(&self, s: &str) -> String {
        s.into()
    }
    fn to_exchange_symbol(&self, s: &str) -> String {
        s.into()
    }
    fn stock_account_fingerprint(&self) -> Option<String> {
        Some("1234567890abcdef12345678".into())
    }
    async fn prepare_stock_submission(&self) -> ExchangeResult<()> {
        Ok(())
    }
    async fn warm_stock_receipts(&self) -> ExchangeResult<()> {
        Ok(())
    }
    fn subscribe_stock_receipts(
        &self,
    ) -> ExchangeResult<broadcast::Receiver<StockPeerOrderReceipt>> {
        Ok(self.events.subscribe())
    }
    fn stock_order_receipt(&self, _: &str) -> Option<StockPeerOrderReceipt> {
        self.row.lock().clone()
    }
    async fn reconcile_stock_order(
        &self,
        original: &StockPeerOrderReceipt,
    ) -> ExchangeResult<Option<StockPeerOrderReceipt>> {
        let persisted = disk_intent(&self.path);
        assert!(persisted.cex_history.attempts > 0);
        assert!(persisted.cex_history.next_check_at_ms > common::time::now_ms());
        self.history_reads.fetch_add(1, Ordering::SeqCst);
        if !self.history_available.load(Ordering::SeqCst) {
            return Ok(None);
        }
        let mut r = original.clone();
        for f in &mut r.fills {
            f.fees = Some(vec![StockTradeFee {
                asset: r.draft.quote_asset.clone(),
                quantity: (stock_exact_decimal(f.cost.as_deref().unwrap()).unwrap()
                    / rust_decimal::Decimal::from(1000))
                .normalize()
                .to_string(),
            }]);
        }
        Ok(Some(r))
    }
    fn track_stock_order(&self, r: StockPeerOrderReceipt) -> ExchangeResult<()> {
        r.validate_stored().unwrap();
        self.row.lock().get_or_insert(r);
        Ok(())
    }
    async fn submit_stock_order(
        &self,
        d: StockPeerOrderDraft,
        client: String,
    ) -> ExchangeResult<StockPeerOrderReceipt> {
        let p = disk_intent(&self.path);
        assert_eq!(p.cex_order.as_ref().unwrap().client_order_id, client);
        assert!(
            self.row.lock().is_none(),
            "restore must not suppress the first submission"
        );
        self.sends.fetch_add(1, Ordering::SeqCst);
        let mut r = StockPeerOrderReceipt::pending(d, client).unwrap();
        let frame = r
            .kraken_submission("local-fixture-token", 7, common::time::now_ms())
            .unwrap();
        assert_eq!(frame["params"]["validate"], false);
        assert_eq!(frame["params"]["time_in_force"], "fok");
        if self.reject {
            r.record_submission_ack(
                StockPeerOrderAck {
                    accepted: false,
                    request_id: 7,
                    received_at_ms: common::time::now_ms(),
                    message: "local rejection".into(),
                },
                None,
            )
            .unwrap();
            *self.row.lock() = Some(r.clone());
            return Ok(r);
        }
        *self.row.lock() = Some(r);
        self.emit_fill(false);
        // Trade arrives on the shared WS before the HTTP caller receives an ACK.
        tokio::task::yield_now().await;
        Err(ExchangeError::Parse("reply lost after match".into()))
    }
    async fn get_funding_rate(&self, _: &str) -> ExchangeResult<shared_types::FundingRateData> {
        panic!("no funding")
    }
    async fn get_funding_rates(
        &self,
        _: Option<&[String]>,
    ) -> ExchangeResult<Vec<shared_types::FundingRateData>> {
        panic!("no funding")
    }
    async fn get_ticker(&self, _: &str) -> ExchangeResult<shared_types::TickerInfo> {
        panic!("no REST ticker")
    }
    async fn get_tickers(
        &self,
        _: Option<&[String]>,
    ) -> ExchangeResult<Vec<shared_types::TickerInfo>> {
        panic!("no REST tickers")
    }
    async fn get_orderbook(&self, _: &str, _: u32) -> ExchangeResult<shared_types::OrderBookInfo> {
        panic!("no depth query")
    }
}
struct Chain {
    path: PathBuf,
    sends: AtomicUsize,
    signs: AtomicUsize,
    failed: bool,
    missing: AtomicBool,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
    pause: bool,
}
#[async_trait::async_trait]
impl ChainTransport for Chain {
    async fn wallet(&self, _: &StockChainCost) -> Result<StockWalletEvidence, String> {
        panic!("original pair does not use recovery wallet reads")
    }
    async fn check(&self, _: &StockChainCost) -> Result<(), String> {
        Ok(())
    }
    fn sign(&self, c: &StockChainCost) -> Result<String, String> {
        self.signs.fetch_add(1, Ordering::SeqCst);
        chain::tests::signed(c)
    }
    async fn send(&self, c: &StockChainCost, signed: &str) -> Result<Option<String>, String> {
        let p = disk_intent(&self.path);
        assert_eq!(&p.terms.basis.chain_cost, c);
        assert_eq!(signed, chain::tests::signed(c).unwrap());
        self.entered.notify_one();
        if self.pause {
            self.release.notified().await;
        }
        self.sends.fetch_add(1, Ordering::SeqCst);
        Err("local reply lost after broadcast".into())
    }
    async fn lookup(
        &self,
        c: &StockChainCost,
        row: &StockChainSubmission,
    ) -> Result<chain::Lookup, String> {
        if self.missing.load(Ordering::SeqCst) {
            return Ok(chain::Lookup {
                receipt: None,
                before: None,
            });
        }
        chain::tests::parsed_finalized(c, row, self.failed)
    }
}
async fn fixture(
    path: &std::path::Path,
    direction: StockChainDirection,
    reject: bool,
    failed: bool,
    pause: bool,
) -> (Arc<BackpackStocks>, StockPeerPlan, Arc<Peer>, Arc<Chain>) {
    let (s, r, a, w, c) = peer_plan::tests::fixture(path, direction);
    s.build_peer_plan_with(
        r,
        &realtime::WsHub::default(),
        |_, _, _, _| async { Ok((a, w)) },
        |_, _| async { Ok(c) },
    )
    .await
    .unwrap();
    let p = s.snapshot().peer_plans[0].clone();
    let peer = Arc::new(Peer::new(
        path.into(),
        reject,
        p.terms.basis.account.clone(),
    ));
    s.peer_feed.as_ref().unwrap().0.register(peer.clone());
    let chain = Arc::new(Chain {
        path: path.into(),
        sends: AtomicUsize::new(0),
        signs: AtomicUsize::new(0),
        failed,
        missing: AtomicBool::new(false),
        entered: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
        pause,
    });
    (s, p, peer, chain)
}
pub(in crate::services::backpack_stocks) async fn recovery_fixture(
    path: &std::path::Path,
    direction: StockChainDirection,
    reject: bool,
) -> (Arc<BackpackStocks>, StockPeerPlan) {
    let (s, p, peer, chain) = fixture(path, direction, reject, !reject, false).await;
    let hub = realtime::WsHub::default();
    s.execute_peer_owned(request(&p), hub.clone(), chain.clone(), Arc::new(|| Ok(())))
        .await
        .unwrap();
    if !reject {
        wait_receipt(&s, &p.plan_id, false).await;
        peer.emit_fill(true);
        wait_receipt(&s, &p.plan_id, true).await;
    }
    s.recheck_peer_with(&p.plan_id, &hub, chain.as_ref())
        .await
        .unwrap();
    let p = s.peer_plan_store.get(&p.plan_id).unwrap();
    (s, p)
}
fn request(p: &StockPeerPlan) -> StockPeerExecutionRequest {
    StockPeerExecutionRequest {
        plan_id: p.plan_id.clone(),
        revision: p.revision,
        confirm_live: true,
    }
}

pub(in crate::services::backpack_stocks) async fn conversion_fixture(path:&std::path::Path,direction:StockChainDirection)->(Arc<BackpackStocks>,StockPeerPlan){
    let (s,p,peer,chain)=fixture(path,direction,false,false,false).await;
    let hub=realtime::WsHub::default();
    s.execute_peer_owned(request(&p),hub.clone(),chain.clone(),Arc::new(||Ok(()))).await.unwrap();
    wait_receipt(&s,&p.plan_id,false).await;peer.emit_fill(true);wait_receipt(&s,&p.plan_id,true).await;
    s.recheck_peer_with(&p.plan_id,&hub,chain.as_ref()).await.unwrap();
    let p=s.peer_plan_store.get(&p.plan_id).unwrap();(s,p)
}

pub(in crate::services::backpack_stocks) async fn native_fixture(path:&std::path::Path)->(Arc<BackpackStocks>,StockPeerPlan){
    let (s,r,a,w,mut c)=peer_plan::tests::fixture(path,StockChainDirection::Sell);
    chain::tests::attach_variant(&mut c,61);
    c.wallet_budget_lamports=Some("7000".into());
    c.wallet_required_lamports=Some("7000".into());
    c.native_valuation=Some(super::super::native_topup::tests::valuation(&c,7000,50_000,common::time::now_ms(),62));
    s.snapshot.write().comparison.as_mut().unwrap().sell=Some(c.quote.clone());
    let hub=realtime::WsHub::default();
    s.build_peer_plan_with(r,&hub,|_,_,_,_|async{Ok((a,w))},|_,_|async{Ok(c)}).await.unwrap();
    let p=s.snapshot().peer_plans[0].clone();
    let peer=Arc::new(Peer::new(path.into(),false,p.terms.basis.account.clone()));
    s.peer_feed.as_ref().unwrap().0.register(peer.clone());
    let io=Arc::new(Chain{path:path.into(),sends:AtomicUsize::new(0),signs:AtomicUsize::new(0),failed:false,missing:AtomicBool::new(false),entered:tokio::sync::Notify::new(),release:tokio::sync::Notify::new(),pause:false});
    s.execute_peer_owned(request(&p),hub.clone(),io.clone(),Arc::new(||Ok(()))).await.unwrap();
    wait_receipt(&s,&p.plan_id,false).await;peer.emit_fill(true);wait_receipt(&s,&p.plan_id,true).await;
    s.recheck_peer_with(&p.plan_id,&hub,io.as_ref()).await.unwrap();
    let p=s.peer_plan_store.get(&p.plan_id).unwrap();
    assert_eq!(p.peer_native_target().unwrap().0,7000);
    (s,p)
}

#[tokio::test]
async fn stock_peer_pair_receipt_merge_advances_partial_totals_without_losing_or_duplicating_fills()
{
    let temp = tempfile::tempdir().unwrap();
    let (_s, p, _, _) = fixture(
        &temp.path().join("peer.jsonl"),
        StockChainDirection::Buy,
        false,
        false,
        false,
    )
    .await;
    let mut first =
        StockPeerOrderReceipt::pending(p.terms.draft.clone(), "local-original".into()).unwrap();
    let half = stock_exact_decimal(&first.draft.quantity).unwrap() / rust_decimal::Decimal::from(2);
    let cost = half * stock_exact_decimal(&first.draft.limit_price).unwrap();
    let event = |index: u32| StockPeerExecutionPatch {
        order_id: "original-multi-fill".into(),
        client_order_id: Some("local-original".into()),
        native_symbol: None,
        side: None,
        order_quantity: None,
        phase: Some(if index == 1 {
            StockCexOrderPhase::Open
        } else {
            StockCexOrderPhase::Filled
        }),
        cumulative_quantity: Some(
            (half * rust_decimal::Decimal::from(index))
                .normalize()
                .to_string(),
        ),
        cumulative_cost: Some(
            (cost * rust_decimal::Decimal::from(index))
                .normalize()
                .to_string(),
        ),
        fill: Some(StockPeerFill {
            execution_id: format!("fill-{index}"),
            trade_id: Some(u64::try_from(index).unwrap()),
            quantity: half.normalize().to_string(),
            price: p.terms.draft.limit_price.clone(),
            cost: Some(cost.normalize().to_string()),
            fees: Some(vec![StockTradeFee {
                asset: "USD".into(),
                quantity: "0.001".into(),
            }]),
            occurred_at_ms: p.terms.created_at_ms + i64::from(index),
        }),
        occurred_at_ms: p.terms.created_at_ms + i64::from(index),
    };
    first.apply(event(1)).unwrap();
    let mut complete = first.clone();
    complete.apply(event(2)).unwrap();
    let mut durable = first.clone();
    durable.merge_snapshot(&complete).unwrap();
    assert_eq!(durable, complete);
    assert!(durable.receipt_complete());
    durable.merge_snapshot(&first).unwrap();
    durable.merge_snapshot(&complete).unwrap();
    assert_eq!(durable, complete);
    complete
        .record_submission_ack(
            StockPeerOrderAck {
                accepted: true,
                request_id: 7,
                received_at_ms: p.terms.created_at_ms + 3,
                message: "late original ack".into(),
            },
            complete.order_id.clone(),
        )
        .unwrap();
    durable.merge_snapshot(&complete).unwrap();
    assert_eq!(durable, complete);
    let mut bad = complete.clone();
    bad.fills.push(bad.fills[0].clone());
    assert!(durable.merge_snapshot(&bad).is_err());
    assert_eq!(durable, complete);
    complete.mark_conflict("original venue evidence conflict");
    durable.merge_snapshot(&complete).unwrap();
    assert_eq!(durable, complete);
    assert!(!durable.receipt_complete());
}
async fn wait_receipt(s: &BackpackStocks, id: &str, ready: bool) -> StockPeerPlan {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let p = s.peer_plan_store.get(id).unwrap();
            if p.cex_order
                .as_ref()
                .is_some_and(|r| !r.fills.is_empty() && r.receipt_complete() == ready)
            {
                break p;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn stock_peer_pair_lost_replies_native_fees_restart_and_one_leg_failure_never_resubmit() {
    for (direction, reject, failed) in [
        (StockChainDirection::Buy, false, false),
        (StockChainDirection::Sell, false, false),
        (StockChainDirection::Buy, true, false),
        (StockChainDirection::Sell, false, true),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("peer.jsonl");
        let (s, p, peer, chain) = fixture(&path, direction, reject, failed, false).await;
        let hub = realtime::WsHub::default();
        s.snapshot
            .write()
            .comparison
            .as_mut()
            .unwrap()
            .buy
            .input_raw = "99999999".into();
        s.execute_peer_owned(request(&p), hub.clone(), chain.clone(), Arc::new(|| Ok(())))
            .await
            .unwrap();
        assert_eq!(peer.sends.load(Ordering::SeqCst), 1);
        assert_eq!(chain.sends.load(Ordering::SeqCst), 1);
        if !reject {
            let before = wait_receipt(&s, &p.plan_id, false).await;
            assert!(
                before.cex_order.unwrap().cash_settlement().is_none(),
                "missing fee is not zero"
            );
            peer.emit_fill(true);
            let after = wait_receipt(&s, &p.plan_id, true).await;
            assert_eq!(
                after
                    .cex_order
                    .unwrap()
                    .cash_settlement()
                    .unwrap()
                    .quote_asset,
                "USD"
            );
        }
        s.recheck_peer_with(&p.plan_id, &hub, chain.as_ref())
            .await
            .unwrap();
        let final_p = s.peer_plan_store.get(&p.plan_id).unwrap();
        assert_eq!(
            final_p
                .chain_submission
                .as_ref()
                .unwrap()
                .receipt
                .as_ref()
                .unwrap()
                .succeeded,
            !failed
        );
        assert!(final_p.cex_order.as_ref().unwrap().receipt_complete());
        assert!(final_p.holds_funds(p.terms.reserved_until_ms + 100000));
        assert!(s
            .cancel_peer_plan(
                StockPlanRevisionRequest {
                    plan_id: p.plan_id.clone(),
                    revision: final_p.revision
                },
                &hub
            )
            .is_err());
        assert!(s
            .wallet_claims
            .check(
                "solana",
                &p.request.wallet_address,
                p.terms.reserved_until_ms + 100000
            )
            .is_err());
        if direction == StockChainDirection::Sell && !failed {
            if let Ok(path) = std::env::var("STOCK_PEER_EXECUTION_CAPTURE_PATH") {
                std::fs::write(path, serde_json::to_vec_pretty(&s.snapshot()).unwrap()).unwrap();
            }
        }
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
                .with_peer_plan_store(path),
        );
        assert!(
            restored.peer_plan_store.problem().is_none(),
            "{:?}",
            restored.peer_plan_store.problem()
        );
        assert_eq!(restored.peer_plan_store.get(&p.plan_id).unwrap(), final_p);
        assert!(restored
            .wallet_claims
            .check(
                "solana",
                &p.request.wallet_address,
                p.terms.reserved_until_ms + 100000
            )
            .is_err());
        restored
            .execute_peer_owned(request(&p), hub, chain.clone(), Arc::new(|| Ok(())))
            .await
            .unwrap();
        assert_eq!(peer.sends.load(Ordering::SeqCst), 1);
        assert_eq!(chain.signs.load(Ordering::SeqCst), 1);
        assert_eq!(chain.sends.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn stock_peer_pair_caller_cancellation_does_not_cancel_owner_or_send_twice() {
    let temp = tempfile::tempdir().unwrap();
    let (s, p, peer, chain) = fixture(
        &temp.path().join("peer.jsonl"),
        StockChainDirection::Buy,
        false,
        false,
        true,
    )
    .await;
    let caller = s.clone();
    let req = request(&p);
    let io = chain.clone();
    let task = tokio::spawn(async move {
        caller
            .execute_peer_owned(req, realtime::WsHub::default(), io, Arc::new(|| Ok(())))
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), chain.entered.notified())
        .await
        .unwrap();
    task.abort();
    let _ = task.await;
    chain.release.notify_one();
    let guard = tokio::time::timeout(
        Duration::from_secs(2),
        s.submission_lock.clone().lock_owned(),
    )
    .await
    .unwrap();
    drop(guard);
    assert_eq!(peer.sends.load(Ordering::SeqCst), 1);
    assert_eq!(chain.sends.load(Ordering::SeqCst), 1);
    s.execute_peer_owned(
        request(&p),
        realtime::WsHub::default(),
        chain.clone(),
        Arc::new(|| Ok(())),
    )
    .await
    .unwrap();
    assert_eq!(chain.signs.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn stock_peer_pair_confirmation_mode_price_and_journal_failure_prevent_both_sends() {
    for case in 0..10 {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("peer.jsonl");
        let (s, p, peer, chain) =
            fixture(&path, StockChainDirection::Buy, false, false, false).await;
        let mut r = request(&p);
        match case {
            0 => r.confirm_live = false,
            1 => r.revision = 999,
            2 => s.snapshot.write().comparison = None,
            3 => {
                std::fs::remove_file(&path).unwrap();
                std::fs::create_dir(&path).unwrap();
            }
            5 => peer.account.lock().stock_available = Some("0".into()),
            6 => peer.account.lock().stock_taker_pct = Some("1".into()),
            7 => peer.account.lock().quote_asset = "USDT".into(),
            8 => {
                s.snapshot
                    .write()
                    .peer
                    .as_mut()
                    .unwrap()
                    .quote
                    .as_mut()
                    .unwrap()
                    .bid = "1".into()
            }
            9 => {
                s.snapshot
                    .write()
                    .peer
                    .as_mut()
                    .unwrap()
                    .quote
                    .as_mut()
                    .unwrap()
                    .source_at_ms = Some(1)
            }
            _ => {}
        }
        let check = Arc::new(move || {
            if case == 4 {
                Err("local kill switch".into())
            } else {
                Ok(())
            }
        });
        assert!(
            s.execute_peer_owned(r, realtime::WsHub::default(), chain.clone(), check)
                .await
                .is_err(),
            "case {case}"
        );
        assert_eq!(peer.sends.load(Ordering::SeqCst), 0);
        assert_eq!(chain.sends.load(Ordering::SeqCst), 0);
        assert!(s.snapshot().peer_plans[0].cex_order.is_none());
    }
}

#[tokio::test]
async fn stock_peer_pair_history_recovers_fees_and_persists_cooldown_across_restart() {
    for available in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("peer.jsonl");
        let (s, p, peer, chain) =
            fixture(&path, StockChainDirection::Sell, false, false, false).await;
        peer.history_available.store(available, Ordering::SeqCst);
        let hub = realtime::WsHub::default();
        s.execute_peer_owned(request(&p), hub.clone(), chain.clone(), Arc::new(|| Ok(())))
            .await
            .unwrap();
        wait_receipt(&s, &p.plan_id, false).await;
        s.recheck_peer_with(&p.plan_id, &hub, chain.as_ref())
            .await
            .unwrap();
        let final_p = s.peer_plan_store.get(&p.plan_id).unwrap();
        assert_eq!(final_p.cex_history.attempts, 1);
        assert_eq!(
            final_p.cex_order.as_ref().unwrap().receipt_complete(),
            available
        );
        assert_eq!(peer.history_reads.load(Ordering::SeqCst), 1);
        assert!(final_p.chain_submission.as_ref().unwrap().receipt.is_some());
        assert!(final_p.holds_funds(p.terms.reserved_until_ms + 100000));
        let aggregator = s.peer_feed.as_ref().unwrap().0.clone();
        if let Ok(file) = std::env::var(if available {
            "STOCK_PEER_HISTORY_CAPTURE_PATH"
        } else {
            "STOCK_PEER_HISTORY_PENDING_CAPTURE_PATH"
        }) {
            std::fs::write(file, serde_json::to_vec_pretty(&s.snapshot()).unwrap()).unwrap();
        }
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
                .with_peer_plan_store(path),
        );
        assert!(
            restored.peer_plan_store.problem().is_none(),
            "{:?}",
            restored.peer_plan_store.problem()
        );
        assert_eq!(restored.peer_plan_store.get(&p.plan_id).unwrap(), final_p);
        restored
            .recheck_peer_with(&p.plan_id, &hub, chain.as_ref())
            .await
            .unwrap();
        assert_eq!(
            peer.history_reads.load(Ordering::SeqCst),
            1,
            "restart and repeated check must honor cooldown or completed receipt"
        );
        if !available {
            peer.emit_fill(true);
            let recovered = wait_receipt(&restored, &p.plan_id, true).await;
            assert!(
                recovered.cex_history.problem.is_none(),
                "late WS completion clears obsolete history warning"
            );
            assert_eq!(peer.history_reads.load(Ordering::SeqCst), 1);
        }
        assert_eq!(peer.sends.load(Ordering::SeqCst), 1);
        assert_eq!(chain.signs.load(Ordering::SeqCst), 1);
        assert_eq!(chain.sends.load(Ordering::SeqCst), 1);
        assert!(restored
            .wallet_claims
            .check(
                "solana",
                &p.request.wallet_address,
                p.terms.reserved_until_ms + 100000
            )
            .is_err());
    }
}
