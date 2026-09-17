use super::*;

#[tokio::test]
async fn stock_rfq_acceptance_manual_recheck_reads_completed_trade_and_keeps_conflicts_across_restart(
) {
    for conflict in [
        "history_cancel",
        "history_expiry",
        "changed_fill",
        "ws_cancel",
        "ws_price",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let m = Mock::new();
        let (root, _server) = server(m.clone()).await;
        let (s, p, hub) = prepared(&root, dir.path(), &m).await;
        s.send_rfq_leg(&p.plan_id, hub.clone()).await.unwrap();
        stop(&s).await;
        m.filled();
        let id = &p.terms.rfq.as_ref().unwrap().request_id;
        s.reconcile_rfq(id, &keys().unwrap()).await.unwrap();
        let filled = receipt(&s, &p);
        assert!(!filled.settlement_pending());
        assert!(
            filled.fill_price.is_none(),
            "REST fillPrice is not a documented WS taker price"
        );
        let apply = |value: Value| {
            rfq_runtime::apply_frame(
                &s,
                &value.to_string(),
                &keys().unwrap().fingerprint(),
                common::time::now_ms(),
            )
        };
        let mut frame: Value = serde_json::from_str(&m.frame("rfqFilled")).unwrap();
        assert!(apply(frame.clone()).unwrap());
        assert_eq!(receipt(&s, &p).fill_price.as_deref(), Some("601"));
        assert!(!apply(frame.clone()).unwrap());

        let before = receipt(&s, &p);
        let reads = m.fill_reads.load(Ordering::SeqCst);
        let history = m.history_reads.load(Ordering::SeqCst);
        let bytes = std::fs::metadata(dir.path().join("plans.jsonl"))
            .unwrap()
            .len();
        s.reconcile_rfq(id, &keys().unwrap()).await.unwrap();
        assert_eq!(m.fill_reads.load(Ordering::SeqCst), reads + 1);
        assert_eq!(m.history_reads.load(Ordering::SeqCst), history + 1);
        assert_eq!(receipt(&s, &p), before);
        assert_eq!(
            std::fs::metadata(dir.path().join("plans.jsonl"))
                .unwrap()
                .len(),
            bytes
        );

        if conflict.starts_with("history_") {
            m.native.lock()["status"] = json!(if conflict == "history_cancel" {
                "Cancelled"
            } else {
                "Expired"
            });
            assert!(s.reconcile_rfq(id, &keys().unwrap()).await.is_err());
        } else if conflict == "changed_fill" {
            let mut fill = m.fill();
            fill["fillQuoteQuantity"] = json!("12.03");
            *m.fill_override.lock() = Some(fill);
            assert!(s.reconcile_rfq(id, &keys().unwrap()).await.is_err());
        } else {
            frame["data"]["T"] = json!(common::time::now_ms() * 1000);
            if conflict == "ws_cancel" {
                frame["data"]["e"] = json!("rfqCancelled");
                frame["data"]["X"] = json!("Cancelled");
            } else {
                // Still within the accepted sell limit, but contradicts the final WS price.
                frame["data"]["p"] = json!("602");
            }
            assert!(apply(frame).is_err());
        }
        let disputed = receipt(&s, &p);
        assert!(
            disputed.acceptance.as_ref().unwrap().evidence_conflict,
            "{conflict}"
        );
        assert!(disputed.settlement.paused && !disputed.needs_follow_up());
        assert_eq!(disputed.fills, before.fills);
        assert_eq!(
            disputed.executed_quote_quantity,
            before.executed_quote_quantity
        );
        assert_eq!(disputed.fill_price, before.fill_price);
        s.rfq_problem_record(id, "temporary network failure".into())
            .unwrap();
        assert_eq!(receipt(&s, &p).problem, disputed.problem);
        m.filled();
        *m.fill_override.lock() = None;
        s.reconcile_rfq(id, &keys().unwrap()).await.unwrap();
        assert_eq!(
            receipt(&s, &p),
            disputed,
            "a later good response cannot erase a conflict"
        );
        if conflict == "history_cancel" {
            if let Ok(path) = std::env::var("STOCK_BP_RFQ_CONFLICT_CAPTURE_PATH") {
                std::fs::write(
                    path,
                    serde_json::to_vec(&s.plan_store.get(&p.plan_id).unwrap()).unwrap(),
                )
                .unwrap();
            }
        }
        drop(s);
        let restored = configured(&root, dir.path(), false);
        assert!(restored.plan_store.problem().is_none());
        assert_eq!(receipt(&restored, &p), disputed);
        let reads = m.fill_reads.load(Ordering::SeqCst);
        restored
            .reconcile_rfq_with_mode(id, &keys().unwrap(), true)
            .await
            .unwrap();
        assert_eq!(m.fill_reads.load(Ordering::SeqCst), reads);
        assert!(restored
            .plan_store
            .get(&p.plan_id)
            .unwrap()
            .holds_funds(common::time::now_ms() + 120_000));
        restored.send_rfq_leg(&p.plan_id, hub).await.unwrap();
        assert_eq!(m.posts.load(Ordering::SeqCst), 1);
        assert_eq!(m.cancels.load(Ordering::SeqCst), 0);
        stop(&restored).await;
    }
}

#[tokio::test]
async fn stock_rfq_acceptance_late_fill_retains_evidence_without_unpausing_disputed_receipt() {
    let dir = tempfile::tempdir().unwrap();
    let m = Mock::new();
    let (root, _server) = server(m.clone()).await;
    let (s, p, hub) = prepared(&root, dir.path(), &m).await;
    s.send_rfq_leg(&p.plan_id, hub).await.unwrap();
    stop(&s).await;
    let mut frame: Value = serde_json::from_str(&m.frame("rfqFilled")).unwrap();
    frame["data"]["u"] = json!("42");
    assert!(rfq_runtime::apply_frame(
        &s,
        &frame.to_string(),
        &keys().unwrap().fingerprint(),
        common::time::now_ms()
    )
    .is_err());
    let problem = receipt(&s, &p).problem;
    m.filled();
    let id = &p.terms.rfq.as_ref().unwrap().request_id;
    s.reconcile_rfq(id, &keys().unwrap()).await.unwrap();
    let r = receipt(&s, &p);
    assert_eq!(r.phase, StockRfqPhase::Filled);
    assert_eq!(r.executed_quantity.as_deref(), Some("0.02"));
    assert!(!r.settlement_pending());
    assert_eq!(r.fills.len(), 1);
    assert!(r.acceptance.as_ref().unwrap().evidence_conflict && r.settlement.paused);
    assert_eq!(r.problem, problem);
    assert!(r.settlement.next_at_ms.is_none());
    drop(s);
    let restored = configured(&root, dir.path(), false);
    assert!(restored.plan_store.problem().is_none());
    assert_eq!(receipt(&restored, &p), r);
    assert_eq!(m.posts.load(Ordering::SeqCst), 1);
}

#[test]
fn stock_rfq_terminal_receipts_distinguish_delayed_updates_from_incompatible_results() {
    let now = common::time::now_ms();
    let (snapshot, _, _) = plans::tests::rfq_fixture(now);
    let mut r = snapshot.rfqs[0].clone();
    r.phase = StockRfqPhase::Cancelled;
    let before = r.clone();
    let mut response = native(&r);
    response["status"] = json!("Filled");
    let rest = rfq_protocol::acknowledgement(&serde_json::to_vec(&response).unwrap()).unwrap();
    assert!(rfq_protocol::apply_rest(&mut r, rest, now).is_err());
    assert_eq!(r, before);
    for old in [
        StockRfqPhase::Cancelled,
        StockRfqPhase::Expired,
        StockRfqPhase::Filled,
    ] {
        r.phase = old;
        for next in [
            StockRfqPhase::Cancelled,
            StockRfqPhase::Expired,
            StockRfqPhase::Filled,
        ] {
            assert_eq!(
                rfq_protocol::terminal_receipt(&r, next).is_ok(),
                old == next
            );
        }
        assert!(!rfq_protocol::terminal_receipt(&r, StockRfqPhase::AwaitingQuotes).unwrap());
        assert!(!rfq_protocol::terminal_receipt(&r, StockRfqPhase::AcceptedBinding).unwrap());
    }
}
