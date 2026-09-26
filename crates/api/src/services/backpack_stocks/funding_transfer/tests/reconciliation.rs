use super::*;

pub(super) async fn received(
    service: &BackpackStocks,
    plan: &StockFundingPlan,
    mock: &Mock,
    root: &str,
    existing_deposit: bool,
) -> StockFundingPlan {
    let rpc = format!("{root}/rpc");
    let preparation = chain::prepare_with(&client(), &rpc, plan, 890880)
        .await
        .unwrap();
    let signed = sign(&preparation);
    let hash = chain::signed_identity(plan, &preparation, &signed).unwrap();
    *mock.encoded.lock() = signed;
    mock.finalized.store(true, Ordering::SeqCst);
    service
        .funding_store
        .insert(plan.clone(), common::time::now_ms())
        .unwrap();
    let ready = service
        .funding_store
        .update_transfer(
            plan,
            StockFundingTransfer {
                deposit_scan: None,
                preparation,
                submitted_at_ms: None,
                transaction_hash: None,
                acknowledged: false,
                query_count: 0,
                last_query_at_ms: None,
                receipt: None,
                deposit: None,
                problem: None,
                evidence_conflict: None,
            },
            common::time::now_ms(),
        )
        .unwrap();
    let mut transfer = ready.transfer.clone().unwrap();
    let at = common::time::now_ms();
    transfer.submitted_at_ms = Some(at);
    transfer.transaction_hash = Some(hash);
    let sent = service
        .funding_store
        .update_transfer(&ready, transfer.clone(), at)
        .unwrap();
    transfer.receipt = Some(chain::receipt_with(&client(), &rpc, &sent).await.unwrap());
    transfer.deposit =
        existing_deposit.then(|| serde_json::from_value(remote(&sent, "pending")).unwrap());
    service
        .funding_store
        .update_transfer(&sent, transfer, common::time::now_ms())
        .unwrap()
}

#[tokio::test]
async fn stock_funding_transfer_deposit_merges_equivalent_values_and_rejects_evidence_erasure() {
    let dir = tempfile::tempdir().unwrap();
    let now = common::time::now_ms();
    let plan = fixture(now, "MU.US");
    let path = dir.path().join("funding.jsonl");
    let (mock, root, _server) = server(plan.clone(), path.clone()).await;
    let service = BackpackStocks::stock_plan_fixture(dir.path().join("plans.jsonl"), now)
        .0
        .with_funding_store(path.clone());
    let saved = received(&service, &plan, &mock, &root, true).await;
    let previous = saved.transfer.as_ref().unwrap();
    let mut same = remote(&saved, "pending");
    same["quantity"] = json!(format!("{:.8}", decimal(&plan.terms.quantity).unwrap()));
    same["createdAt"] = json!(chrono::DateTime::parse_from_rfc3339(
        same["createdAt"].as_str().unwrap()
    )
    .unwrap()
    .with_timezone(&chrono::FixedOffset::east_opt(8 * 3600).unwrap())
    .to_rfc3339());
    same.as_object_mut().unwrap().remove("fromAddress");
    same.as_object_mut().unwrap().remove("toAddress");
    let merged = deposit_from_rows(&saved, &json!([same])).unwrap().unwrap();
    assert_eq!(Some(merged.clone()), previous.deposit);
    let before = std::fs::read(&path).unwrap();
    let mut unchanged = previous.clone();
    unchanged.deposit = Some(merged);
    assert_eq!(
        service
            .funding_store
            .update_transfer(&saved, unchanged, common::time::now_ms())
            .unwrap(),
        saved
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let encoded = serde_json::to_value(previous).unwrap();
    assert!(encoded.get("evidenceConflict").is_none());
    assert_eq!(
        serde_json::from_value::<StockFundingTransfer>(encoded).unwrap(),
        *previous
    );

    // Older journals allowed pending quantities and decimal formatting to update.
    let mut pending = previous.clone();
    pending.deposit.as_mut().unwrap().quantity = "0.01".into();
    let interim = service
        .funding_store
        .update_transfer(&saved, pending.clone(), common::time::now_ms())
        .unwrap();
    pending.deposit.as_mut().unwrap().quantity = "0.02000000".into();
    let saved = service
        .funding_store
        .update_transfer(&interim, pending, common::time::now_ms())
        .unwrap();
    let previous = saved.transfer.as_ref().unwrap();

    let mut sparse = saved.clone();
    let old = sparse.transfer.as_mut().unwrap().deposit.as_mut().unwrap();
    old.to_address = None;
    old.from_address = None;
    let enriched = deposit_from_rows(&sparse, &json!([remote(&saved, "confirmed")]))
        .unwrap()
        .unwrap();
    assert_eq!(enriched.to_address.as_ref(), Some(&plan.terms.destination));
    assert_eq!(
        enriched.from_address.as_ref(),
        Some(&plan.request.wallet_address)
    );

    let mut confirmed = saved.clone();
    confirmed
        .transfer
        .as_mut()
        .unwrap()
        .deposit
        .as_mut()
        .unwrap()
        .status = "confirmed".into();
    let mut changed = remote(&saved, "confirmed");
    changed["quantity"] = json!("0.03");
    assert!(matches!(
        deposit_from_rows(&confirmed, &json!([changed])),
        Err(ReadProblem::Conflict(_))
    ));
    assert!(matches!(
        deposit_from_rows(&confirmed, &json!([remote(&saved, "pending")])),
        Err(ReadProblem::Conflict(_))
    ));

    let mut conflict = previous.clone();
    ReadProblem::Conflict("原入账数量冲突".into()).record(&mut conflict);
    let frozen = service
        .funding_store
        .update_transfer(&saved, conflict, common::time::now_ms())
        .unwrap();
    assert!(frozen.funding_followup_at().is_none());
    let mut retry = frozen.transfer.clone().unwrap();
    ReadProblem::Unavailable("历史暂不可读".into()).record(&mut retry);
    assert_eq!(
        retry.evidence_conflict,
        frozen.transfer.as_ref().unwrap().evidence_conflict
    );
    let mut erased = retry.clone();
    erased.evidence_conflict = None;
    assert!(service
        .funding_store
        .update_transfer(&frozen, erased, common::time::now_ms())
        .is_err());
    let updated = service
        .funding_store
        .update_transfer(&frozen, retry, common::time::now_ms())
        .unwrap();
    drop(service);
    let claims = Arc::new(crate::services::onchain_wallet_claims::WalletClaims::default());
    let restored = funding_store::FundingStore::load(Some(path.clone()), claims.clone());
    assert!(restored.problem().is_none());
    assert_eq!(restored.get(&plan.plan_id).unwrap(), updated);
    assert!(claims
        .check("solana", &plan.request.wallet_address, now + 1_000_000)
        .is_err());
    assert_account_held(&claims, now + 1_000_000);
    drop(restored);
    // Replay must also refuse an appended record which erases the conflict.
    let mut lines = std::fs::read_to_string(&path).unwrap();
    let mut tail: Value = serde_json::from_str(lines.lines().last().unwrap()).unwrap();
    tail["plan"]["revision"] = json!(updated.revision + 1);
    tail["plan"]["transfer"]
        .as_object_mut()
        .unwrap()
        .remove("evidenceConflict");
    lines.push_str(&format!("{}\n", tail));
    std::fs::write(&path, lines).unwrap();
    let reopened = funding_store::FundingStore::load(Some(path), claims);
    assert!(reopened.problem().is_some());
}

#[tokio::test]
async fn stock_funding_transfer_deposit_conflicts_survive_http_restart_and_later_confirmation() {
    for case in 0..13 {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("funding.jsonl");
        let now = common::time::now_ms();
        let plan = fixture(now, "MU.US");
        let (mock, root, _server) = server(plan.clone(), path.clone()).await;
        let mut service = BackpackStocks::stock_plan_fixture(dir.path().join("plans.jsonl"), now)
            .0
            .with_funding_store(path.clone());
        service.root = root.clone();
        let saved = received(&service, &plan, &mock, &root, case != 12).await;
        let mut row = remote(&saved, "confirmed");
        match case {
            1 => row["id"] = json!(99),
            2 => row["source"] = json!("ethereum"),
            3 => row["symbol"] = json!("USDC"),
            4 => row["toAddress"] = json!(address(3)),
            5 | 12 => row["quantity"] = json!("123"),
            6 => {
                row["createdAt"] = json!(chrono::DateTime::from_timestamp_millis(
                    saved.transfer.as_ref().unwrap().submitted_at_ms.unwrap() + 1000
                )
                .unwrap()
                .to_rfc3339())
            }
            9 => {
                row.as_object_mut().unwrap().remove("quantity");
            }
            _ => {}
        }
        let mut pages = vec![json!({"transactionHash":"other-transaction"}); 100];
        pages[0] = row.clone();
        let mut duplicate = row.clone();
        duplicate["id"] = json!(99);
        pages.push(duplicate);
        *mock.history.lock() = match case {
            0 => json!([row.clone(), row]),
            7 => json!([]),
            8 => json!({"temporary":"invalid history envelope"}),
            10 => json!(pages),
            11 => json!([remote(&saved, "pending")]),
            _ => json!([row]),
        };
        service
            .recheck_funding_transfer_with(&saved, &rfq_tests::keys().unwrap(), |_| async {
                panic!("saved chain receipt must not be queried again")
            })
            .await
            .unwrap();
        let current = service.funding_store.get(&plan.plan_id).unwrap();
        let transfer = current.transfer.as_ref().unwrap();
        assert_eq!(
            transfer.evidence_conflict.is_some(),
            case <= 6 || case == 10 || case == 12,
            "case {case}"
        );
        assert_eq!(transfer.receipt, saved.transfer.as_ref().unwrap().receipt);
        if case == 5 || case == 12 {
            assert_eq!(transfer.deposit.as_ref().unwrap().quantity, "123");
        } else if case == 10 {
            assert_eq!(transfer.deposit.as_ref().unwrap().status, "confirmed");
        } else {
            assert_eq!(transfer.deposit, saved.transfer.as_ref().unwrap().deposit);
        }
        assert_eq!(current.phase, StockFundingPlanPhase::DepositPending);
        assert_eq!(
            current.funding_followup_at().is_none(),
            transfer.evidence_conflict.is_some()
        );
        assert_eq!(mock.sends.load(Ordering::SeqCst), 0);
        if case != 0 {
            continue;
        }

        drop(service);
        let mut restored =
            BackpackStocks::stock_plan_fixture(dir.path().join("restart.jsonl"), now)
                .0
                .with_funding_store(path.clone());
        restored.root = root.clone();
        assert_eq!(restored.funding_store.get(&plan.plan_id).unwrap(), current);
        assert!(restored.funding_store.next_followup().is_none());
        *mock.history.lock() = json!([remote(&saved, "confirmed")]);
        let queries = mock.queries.load(Ordering::SeqCst);
        restored
            .recheck_funding_transfer_with(&current, &rfq_tests::keys().unwrap(), |_| async {
                panic!("cooldown cannot query chain")
            })
            .await
            .unwrap();
        assert_eq!(mock.queries.load(Ordering::SeqCst), queries);
        tokio::time::sleep(Duration::from_millis(5010)).await;
        restored
            .recheck_funding_transfer_with(&current, &rfq_tests::keys().unwrap(), |_| async {
                panic!("restart must preserve original receipt")
            })
            .await
            .unwrap();
        let still_held = restored.funding_store.get(&plan.plan_id).unwrap();
        let observed = still_held.transfer.as_ref().unwrap();
        assert_eq!(observed.deposit.as_ref().unwrap().status, "confirmed");
        assert_eq!(observed.evidence_conflict, transfer.evidence_conflict);
        assert_eq!(observed.receipt, transfer.receipt);
        assert_eq!(still_held.phase, StockFundingPlanPhase::DepositPending);
        assert!(still_held.funding_followup_at().is_none());
        assert!(restored
            .wallet_claims
            .check("solana", &plan.request.wallet_address, now + 1_000_000)
            .is_err());
        assert_account_held(&restored.wallet_claims, now + 1_000_000);
        assert_eq!(mock.sends.load(Ordering::SeqCst), 0);
        if let Ok(path) = std::env::var("STOCK_FUNDING_TRANSFER_CONFLICT_CAPTURE_PATH") {
            std::fs::write(
                path,
                serde_json::to_vec_pretty(&restored.snapshot()).unwrap(),
            )
            .unwrap();
        }
    }
}
