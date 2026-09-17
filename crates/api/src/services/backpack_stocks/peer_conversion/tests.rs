use super::*;
use exchange::{ExchangeError, ExchangeResult};
use rust_decimal::Decimal;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use tokio::sync::broadcast;

struct Peer {
    inventory: bool,
    path: std::path::PathBuf,
    account: Mutex<StockPeerAccount>,
    rows: Mutex<std::collections::BTreeMap<String, StockPeerOrderReceipt>>,
    events: broadcast::Sender<StockPeerOrderReceipt>,
    sends: AtomicUsize,
    reads: AtomicUsize,
    reject: AtomicBool,
    history: AtomicBool,
    stale: AtomicBool,
    pause: AtomicBool,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
impl Peer {
    fn emit(&self, original: &StockPeerOrderReceipt, known: bool) -> StockPeerOrderReceipt {
        let mut row = original.clone();
        let d = &row.draft;
        let at = d.prepared_at_ms + 1;
        let qty = stock_exact_decimal(&d.quantity).unwrap();
        let price = stock_exact_decimal(&d.limit_price).unwrap();
        let cost = qty * price;
        let fee_pct = if self.inventory {
            self.account.lock().stock_taker_pct.clone().unwrap()
        } else {
            "0.2".into()
        };
        row.apply(StockPeerExecutionPatch {
            order_id: format!("O-{}", row.client_order_id),
            client_order_id: Some(row.client_order_id.clone()),
            native_symbol: Some(d.request.selection.native_symbol.clone()),
            side: Some(
                if d.request.direction == StockChainDirection::Buy {
                    "sell"
                } else {
                    "buy"
                }
                .into(),
            ),
            order_quantity: Some(d.quantity.clone()),
            phase: Some(StockCexOrderPhase::Filled),
            cumulative_quantity: Some(d.quantity.clone()),
            cumulative_cost: Some(cost.normalize().to_string()),
            occurred_at_ms: at,
            fill: Some(StockPeerFill {
                execution_id: format!("E-{}", row.client_order_id),
                trade_id: Some(99),
                quantity: d.quantity.clone(),
                price: d.limit_price.clone(),
                cost: Some(cost.normalize().to_string()),
                fees: known.then(|| {
                    vec![StockTradeFee {
                        asset: d.quote_asset.clone(),
                        quantity: (cost * stock_exact_decimal(&fee_pct).unwrap()
                            / Decimal::from(100))
                        .normalize()
                        .to_string(),
                    }]
                }),
                occurred_at_ms: at,
            }),
        })
        .unwrap();
        row
    }
}
#[async_trait::async_trait]
impl ExchangeAdapter for Peer {
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
    async fn stock_cash_account(&self, _: &str) -> ExchangeResult<StockPeerAccount> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        let mut a = self.account.lock().clone();
        a.observed_at_ms = common::time::now_ms();
        Ok(a)
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
    fn track_stock_order(&self, o: StockPeerOrderReceipt) -> ExchangeResult<()> {
        self.rows
            .lock()
            .entry(o.client_order_id.clone())
            .or_insert(o);
        Ok(())
    }
    fn stock_order_receipt(&self, id: &str) -> Option<StockPeerOrderReceipt> {
        self.rows.lock().get(id).cloned()
    }
    async fn public_ws_spot_snapshot(
        &self,
        symbols: &[String],
    ) -> ExchangeResult<exchange::PublicWsSnapshot<shared_types::SpotTick>> {
        let now = common::time::now_ms();
        let at = if self.stale.load(Ordering::SeqCst) {
            now - 10_000
        } else {
            now
        };
        let symbol = if self.inventory {
            self.account.lock().native_symbol.clone()
        } else {
            "USDC/USD".into()
        };
        assert_eq!(symbols, &[symbol.clone()]);
        Ok(exchange::PublicWsSnapshot::Ready(vec![
            shared_types::SpotTick {
                venue: "kraken".into(),
                symbol,
                bid: if self.inventory { "599" } else { "0.9998" }
                    .parse()
                    .unwrap(),
                ask: if self.inventory { "601" } else { "1.0002" }
                    .parse()
                    .unwrap(),
                last: if self.inventory { 600.into() } else { 1.into() },
                bid_size: Some(1000.into()),
                ask_size: Some(1000.into()),
                volume_24h: 0.into(),
                exchange_ts_ms: Some(at),
                received_at_ms: at,
            },
        ]))
    }
    async fn submit_stock_order(
        &self,
        d: StockPeerOrderDraft,
        client: String,
    ) -> ExchangeResult<StockPeerOrderReceipt> {
        let v: serde_json::Value = serde_json::from_str(
            std::fs::read_to_string(&self.path)
                .unwrap()
                .lines()
                .last()
                .unwrap(),
        )
        .unwrap();
        let p: StockPeerPlan = serde_json::from_value(v["plan"].clone()).unwrap();
        let saved = if self.inventory {
            p.inventory_orders.last().unwrap().order.as_ref().unwrap()
        } else {
            p.conversions.last().unwrap().order.as_ref().unwrap()
        };
        assert_eq!(saved.client_order_id, client);
        assert_eq!(saved.draft, d);
        let mut row = StockPeerOrderReceipt::pending(d, client).unwrap();
        let frame = row
            .kraken_submission("fixture", 1, common::time::now_ms())
            .unwrap();
        assert_eq!(
            frame["params"]["symbol"],
            if self.inventory {
                self.account.lock().native_symbol.clone()
            } else {
                "USDC/USD".into()
            }
        );
        assert_eq!(frame["params"]["validate"], false);
        assert_eq!(frame["params"]["margin"], false);
        assert_eq!(frame["params"]["fee_preference"], "quote");
        assert_eq!(frame["params"]["time_in_force"], "fok");
        assert!(
            self.rows.lock().get(&row.client_order_id).is_none(),
            "restore before send would suppress original submission"
        );
        self.sends.fetch_add(1, Ordering::SeqCst);
        if self.reject.load(Ordering::SeqCst) {
            row.record_submission_ack(
                StockPeerOrderAck {
                    accepted: false,
                    request_id: 1,
                    received_at_ms: common::time::now_ms(),
                    message: "local rejected".into(),
                },
                None,
            )
            .unwrap();
        } else {
            row = self.emit(&row, false);
        }
        self.rows
            .lock()
            .insert(row.client_order_id.clone(), row.clone());
        self.events.send(row).ok();
        self.entered.notify_one();
        if self.pause.load(Ordering::SeqCst) {
            self.release.notified().await;
        }
        Err(ExchangeError::Parse("local reply lost".into()))
    }
    async fn reconcile_stock_order(
        &self,
        o: &StockPeerOrderReceipt,
    ) -> ExchangeResult<Option<StockPeerOrderReceipt>> {
        let v: serde_json::Value = serde_json::from_str(
            std::fs::read_to_string(&self.path)
                .unwrap()
                .lines()
                .last()
                .unwrap(),
        )
        .unwrap();
        let p: StockPeerPlan = serde_json::from_value(v["plan"].clone()).unwrap();
        assert!(
            if self.inventory {
                p.inventory_orders.last().unwrap().history.attempts
            } else {
                p.conversions.last().unwrap().history.attempts
            } > 0
        );
        Ok(self
            .history
            .load(Ordering::SeqCst)
            .then(|| self.emit(o, true)))
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
        panic!("no REST ticker")
    }
    async fn get_orderbook(&self, _: &str, _: u32) -> ExchangeResult<shared_types::OrderBookInfo> {
        panic!("no REST depth")
    }
}
async fn fixture(
    path: &std::path::Path,
    direction: StockChainDirection,
) -> (Arc<BackpackStocks>, StockPeerPlan, Arc<Peer>) {
    fixture_with_mode(path, direction, false).await
}
async fn fixture_with_mode(
    path: &std::path::Path,
    direction: StockChainDirection,
    inventory: bool,
) -> (Arc<BackpackStocks>, StockPeerPlan, Arc<Peer>) {
    let (s, p) = peer_execution::tests::conversion_fixture(path, direction).await;
    let mut a = p.terms.basis.account.clone();
    a.usdc_available = Some("100".into());
    let peer = Arc::new(Peer {
        inventory,
        path: path.into(),
        account: Mutex::new(a),
        rows: Mutex::new(Default::default()),
        events: broadcast::channel(64).0,
        sends: AtomicUsize::new(0),
        reads: AtomicUsize::new(0),
        reject: AtomicBool::new(false),
        history: AtomicBool::new(true),
        stale: AtomicBool::new(false),
        pause: AtomicBool::new(false),
        entered: Default::default(),
        release: Default::default(),
    });
    s.peer_feed.as_ref().unwrap().0.register(peer.clone());
    let mut m = p.terms.basis.peer.instrument.clone().unwrap();
    if !inventory {
        m.native_symbol = "USDC/USD".into();
        m.canonical_symbol = "USDC".into();
        m.display_symbol = "USDC/USD".into();
        m.asset_class = shared_types::InstrumentAssetClass::Crypto;
        m.execution_supported = true;
        m.price_tick = Some(0.0001);
        m.qty_step = Some(0.00000001);
        m.min_qty = Some(5.0);
        m.min_notional = Some(0.5);
    }
    m.checked_at_ms = common::time::now_ms();
    s.peer_sources.as_ref().unwrap().0.upsert(m).unwrap();
    (s, p, peer)
}

mod inventory;
fn build(p: &StockPeerPlan) -> StockPeerConversionRequest {
    StockPeerConversionRequest {
        plan_id: p.plan_id.clone(),
        revision: p.revision,
        usdc_limit: if p.peer_conversion_gap().unwrap() > Decimal::ZERO {
            "1"
        } else {
            "100"
        }
        .into(),
    }
}
fn action(p: &StockPeerPlan) -> StockRecoveryActionRequest {
    StockRecoveryActionRequest {
        plan_id: p.plan_id.clone(),
        revision: p.revision,
        index: p.conversions.len() - 1,
    }
}
fn submit(p: &StockPeerPlan) -> StockPeerRecoverySubmitRequest {
    StockPeerRecoverySubmitRequest {
        plan_id: p.plan_id.clone(),
        revision: p.revision,
        index: p.conversions.len() - 1,
        confirm_live: true,
    }
}
fn capture(s: &BackpackStocks, state: &str) {
    if let Ok(path) = std::env::var(format!("STOCK_PEER_CONVERSION_{state}_CAPTURE_PATH")) {
        std::fs::write(path, serde_json::to_vec_pretty(&s.snapshot()).unwrap()).unwrap();
    }
}

#[tokio::test]
async fn stock_peer_conversion_round_trip_receipts_fees_and_restart_never_resubmit() {
    for direction in [StockChainDirection::Buy, StockChainDirection::Sell] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("peer.jsonl");
        let (s, p, peer) = fixture(&path, direction).await;
        let hub = realtime::WsHub::default();
        let request = build(&p);
        s.prepare_peer_conversion(request.clone(), &hub)
            .await
            .unwrap();
        let reads = peer.reads.load(Ordering::SeqCst);
        s.prepare_peer_conversion(request, &hub).await.unwrap();
        assert_eq!(reads, peer.reads.load(Ordering::SeqCst));
        let ready = s.peer_plan_store.get(&p.plan_id).unwrap();
        capture(&s, "READY");
        let mut no = submit(&ready);
        no.confirm_live = false;
        assert!(s
            .submit_peer_conversion_with(no, hub.clone(), Arc::new(|| Ok(())))
            .await
            .is_err());
        assert!(s
            .submit_peer_conversion_with(
                submit(&ready),
                hub.clone(),
                Arc::new(|| Err("local stop".into()))
            )
            .await
            .is_err());
        assert_eq!(peer.sends.load(Ordering::SeqCst), 0);
        s.submit_peer_conversion_with(submit(&ready), hub.clone(), Arc::new(|| Ok(())))
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let p = s.peer_plan_store.get(&p.plan_id).unwrap();
                if !p.conversions[0].order.as_ref().unwrap().fills.is_empty() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        let unknown = s.peer_plan_store.get(&p.plan_id).unwrap();
        assert_eq!(
            unknown.accounting().status,
            StockAccountingStatus::AwaitingReceipts
        );
        assert!(unknown.peer_conversion_gap().is_err());
        assert!(s.cancel_peer_conversion(action(&unknown), &hub).is_err());
        capture(&s, "PENDING");
        let agg = s.peer_feed.as_ref().unwrap().0.clone();
        let sources = s.peer_sources.clone().unwrap();
        drop(s);
        let restored = Arc::new(
            BackpackStocks::new()
                .unwrap()
                .with_peer_feed(
                    agg,
                    Arc::new(
                        crate::services::market_subscriptions::MarketSubscriptions::load(None),
                    ),
                )
                .with_peer_markets(sources.0, sources.1)
                .with_peer_plan_store(path.clone()),
        );
        assert!(
            restored.peer_plan_store.problem().is_none(),
            "{:?}",
            restored.peer_plan_store.problem()
        );
        restored
            .submit_peer_conversion_with(submit(&ready), hub.clone(), Arc::new(|| Ok(())))
            .await
            .unwrap();
        assert_eq!(peer.sends.load(Ordering::SeqCst), 1);
        restored
            .recheck_peer_conversion(action(&unknown), &hub)
            .await
            .unwrap();
        let final_p = restored.peer_plan_store.get(&p.plan_id).unwrap();
        let a = final_p.accounting();
        assert_eq!(a.status, StockAccountingStatus::LegsReconciled, "{a:?}");
        let remainder = stock_exact_decimal(&a.cash_totals["USD"]).unwrap();
        assert!(remainder >= Decimal::ZERO && remainder < "0.00000002".parse().unwrap());
        let flows = final_p.conversions[0].cash_changes().unwrap();
        let original = p.accounting();
        for (asset, delta) in flows {
            assert_eq!(
                stock_exact_decimal(&a.cash_totals[&asset]).unwrap(),
                stock_exact_decimal(&original.cash_totals[&asset]).unwrap()
                    + stock_exact_decimal(&delta).unwrap()
            );
        }
        assert_eq!(a.net_stock_shares, original.net_stock_shares);
        let mut overspent = final_p.clone();
        overspent.conversions[0].order.as_mut().unwrap().fills[0]
            .fees
            .as_mut()
            .unwrap()[0]
            .quantity = "1".into();
        assert!(overspent.conversions[0].cash_changes().is_err());
        let actual = overspent.conversions[0].native_cash_changes().unwrap();
        let report = overspent.accounting();
        assert_eq!(report.status, StockAccountingStatus::NeedsReview);
        assert!(overspent.peer_conversion_gap().is_err());
        for (asset, delta) in actual {
            assert_eq!(
                stock_exact_decimal(&report.cash_totals[&asset]).unwrap(),
                stock_exact_decimal(&original.cash_totals[&asset]).unwrap()
                    + stock_exact_decimal(&delta).unwrap()
            );
        }
        assert!(final_p.holds_funds(i64::MAX));
        capture(&restored, "COMPLETED");
        let row = final_p.conversions[0].order.clone().unwrap();
        restored
            .peer_plan_store
            .conversion_receipt(&p.plan_id, 0, &row)
            .unwrap();
        assert_eq!(restored.peer_plan_store.get(&p.plan_id).unwrap(), final_p);
        let mut conflict = row;
        conflict.mark_conflict("local late conflict");
        restored
            .peer_plan_store
            .conversion_receipt(&p.plan_id, 0, &conflict)
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
        assert!(again.snapshot().peer_plans[0]
            .peer_conversion_gap()
            .is_err());
    }
}

#[tokio::test]
async fn stock_peer_conversion_cancel_bad_evidence_and_caller_disconnect() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("peer.jsonl");
    let (s, p, peer) = fixture(&path, StockChainDirection::Sell).await;
    let hub = realtime::WsHub::default();
    peer.stale.store(true, Ordering::SeqCst);
    // The original plan already warmed this FX pair. Only reject once that
    // shared WS quote also expires; an older arrival must not evict fresh data.
    tokio::time::sleep(Duration::from_millis(3_100)).await;
    assert!(s.prepare_peer_conversion(build(&p), &hub).await.is_err());
    peer.stale.store(false, Ordering::SeqCst);
    s.prepare_peer_conversion(build(&p), &hub).await.unwrap();
    let ready = s.peer_plan_store.get(&p.plan_id).unwrap();
    let row = ready.conversions[0].clone();
    for case in 0..10 {
        let mut c = row.clone();
        match case {
            0 => c.request.usdc_limit = "0.001".into(),
            1 => c.market.canonical_symbol = "BTC".into(),
            2 => c.market.price_tick = None,
            3 => c.quote.source = "rest".into(),
            4 => c.account.fx_taker_pct = None,
            5 => c.account.usdc_available = Some("0".into()),
            6 => c.quote.bid_quantity = Some("0".into()),
            7 => c.market.asset_class = shared_types::InstrumentAssetClass::Equity,
            8 => c.quote.symbol = "USDC/USDT".into(),
            _ => c.market.source_url = Some("unknown".into()),
        };
        assert!(
            StockPeerConversion::compile(
                &p,
                c.request,
                c.market,
                c.quote,
                c.account,
                row.draft.prepared_at_ms
            )
            .is_err(),
            "case {case}"
        );
    }
    s.cancel_peer_conversion(action(&ready), &hub).unwrap();
    let cancelled = s.peer_plan_store.get(&p.plan_id).unwrap();
    assert!(s
        .submit_peer_conversion_with(submit(&cancelled), hub.clone(), Arc::new(|| Ok(())))
        .await
        .is_err());
    s.prepare_peer_conversion(build(&cancelled), &hub)
        .await
        .unwrap();
    let ready = s.peer_plan_store.get(&p.plan_id).unwrap();
    peer.pause.store(true, Ordering::SeqCst);
    let caller = s.clone();
    let r = submit(&ready);
    let task = tokio::spawn(async move {
        caller
            .submit_peer_conversion_with(r, realtime::WsHub::default(), Arc::new(|| Ok(())))
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), peer.entered.notified())
        .await
        .unwrap();
    task.abort();
    let _ = task.await;
    peer.release.notify_one();
    let guard = s.submission_lock.lock().await;
    drop(guard);
    s.submit_peer_conversion_with(submit(&ready), hub.clone(), Arc::new(|| Ok(())))
        .await
        .unwrap();
    assert_eq!(peer.sends.load(Ordering::SeqCst), 1);
    let current = s.peer_plan_store.get(&p.plan_id).unwrap();
    s.recheck_peer_conversion(action(&current), &hub)
        .await
        .unwrap();
    assert!(s.peer_plan_store.get(&p.plan_id).unwrap().conversions[1]
        .cash_changes()
        .is_ok());
}
