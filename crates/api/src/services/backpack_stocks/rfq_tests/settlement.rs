use super::*;

#[tokio::test]
async fn backpack_stock_rfq_settlement_restart_keeps_budget_and_manual_recheck_recovers_without_posts(
) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rfq.jsonl");
    let m = Mock::new();
    let (root, _server) = server(m.clone()).await;
    let s = configured(&root, path.clone());
    let now = common::time::now_ms();
    let req = request(StockRfqSide::Bid);
    let (r, _) = s
        .rfq_store
        .claim(
            req.clone(),
            &keys().unwrap().fingerprint(),
            "MU.US_USDC_RFQ".into(),
            now,
        )
        .unwrap();
    let mut ack = native(&rfq_protocol::submit_body(&r));
    ack["status"] = "Filled".into();
    m.rows.lock().insert(r.client_id, ack.clone());
    s.rfq_store
        .change(&req.request_id, true, |r| {
            rfq_protocol::apply_rest(
                r,
                rfq_protocol::acknowledgement(&serde_json::to_vec(&ack).unwrap()).unwrap(),
                now,
            )?;
            r.settlement.attempts = 5;
            r.settlement.next_at_ms = Some(now + 60_000);
            Ok(true)
        })
        .unwrap();
    drop(s);
    let hub = realtime::WsHub::new(16);
    let s = configured(&root, path.clone());
    assert!(s.rfq_store.records()[0].needs_follow_up());
    s.resume_rfq(hub.clone());
    until(|| m.subscriptions.load(Ordering::SeqCst) > 0).await;
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(m.fill_reads.load(Ordering::SeqCst), 0);
    s.rfq_store
        .change(&req.request_id, true, |r| {
            r.settlement.next_at_ms = Some(common::time::now_ms() - 1);
            Ok(true)
        })
        .unwrap();
    until(|| {
        s.rfq_store
            .get(&req.request_id)
            .unwrap()
            .executed_quantity
            .is_some()
    })
    .await;
    let r = s.rfq_store.get(&req.request_id).unwrap();
    assert_eq!(r.settlement.attempts, 6);
    assert!(r.settlement.paused && r.settlement_pending() && !r.needs_follow_up());
    let weak = Arc::downgrade(&s);
    drop(s);
    until(|| weak.upgrade().is_none()).await;
    let s = configured(&root, path);
    s.resume_rfq(hub.clone());
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert_eq!(m.fill_reads.load(Ordering::SeqCst), 1);
    assert_eq!(m.connections.load(Ordering::SeqCst), 1);
    m.complete_fills.store(true, Ordering::SeqCst);
    s.recheck_rfq(&req.request_id, hub).await.unwrap();
    let r = s.rfq_store.get(&req.request_id).unwrap();
    assert!(!r.unresolved() && !r.settlement.paused);
    assert_eq!(r.executed_quantity.as_deref(), Some("1"));
    assert_eq!(m.fill_reads.load(Ordering::SeqCst), 2);
    assert_eq!(m.posts.load(Ordering::SeqCst), 0);
    assert_eq!(m.cancels.load(Ordering::SeqCst), 0);
}

#[test]
fn backpack_stock_rfq_fill_history_is_exact_replay_safe_and_preserves_good_rows_on_errors() {
    let dir = tempfile::tempdir().unwrap();
    let store = rfq_store::RfqStore::load(Some(dir.path().join("rfq.jsonl")));
    let now = common::time::now_ms();
    let (mut r, _) = store
        .claim(
            request(StockRfqSide::Bid),
            "fixture",
            "MU.US_USDC_RFQ".into(),
            now,
        )
        .unwrap();
    let mut ack = native(&rfq_protocol::submit_body(&r));
    ack["status"] = "Filled".into();
    rfq_protocol::apply_rest(
        &mut r,
        rfq_protocol::acknowledgement(&serde_json::to_vec(&ack).unwrap()).unwrap(),
        now,
    )
    .unwrap();
    let fill = fill_row(&ack, "9007199254740997");
    let page = serde_json::to_vec(&vec![fill.clone(), fill.clone()]).unwrap();
    rfq_history::fills(&mut r, &page, now).unwrap();
    assert_eq!(r.fills.len(), 1);
    assert!(r.settlement_pending());
    assert!(!rfq_history::fills(&mut r, &page, now + 1).unwrap());
    let before = r.clone();
    for case in [
        "missing",
        "negative",
        "wrong_side",
        "wrong_client",
        "wrong_market",
        "quote_conflict",
        "regression",
        "truncated",
        "empty",
    ] {
        let mut bad = fill.clone();
        let page = match case {
            "missing" => {
                bad.as_object_mut().unwrap().remove("fillQuantity");
                json!([bad])
            }
            "negative" => {
                bad["fillQuoteQuantity"] = "-1".into();
                json!([bad])
            }
            "wrong_side" => {
                bad["side"] = "Ask".into();
                json!([bad])
            }
            "wrong_client" => {
                bad["clientId"] = 0.into();
                json!([bad])
            }
            "wrong_market" => {
                bad["symbol"] = "BTC_USDC_RFQ".into();
                json!([bad])
            }
            "quote_conflict" => {
                bad["fillPrice"] = "999".into();
                json!([fill.clone(), bad])
            }
            "regression" => {
                bad["fillQuantity"] = "0.25".into();
                json!([bad])
            }
            "truncated" => json!(vec![fill.clone(); 100]),
            _ => json!([]),
        };
        assert!(
            rfq_history::fills(&mut r, &serde_json::to_vec(&page).unwrap(), now + 1).is_err(),
            "accepted {case}"
        );
        assert_eq!(r, before, "mutated good data on {case}");
    }
    let more = fill_row(&ack, "9007199254740998");
    rfq_history::fills(
        &mut r,
        &serde_json::to_vec(&json!([fill, more])).unwrap(),
        now + 1,
    )
    .unwrap();
    assert!(!r.settlement_pending());
    assert_eq!(r.executed_quote_quantity.as_deref(), Some("101.05"));
    let mut legacy = serde_json::to_value(&r).unwrap();
    legacy.as_object_mut().unwrap().remove("fills");
    legacy.as_object_mut().unwrap().remove("settlement");
    let old: StockRfq = serde_json::from_value(legacy).unwrap();
    assert!(
        old.needs_follow_up(),
        "legacy totals without native fill rows need rechecking"
    );
}
