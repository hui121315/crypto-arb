use super::*;

#[tokio::test]
async fn stock_order_manual_refresh_checks_completed_history_without_resend_or_conflict_erasure() {
    for conflict in ["changed_fee", "history_cancel"] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("orders.jsonl");
        let mock = Mock::new(path.clone());
        let (root, _server) = server(mock.clone()).await;
        let (service, plan) = prepared(&root, path.clone()).await;
        let hub = realtime::WsHub::new(16);
        service
            .send_orderbook_leg(&plan.plan_id, hub.clone())
            .await
            .unwrap();
        stop(&service).await;
        mock.settle(false);
        service
            .reconcile_stock_order(&plan.plan_id, &keys().unwrap(), false)
            .await
            .unwrap();
        let complete = service.plan_store.get(&plan.plan_id).unwrap();
        assert!(complete.cex_order.as_ref().unwrap().receipt_complete());

        let reads = mock.reads.load(Ordering::SeqCst);
        let journal = std::fs::read(&path).unwrap();
        service
            .reconcile_stock_order(&plan.plan_id, &keys().unwrap(), true)
            .await
            .unwrap();
        assert_eq!(
            mock.reads.load(Ordering::SeqCst),
            reads,
            "completed orders are not background-polled"
        );
        service
            .recheck_stock_order(&plan.plan_id, hub.clone())
            .await
            .unwrap();
        stop(&service).await;
        assert_eq!(
            mock.reads.load(Ordering::SeqCst),
            reads + 2,
            "manual refresh reads original history and fills"
        );
        assert_eq!(service.plan_store.get(&plan.plan_id).unwrap(), complete);
        assert_eq!(std::fs::read(&path).unwrap(), journal);

        if conflict == "changed_fee" {
            mock.fills.lock()[0]["fee"] = json!("0.06");
        } else {
            mock.order.lock().as_mut().unwrap()["status"] = json!("Cancelled");
        }
        assert!(service
            .recheck_stock_order(&plan.plan_id, hub.clone())
            .await
            .is_err());
        stop(&service).await;
        let disputed = service.plan_store.get(&plan.plan_id).unwrap();
        let record = disputed.cex_order.as_ref().unwrap();
        assert!(record.evidence_conflict && record.recheck.paused);
        assert!(!record.receipt_complete() && !record.needs_follow_up());
        assert_eq!(record.fills, complete.cex_order.as_ref().unwrap().fills);
        assert!(disputed.holds_funds(i64::MAX));
        assert!(!disputed.accounting().can_settle());

        let journal = std::fs::read(&path).unwrap();
        assert!(service
            .recheck_stock_order(&plan.plan_id, hub.clone())
            .await
            .is_err());
        stop(&service).await;
        assert_eq!(
            std::fs::read(&path).unwrap(),
            journal,
            "repeated conflict must not append again"
        );
        service
            .stock_order_problem(&plan.plan_id, "temporary transport failure".into())
            .unwrap();
        assert_eq!(service.plan_store.get(&plan.plan_id).unwrap(), disputed);
        mock.settle(false);
        service
            .recheck_stock_order(&plan.plan_id, hub.clone())
            .await
            .unwrap();
        stop(&service).await;
        assert_eq!(
            service.plan_store.get(&plan.plan_id).unwrap(),
            disputed,
            "a later matching receipt cannot erase a conflict"
        );
        assert_eq!(std::fs::read(&path).unwrap(), journal);
        drop(service);

        let (mut restored, _) = BackpackStocks::stock_plan_fixture(path, common::time::now_ms());
        restored.root = root;
        assert!(restored.plan_store.problem().is_none());
        assert_eq!(restored.plan_store.get(&plan.plan_id).unwrap(), disputed);
        let reads = mock.reads.load(Ordering::SeqCst);
        restored
            .reconcile_stock_order(&plan.plan_id, &keys().unwrap(), true)
            .await
            .unwrap();
        assert_eq!(mock.reads.load(Ordering::SeqCst), reads);
        assert!(restored
            .wallet_claims
            .check("solana", &plan.request.wallet_address, i64::MAX)
            .is_err());
        assert_eq!(mock.posts.load(Ordering::SeqCst), 1);
    }
}
