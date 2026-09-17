use super::*;

fn received(
    service: &BackpackStocks,
    plan: &StockFundingPlan,
    fee: Option<&str>,
) -> StockFundingPlan {
    let now = plan.terms.created_at_ms;
    service.funding_store.insert(plan.clone(), now).unwrap();
    let begun = service
        .funding_store
        .update(plan, intent(plan, now), now)
        .unwrap();
    let mut w = begun.withdrawal.clone().unwrap();
    let mut value = remote(plan);
    value["fee"] = json!(fee);
    w.remote = Some(parse(&begun, value).unwrap());
    w.receipt = Some(StockFundingReceipt {
        transaction_hash: bs58::encode([7; 64]).into_string(),
        destination: plan.terms.destination.clone(),
        mint: plan.terms.token.contract_address.clone().unwrap(),
        decimals: 6,
        credited_raw: "10000000".into(),
        slot: plan.terms.mint.slot + 1,
        block_time_ms: now,
        network_fee_lamports: 5000,
        fee_payer: bs58::encode([6; 32]).into_string(),
        checked_at_ms: now,
    });
    service.funding_store.update(&begun, w, now).unwrap()
}

#[test]
fn stock_funding_withdrawal_merges_late_fee_without_erasing_evidence_or_legacy_encoding() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("funding.jsonl");
    let service = BackpackStocks::stock_plan_fixture(dir.path().join("plans.jsonl"), 10_000)
        .0
        .with_funding_store(path.clone());
    let plan = funding_plan::tests::fixture(10_000);
    let saved = received(&service, &plan, Some("0.5"));
    let mut same = remote(&plan);
    same["quantity"] = json!(format!("{:.4}", decimal(&plan.terms.quantity).unwrap()));
    same["fee"] = json!("0.5000");
    let previous = saved.withdrawal.as_ref().unwrap();
    assert_eq!(
        parse(&saved, same).unwrap(),
        previous.remote.clone().unwrap()
    );
    let mut sparse = remote(&plan);
    sparse.as_object_mut().unwrap().remove("fee");
    sparse.as_object_mut().unwrap().remove("transactionHash");
    assert_eq!(
        parse(&saved, sparse).unwrap(),
        previous.remote.clone().unwrap()
    );
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(
        service
            .funding_store
            .update(&saved, previous.clone(), 10_001)
            .unwrap(),
        saved
    );
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    let encoded = serde_json::to_value(previous).unwrap();
    assert!(encoded.get("evidenceConflict").is_none());
    assert_eq!(
        serde_json::from_value::<StockFundingWithdrawal>(encoded).unwrap(),
        *previous
    );

    let mut conflict = previous.clone();
    ReadProblem::Conflict("原提现费用冲突".into()).record(&mut conflict);
    let frozen = service
        .funding_store
        .update(&saved, conflict, 10_001)
        .unwrap();
    assert!(frozen.funding_followup_at().is_none());
    let mut unavailable = frozen.withdrawal.clone().unwrap();
    ReadProblem::Unavailable("历史暂不可读".into()).record(&mut unavailable);
    assert_eq!(
        unavailable.evidence_conflict.as_deref(),
        Some("原提现费用冲突")
    );
    let mut erased = frozen.withdrawal.clone().unwrap();
    erased.evidence_conflict = None;
    assert!(service
        .funding_store
        .update(&frozen, erased, 10_002)
        .is_err());
    drop(service);
    let claims = Arc::new(WalletClaims::default());
    let restored = funding_store::FundingStore::load(Some(path), claims.clone());
    assert!(restored.problem().is_none());
    assert_eq!(restored.get(&plan.plan_id).unwrap(), frozen);
    assert!(claims
        .check("solana", &plan.request.wallet_address, 1_000_000)
        .is_err());
}

#[tokio::test]
async fn stock_funding_withdrawal_received_recheck_reads_original_history_and_preserves_conflicts()
{
    let history = Arc::new(Mutex::new(json!([])));
    let get_count = Arc::new(AtomicUsize::new(0));
    let response = history.clone();
    let calls = get_count.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let root = format!("http://{}", listener.local_addr().unwrap());
    let router = Router::new().route(
        PATH,
        get(
            move |headers: HeaderMap, Query(q): Query<BTreeMap<String, String>>| {
                let reply = response.clone();
                let calls = calls.clone();
                async move {
                    assert_eq!(q.len(), 2);
                    assert_eq!(q["limit"], "2");
                    rfq_tests::signed(&headers, "withdrawalQueryAll", q);
                    calls.fetch_add(1, Ordering::SeqCst);
                    Json(reply.lock().clone())
                }
            },
        ),
    );
    let _server = Server(tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap()
    }));
    for case in 0..11 {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("funding.jsonl");
        let now = common::time::now_ms() - 10_000;
        let plan = funding_plan::tests::fixture(now);
        let mut service = BackpackStocks::stock_plan_fixture(dir.path().join("plans.jsonl"), now)
            .0
            .with_funding_store(path.clone());
        service.root = root.clone();
        let saved = received(
            &service,
            &plan,
            if case == 0 || case == 10 {
                None
            } else {
                Some("0.5")
            },
        );
        let mut row = remote(&plan);
        match case {
            1 => row["id"] = json!(44),
            2 => row["transactionHash"] = json!(bs58::encode([9; 64]).into_string()),
            3 => row["fee"] = json!("0.6"),
            4 => row["status"] = json!("processing"),
            5 => row["toAddress"] = json!(bs58::encode([5; 32]).into_string()),
            6 => row["quantity"] = json!("100"),
            10 => row["fee"] = json!("0"),
            _ => {}
        }
        *history.lock() = match case {
            7 => json!([row.clone(), row]),
            8 => json!([]),
            9 => json!([{"id":43}]),
            _ => json!([row]),
        };
        let before = get_count.load(Ordering::SeqCst);
        service
            .recheck_funding_with(&plan.plan_id, &rfq_tests::keys().unwrap(), |_, _| async {
                panic!("previous finalized receipt must not be fetched again")
            })
            .await
            .unwrap();
        assert_eq!(get_count.load(Ordering::SeqCst), before + 1, "case {case}");
        let current = service.funding_store.get(&plan.plan_id).unwrap();
        let w = current.withdrawal.as_ref().unwrap();
        assert_eq!(w.receipt, saved.withdrawal.as_ref().unwrap().receipt);
        assert_eq!(
            w.evidence_conflict.is_some(),
            (1..=7).contains(&case),
            "case {case}"
        );
        if case == 0 || case == 10 {
            assert_eq!(
                w.remote.as_ref().unwrap().fee.as_deref(),
                Some(if case == 0 { "0.5" } else { "0" })
            );
            assert!(w.problem.as_deref().unwrap().contains("扣账"));
        } else {
            assert_eq!(w.remote, saved.withdrawal.as_ref().unwrap().remote);
        }
        assert!(current.phase.holds_funds());
        assert!(current.funding_followup_at().is_none());
        if case != 3 {
            continue;
        }

        // A restart and a later matching history can supplement evidence, never clear its conflict.
        drop(service);
        let mut restored =
            BackpackStocks::stock_plan_fixture(dir.path().join("restart.jsonl"), now)
                .0
                .with_funding_store(path.clone());
        restored.root = root.clone();
        assert_eq!(restored.funding_store.get(&plan.plan_id).unwrap(), current);
        assert!(restored.funding_store.next_followup().is_none());
        assert!(restored
            .wallet_claims
            .check("solana", &plan.request.wallet_address, now + 1_000_000)
            .is_err());
        *history.lock() = json!([remote(&plan)]);
        assert!(restored
            .recheck_funding_with(&plan.plan_id, &rfq_tests::keys().unwrap(), |_, _| async {
                panic!("must respect original query cooldown")
            })
            .await
            .is_err());
        tokio::time::sleep(Duration::from_millis(QUERY_INTERVAL_MS as u64)).await;
        restored
            .recheck_funding_with(&plan.plan_id, &rfq_tests::keys().unwrap(), |_, _| async {
                panic!("must reuse receipt after restart")
            })
            .await
            .unwrap();
        let final_plan = restored.funding_store.get(&plan.plan_id).unwrap();
        assert_eq!(
            final_plan.withdrawal.as_ref().unwrap().evidence_conflict,
            w.evidence_conflict
        );
        assert_eq!(final_plan.withdrawal.as_ref().unwrap().receipt, w.receipt);
        assert!(final_plan.funding_followup_at().is_none());
        if let Ok(path) = std::env::var("STOCK_FUNDING_RECONCILIATION_CAPTURE_PATH") {
            std::fs::write(path, serde_json::to_vec(&restored.snapshot()).unwrap()).unwrap();
        }
    }
}
