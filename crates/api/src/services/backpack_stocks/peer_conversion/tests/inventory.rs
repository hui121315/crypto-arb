use super::*;
use crate::services::backpack_stocks::{peer_inventory::MintReader, peer_recovery};

fn mint() -> MintReader {
    Arc::new(|p| {
        Box::pin(async move {
            let mut m = p.terms.basis.chain_cost.mint.clone();
            m.checked_at_ms = common::time::now_ms();
            m.chain_time_ms = m.checked_at_ms;
            m.slot = p.peer_minimum_slot()?;
            Ok(m)
        })
    })
}
fn build(p: &StockPeerPlan) -> StockPeerInventoryRequest {
    StockPeerInventoryRequest {
        plan_id: p.plan_id.clone(),
        revision: p.revision,
        quote_limit: if p.peer_inventory_gap().unwrap().is_sign_negative() {
            "100"
        } else {
            "0.01"
        }
        .into(),
    }
}
fn action(p: &StockPeerPlan) -> StockRecoveryActionRequest {
    StockRecoveryActionRequest {
        plan_id: p.plan_id.clone(),
        revision: p.revision,
        index: p.inventory_orders.len() - 1,
    }
}
fn submit(p: &StockPeerPlan) -> StockPeerRecoverySubmitRequest {
    StockPeerRecoverySubmitRequest {
        plan_id: p.plan_id.clone(),
        revision: p.revision,
        index: p.inventory_orders.len() - 1,
        confirm_live: true,
    }
}
fn capture(s: &BackpackStocks, state: &str) {
    if let Ok(path) = std::env::var(format!("STOCK_PEER_INVENTORY_{state}_CAPTURE_PATH")) {
        std::fs::write(path, serde_json::to_vec_pretty(&s.snapshot()).unwrap()).unwrap();
    }
}
async fn wait_fill(s: &BackpackStocks, id: &str) -> StockPeerPlan {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let p = s.peer_plan_store.get(id).unwrap();
            if p.inventory_orders
                .last()
                .unwrap()
                .order
                .as_ref()
                .is_some_and(|o| !o.fills.is_empty())
            {
                return p;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn stock_peer_inventory_two_directions_restart_actual_costs_and_chain_restoration() {
    for direction in [StockChainDirection::Buy, StockChainDirection::Sell] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("peer.jsonl");
        let (s, p, peer) = fixture_with_mode(&path, direction, true).await;
        let hub = realtime::WsHub::default();
        let r = build(&p);
        s.prepare_peer_inventory_with(r.clone(), &hub, mint())
            .await
            .unwrap();
        let reads = peer.reads.load(Ordering::SeqCst);
        s.prepare_peer_inventory_with(
            r,
            &hub,
            Arc::new(|_| Box::pin(async { panic!("idempotent build must not re-read") })),
        )
        .await
        .unwrap();
        assert_eq!(reads, peer.reads.load(Ordering::SeqCst));
        let ready = s.peer_plan_store.get(&p.plan_id).unwrap();
        assert_eq!(
            stock_exact_decimal(&ready.inventory_orders[0].draft.quantity).unwrap(),
            p.peer_inventory_gap().unwrap().abs()
        );
        assert!(!ready.peer_recovery_available(common::time::now_ms()));
        assert!(!ready.peer_conversion_available(common::time::now_ms()));
        capture(&s, "READY");
        let mut no = submit(&ready);
        no.confirm_live = false;
        assert!(s
            .submit_peer_inventory_with(no, hub.clone(), Arc::new(|| Ok(())), mint())
            .await
            .is_err());
        assert!(s
            .submit_peer_inventory_with(
                submit(&ready),
                hub.clone(),
                Arc::new(|| Err("stopped".into())),
                mint()
            )
            .await
            .is_err());
        assert_eq!(peer.sends.load(Ordering::SeqCst), 0);
        s.submit_peer_inventory_with(submit(&ready), hub.clone(), Arc::new(|| Ok(())), mint())
            .await
            .unwrap();
        let pending = wait_fill(&s, &p.plan_id).await;
        assert_eq!(
            pending.accounting().status,
            StockAccountingStatus::AwaitingReceipts
        );
        assert!(pending.peer_recovery_target().is_err());
        assert!(s.cancel_peer_inventory(action(&pending), &hub).is_err());
        capture(&s, "PENDING");
        let agg = s.peer_feed.as_ref().unwrap().0.clone();
        let sources = s.peer_sources.clone().unwrap();
        drop(s);
        let s = Arc::new(
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
            s.peer_plan_store.problem().is_none(),
            "{:?}",
            s.peer_plan_store.problem()
        );
        s.submit_peer_inventory_with(submit(&ready), hub.clone(), Arc::new(|| Ok(())), mint())
            .await
            .unwrap();
        assert_eq!(peer.sends.load(Ordering::SeqCst), 1);
        s.recheck_peer_inventory(action(&pending), &hub)
            .await
            .unwrap();
        let cex = s.peer_plan_store.get(&p.plan_id).unwrap();
        let report = cex.accounting();
        assert_eq!(report.cex_stock_shares.as_deref(), Some("0"), "{report:?}");
        assert!(report.recovery_target.is_some(), "{report:?}");
        assert!(cex.peer_recovery_available(common::time::now_ms()));
        assert!(cex.peer_inventory_gap().is_err());
        let cash = cex.inventory_orders[0].actual_changes().unwrap();
        assert_eq!(
            stock_exact_decimal(&report.cash_totals["USD"]).unwrap(),
            stock_exact_decimal(&p.accounting().cash_totals["USD"]).unwrap()
                + stock_exact_decimal(&cash.quote_change).unwrap()
        );
        let done = peer_recovery::tests::restore_inventory_chain(&s, &cex, &path).await;
        let actual = done.accounting();
        assert_eq!(
            actual.status,
            StockAccountingStatus::LegsReconciled,
            "{actual:?}"
        );
        assert_eq!(actual.net_stock_shares.as_deref(), Some("0"));
        assert_eq!(actual.chain_stock_shares.as_deref(), Some("0"));
        assert_eq!(actual.cex_stock_shares.as_deref(), Some("0"));
        assert!(actual.recovery_target.is_none());
        assert!(
            done.holds_funds(i64::MAX),
            "cash/fees still require final settlement"
        );
        capture(&s, "COMPLETED");
        crate::services::backpack_stocks::peer_settlement::tests::verify_completed_inventory(&done, &path);
        let row = done.inventory_orders[0].order.clone().unwrap();
        s.peer_plan_store
            .inventory_receipt(&p.plan_id, 0, &row)
            .unwrap();
        assert_eq!(s.peer_plan_store.get(&p.plan_id).unwrap(), done);
        let mut over = done.clone();
        over.inventory_orders[0].order.as_mut().unwrap().fills[0]
            .fees
            .as_mut()
            .unwrap()[0]
            .quantity = "1".into();
        assert!(over.inventory_orders[0].observed_changes().is_ok());
        assert_eq!(over.accounting().status, StockAccountingStatus::NeedsReview);
        assert!(over.peer_conversion_gap().is_err());
        let mut conflict = row;
        conflict.mark_conflict("late inventory conflict");
        s.peer_plan_store
            .inventory_receipt(&p.plan_id, 0, &conflict)
            .unwrap();
        drop(s);
        let recovered = BackpackStocks::new().unwrap().with_peer_plan_store(path);
        assert!(
            recovered.peer_plan_store.problem().is_none(),
            "{:?}",
            recovered.peer_plan_store.problem()
        );
        assert_eq!(
            recovered.snapshot().peer_plans[0].accounting().status,
            StockAccountingStatus::NeedsReview
        );
    }
}

#[tokio::test]
async fn stock_peer_inventory_limits_stale_evidence_cancel_and_owned_send() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("peer.jsonl");
    let (s, p, peer) = fixture_with_mode(&path, StockChainDirection::Buy, true).await;
    let hub = realtime::WsHub::default();
    peer.stale.store(true, Ordering::SeqCst);
    assert!(s
        .prepare_peer_inventory_with(build(&p), &hub, mint())
        .await
        .is_err());
    peer.stale.store(false, Ordering::SeqCst);
    s.prepare_peer_inventory_with(build(&p), &hub, mint())
        .await
        .unwrap();
    let ready = s.peer_plan_store.get(&p.plan_id).unwrap();
    let c = &ready.inventory_orders[0];
    for case in 0..10 {
        let mut r = c.request.clone();
        let mut m = c.market.clone();
        let mut q = c.quote.clone();
        let mut a = c.account.clone();
        let mut token = c.mint.clone();
        match case {
            0 => r.quote_limit = "0.01".into(),
            1 => a.quote_available = Some("0".into()),
            2 => q.ask_quantity = Some("0".into()),
            3 => m.qty_step = Some(10.0),
            4 => m.asset_class = shared_types::InstrumentAssetClass::Crypto,
            5 => token.ui_multiplier = "2".into(),
            6 => token.slot = 0,
            7 => a.stock_taker_pct = None,
            8 => q.source_at_ms = Some(c.draft.prepared_at_ms - 10_000),
            _ => a.stock_asset = "another-stock".into(),
        }
        assert!(
            StockPeerInventory::compile(&p, r, m, q, a, token, c.draft.prepared_at_ms).is_err(),
            "case {case}"
        );
    }
    s.cancel_peer_inventory(action(&ready), &hub).unwrap();
    let cancelled = s.peer_plan_store.get(&p.plan_id).unwrap();
    assert!(s
        .submit_peer_inventory_with(submit(&cancelled), hub.clone(), Arc::new(|| Ok(())), mint())
        .await
        .is_err());
    s.prepare_peer_inventory_with(build(&cancelled), &hub, mint())
        .await
        .unwrap();
    let ready = s.peer_plan_store.get(&p.plan_id).unwrap();
    peer.pause.store(true, Ordering::SeqCst);
    let service = s.clone();
    let request = submit(&ready);
    let call = tokio::spawn(async move {
        service
            .submit_peer_inventory_with(
                request,
                realtime::WsHub::default(),
                Arc::new(|| Ok(())),
                mint(),
            )
            .await
    });
    peer.entered.notified().await;
    call.abort();
    let _ = call.await;
    peer.release.notify_one();
    let sent = wait_fill(&s, &p.plan_id).await;
    s.submit_peer_inventory_with(submit(&sent), hub.clone(), Arc::new(|| Ok(())), mint())
        .await
        .unwrap();
    assert_eq!(peer.sends.load(Ordering::SeqCst), 1);
    assert!(sent.holds_funds(i64::MAX));
}
