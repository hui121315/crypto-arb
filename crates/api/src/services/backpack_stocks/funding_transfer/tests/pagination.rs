use super::*;

fn history(plan: &StockFundingPlan, count: usize) -> Value {
    let mut rows = (0..count)
        .map(|i| json!({"id":1000+i,"transactionHash":format!("other-local-transfer-{i}")}))
        .collect::<Vec<_>>();
    rows[50] = remote(plan, "confirmed");
    json!(rows)
}

fn offsets(mock: &Mock) -> Vec<u32> {
    mock.history_requests
        .lock()
        .iter()
        .map(|r| r["offset"].parse().unwrap())
        .collect()
}

fn held(service: &BackpackStocks, plan: &StockFundingPlan) {
    let at = common::time::now_ms() + 86_400_000;
    assert_eq!(plan.phase, StockFundingPlanPhase::DepositPending);
    assert!(service
        .wallet_claims
        .check("solana", &plan.request.wallet_address, at)
        .is_err());
    assert_account_held(&service.wallet_claims, at);
}

#[tokio::test]
async fn stock_funding_transfer_history_pagination_restart_continues_beyond_400_without_early_release(
) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("funding.jsonl");
    let now = common::time::now_ms();
    let plan = fixture(now, "MU.US");
    let (mock, root, _server) = server(plan.clone(), path.clone()).await;
    let mut service = BackpackStocks::stock_plan_fixture(dir.path().join("plans.jsonl"), now)
        .0
        .with_funding_store(path.clone());
    service.root = root.clone();
    let received = reconciliation::received(&service, &plan, &mock, &root, true).await;
    *mock.history.lock() = history(&received, 650);
    service
        .recheck_funding_transfer_with(&received, &rfq_tests::keys().unwrap(), |_| async {
            panic!("original chain receipt already saved")
        })
        .await
        .unwrap();
    let pending = service.funding_store.get(&plan.plan_id).unwrap();
    held(&service, &pending);
    let t = pending.transfer.as_ref().unwrap();
    assert_eq!(t.deposit.as_ref().unwrap().status, "confirmed");
    assert_eq!(t.deposit_scan.as_ref().unwrap().scanned_rows, 400);
    assert!(t.deposit_scan.as_ref().unwrap().completed_at_ms.is_none());
    assert!(service.account.read().evidence.is_some());
    assert_eq!(offsets(&mock), [0, 100, 200, 300]);
    let initial = service.snapshot();
    drop(service);

    let mut restored = BackpackStocks::stock_plan_fixture(dir.path().join("restart.jsonl"), now)
        .0
        .with_funding_store(path.clone());
    restored.root = root;
    assert_eq!(restored.funding_store.get(&plan.plan_id).unwrap(), pending);
    restored
        .recheck_funding_transfer_with(&pending, &rfq_tests::keys().unwrap(), |_| async {
            panic!("cooldown")
        })
        .await
        .unwrap();
    assert_eq!(mock.queries.load(Ordering::SeqCst), 4);
    tokio::time::sleep(Duration::from_millis(5010)).await;
    restored
        .recheck_funding_transfer_with(&pending, &rfq_tests::keys().unwrap(), |_| async {
            panic!("must reuse receipt")
        })
        .await
        .unwrap();
    let completed = restored.funding_store.get(&plan.plan_id).unwrap();
    assert_eq!(completed.phase, StockFundingPlanPhase::Deposited);
    let scan = completed
        .transfer
        .as_ref()
        .unwrap()
        .deposit_scan
        .as_ref()
        .unwrap();
    assert_eq!(scan.scanned_rows, 650);
    assert!(scan.completed_at_ms.is_some() && scan.matched);
    assert_eq!(offsets(&mock), [0, 100, 200, 300, 300, 400, 500, 600]);
    let requests = mock.history_requests.lock();
    assert!(requests
        .iter()
        .all(|r| r["from"] == requests[0]["from"] && r["to"] == requests[0]["to"]));
    drop(requests);
    assert!(restored.account.read().evidence.is_none());
    let reads = mock.queries.load(Ordering::SeqCst);
    assert!(restored
        .scan_funding_deposit(&completed, &rfq_tests::keys().unwrap())
        .await
        .is_err());
    assert_eq!(mock.queries.load(Ordering::SeqCst), reads);
    assert!(restored
        .wallet_claims
        .check(
            "solana",
            &plan.request.wallet_address,
            common::time::now_ms()
        )
        .is_ok());
    assert_eq!(mock.sends.load(Ordering::SeqCst), 0);
    if let Ok(path) = std::env::var("STOCK_FUNDING_SCAN_CAPTURE_PATH") {
        std::fs::write(
            path,
            serde_json::to_vec_pretty(&json!({"pending":initial,"completed":restored.snapshot()}))
                .unwrap(),
        )
        .unwrap();
    }
    drop(restored);
    let restored = funding_store::FundingStore::load(
        Some(path),
        Arc::new(crate::services::onchain_wallet_claims::WalletClaims::default()),
    );
    assert!(restored.problem().is_none());
    assert_eq!(restored.get(&plan.plan_id).unwrap(), completed);
}

#[tokio::test]
async fn stock_funding_transfer_history_page_failure_movement_and_late_record_never_rebroadcast() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("funding.jsonl");
    let now = common::time::now_ms();
    let plan = fixture(now, "MU.US");
    let (mock, root, _server) = server(plan.clone(), path.clone()).await;
    let mut service = BackpackStocks::stock_plan_fixture(dir.path().join("plans.jsonl"), now)
        .0
        .with_funding_store(path.clone());
    service.root = root.clone();
    let received = reconciliation::received(&service, &plan, &mock, &root, true).await;
    *mock.history.lock() = history(&received, 650);
    *mock.fail_history_offset.lock() = Some(200);
    service
        .recheck_funding_transfer_with(&received, &rfq_tests::keys().unwrap(), |_| async {
            panic!("saved receipt")
        })
        .await
        .unwrap();
    let failed = service.funding_store.get(&plan.plan_id).unwrap();
    assert_eq!(offsets(&mock), [0, 100, 200]);
    assert_eq!(
        failed
            .transfer
            .as_ref()
            .unwrap()
            .deposit_scan
            .as_ref()
            .unwrap()
            .scanned_rows,
        200
    );
    held(&service, &failed);
    *mock.fail_history_offset.lock() = None;
    mock.history.lock()[150]["id"] = json!(7777);
    // Exercise the paging worker directly; the public recheck cooldown is covered above.
    service
        .scan_funding_deposit(&failed, &rfq_tests::keys().unwrap())
        .await
        .unwrap();
    let reset = service.funding_store.get(&plan.plan_id).unwrap();
    assert_eq!(offsets(&mock).last(), Some(&100));
    assert_eq!(
        reset
            .transfer
            .as_ref()
            .unwrap()
            .deposit_scan
            .as_ref()
            .unwrap()
            .scanned_rows,
        0
    );
    assert!(reset.transfer.as_ref().unwrap().evidence_conflict.is_none());
    held(&service, &reset);
    assert_eq!(
        reset.transfer.as_ref().unwrap().deposit,
        failed.transfer.as_ref().unwrap().deposit
    );

    let mut moving = history(&received, 650);
    moving[550] = remote(&received, "confirmed");
    *mock.history.lock() = moving;
    service
        .scan_funding_deposit(&reset, &rfq_tests::keys().unwrap())
        .await
        .unwrap();
    let first = service.funding_store.get(&plan.plan_id).unwrap();
    service
        .scan_funding_deposit(&first, &rfq_tests::keys().unwrap())
        .await
        .unwrap();
    let reset = service.funding_store.get(&plan.plan_id).unwrap();
    assert!(reset.transfer.as_ref().unwrap().evidence_conflict.is_none());
    assert_eq!(
        reset
            .transfer
            .as_ref()
            .unwrap()
            .deposit_scan
            .as_ref()
            .unwrap()
            .scanned_rows,
        0
    );
    assert!(reset
        .transfer
        .as_ref()
        .unwrap()
        .problem
        .as_deref()
        .unwrap()
        .contains("跨页重现"));
    held(&service, &reset);

    *mock.history.lock() = json!([]);
    service
        .scan_funding_deposit(&reset, &rfq_tests::keys().unwrap())
        .await
        .unwrap();
    let empty = service.funding_store.get(&plan.plan_id).unwrap();
    let scan = empty
        .transfer
        .as_ref()
        .unwrap()
        .deposit_scan
        .as_ref()
        .unwrap();
    assert!(scan.completed_at_ms.is_some() && !scan.matched);
    held(&service, &empty);
    *mock.history.lock() = json!([remote(&received, "confirmed")]);
    service
        .scan_funding_deposit(&empty, &rfq_tests::keys().unwrap())
        .await
        .unwrap();
    assert_eq!(
        service.funding_store.get(&plan.plan_id).unwrap().phase,
        StockFundingPlanPhase::Deposited
    );
    assert_eq!(mock.sends.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn stock_funding_transfer_history_resume_detects_conflicts_and_rejects_cursor_tampering() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("funding.jsonl");
    let now = common::time::now_ms();
    let plan = fixture(now, "MU.US");
    let (mock, root, _server) = server(plan.clone(), path.clone()).await;
    let mut service = BackpackStocks::stock_plan_fixture(dir.path().join("plans.jsonl"), now)
        .0
        .with_funding_store(path.clone());
    service.root = root.clone();
    let received = reconciliation::received(&service, &plan, &mock, &root, true).await;
    let mut rows = history(&received, 650);
    let mut conflicting = remote(&received, "confirmed");
    conflicting["id"] = json!(18);
    rows[550] = conflicting;
    *mock.history.lock() = rows;
    service
        .recheck_funding_transfer_with(&received, &rfq_tests::keys().unwrap(), |_| async {
            panic!("saved receipt")
        })
        .await
        .unwrap();
    let pending = service.funding_store.get(&plan.plan_id).unwrap();
    for case in 0..4 {
        let mut t = pending.transfer.clone().unwrap();
        match case {
            0 => t.deposit_scan = None,
            1 => t.deposit_scan.as_mut().unwrap().scanned_rows += 200,
            2 => t.deposit_scan.as_mut().unwrap().to_ms += 1,
            _ => t.deposit_scan.as_mut().unwrap().matched = false,
        }
        assert!(
            service
                .funding_store
                .update_transfer(&pending, t, common::time::now_ms())
                .is_err(),
            "case {case}"
        );
    }
    let before = std::fs::read(&path).unwrap();
    assert_eq!(
        service
            .funding_store
            .update_transfer(
                &pending,
                pending.transfer.clone().unwrap(),
                common::time::now_ms()
            )
            .unwrap(),
        pending
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let backup = path.with_extension("backup");
    std::fs::rename(&path, &backup).unwrap();
    std::fs::create_dir(&path).unwrap();
    assert!(service
        .scan_funding_deposit(&pending, &rfq_tests::keys().unwrap())
        .await
        .is_err());
    assert!(service.funding_store.problem().is_some());
    assert!(service.funding_store.get(&plan.plan_id).is_err());
    assert!(service.funding_store.next_followup().is_none());
    assert_eq!(service.funding_store.records(), vec![pending.clone()]);
    assert_eq!(std::fs::read(&backup).unwrap(), before);
    held(&service, &pending);
    std::fs::remove_dir(&path).unwrap();
    std::fs::rename(backup, &path).unwrap();
    drop(service);
    let mut service = BackpackStocks::stock_plan_fixture(dir.path().join("restart.jsonl"), now)
        .0
        .with_funding_store(path.clone());
    service.root = root;
    assert!(service.funding_store.problem().is_none());
    assert_eq!(service.funding_store.get(&plan.plan_id).unwrap(), pending);
    held(&service, &pending);
    service
        .scan_funding_deposit(&pending, &rfq_tests::keys().unwrap())
        .await
        .unwrap();
    let conflicted = service.funding_store.get(&plan.plan_id).unwrap();
    held(&service, &conflicted);
    assert!(conflicted
        .transfer
        .as_ref()
        .unwrap()
        .evidence_conflict
        .is_some());
    assert!(conflicted.funding_followup_at().is_none());
    assert_eq!(
        conflicted
            .transfer
            .as_ref()
            .unwrap()
            .deposit
            .as_ref()
            .unwrap()
            .id,
        17
    );
    assert_eq!(mock.sends.load(Ordering::SeqCst), 0);
    drop(service);
    let claims = Arc::new(crate::services::onchain_wallet_claims::WalletClaims::default());
    let restored = funding_store::FundingStore::load(Some(path.clone()), claims.clone());
    assert!(restored.problem().is_none());
    assert_eq!(restored.get(&plan.plan_id).unwrap(), conflicted);
    drop(restored);
    let mut log = std::fs::read_to_string(&path).unwrap();
    let mut row: Value = serde_json::from_str(log.lines().last().unwrap()).unwrap();
    row["plan"]["revision"] = json!(conflicted.revision + 1);
    row["plan"]["transfer"]["depositScan"]["scannedRows"] = json!(99_900);
    log.push_str(&format!("{row}\n"));
    std::fs::write(&path, log).unwrap();
    let broken = funding_store::FundingStore::load(Some(path), claims);
    assert!(broken.problem().is_some());
}
