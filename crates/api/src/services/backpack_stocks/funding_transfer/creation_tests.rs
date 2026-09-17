use super::*;

#[tokio::test]
async fn stock_funding_transfer_creation_exact_message_budget_races_and_receipts() {
    for asset in ["MU.US", "USDC"] {
        let now = common::time::now_ms();
        let mut plan = fixture(now, asset);
        let tmp = tempfile::tempdir().unwrap();
        let (mock, root, _server) = server(plan.clone(), tmp.path().join("funding.jsonl")).await;
        mock.create_destination.store(true, Ordering::SeqCst);
        let rpc = format!("{root}/rpc");
        let p = chain::prepare_with(&client(), &rpc, &plan, 890880)
            .await
            .unwrap();
        let creation = p.account_creation.as_ref().unwrap();
        assert_eq!(
            creation.account_size,
            if asset == "USDC" { 165 } else { 174 }
        );
        assert_eq!(creation.rent_budget_lamports, mock.rent());
        assert_eq!(
            p.destination_token_account.as_deref(),
            Some(mock.ata().as_str())
        );
        let raw = STANDARD.decode(&p.transaction_base64).unwrap();
        assert_eq!(&raw[65..69], &[1, 0, 5, 8]);
        assert_eq!(
            &raw[357..376],
            &[2, 7, 6, 0, 2, 5, 3, 6, 4, 1, 1, 4, 4, 1, 3, 2, 0, 10, 12]
        );
        chain::check_with(&client(), &rpc, &plan, &p).await.unwrap();
        for bad in [
            "creation_owner",
            "missing_size",
            "account_size",
            "rent_increased",
            "balance",
            "paused",
        ] {
            *mock.bad.lock() = bad.into();
            assert!(
                chain::check_with(&client(), &rpc, &plan, &p).await.is_err(),
                "{asset}: {bad}"
            );
        }
        *mock.bad.lock() = String::new();
        *mock.bad.lock() = "destination_frozen".into();
        let error = chain::prepare_with(&client(), &rpc, &plan, 890880)
            .await
            .unwrap_err();
        assert!(
            error.contains("均被冻结"),
            "a frozen non-ATA must not be treated as an absent recipient: {error}"
        );
        *mock.bad.lock() = String::new();
        mock.destination_ready.store(true, Ordering::SeqCst);
        chain::check_with(&client(), &rpc, &plan, &p).await.unwrap();
        assert_eq!(
            chain::message(&plan, &p).unwrap(),
            raw[65..],
            "another party creating the ATA must not rewrite our message"
        );
        mock.destination_ready.store(false, Ordering::SeqCst);
        let signed = sign(&p);
        let hash = chain::signed_identity(&plan, &p, &signed).unwrap();
        *mock.encoded.lock() = signed;
        mock.finalized.store(true, Ordering::SeqCst);
        let at = common::time::now_ms();
        plan.phase = StockFundingPlanPhase::Transferring;
        plan.updated_at_ms = at;
        plan.transfer = Some(StockFundingTransfer {
            preparation: p,
            submitted_at_ms: Some(at),
            transaction_hash: Some(hash),
            acknowledged: false,
            query_count: 0,
            last_query_at_ms: None,
            receipt: None,
            deposit: None,
            problem: None,
        });
        for (bad, ready, expense) in [
            ("", false, mock.rent()),
            ("", true, 0),
            ("prefunded", false, mock.rent() - 10000),
        ] {
            *mock.bad.lock() = bad.into();
            mock.destination_ready.store(ready, Ordering::SeqCst);
            let receipt = chain::receipt_with(&client(), &rpc, &plan).await.unwrap();
            assert!(receipt.within_plan && receipt.succeeded);
            assert_eq!(receipt.account_creation_lamports, expense);
            assert_eq!(
                receipt.wallet_debit_lamports,
                expense + receipt.network_fee_lamports
            );
            let mut complete = plan.clone();
            complete.updated_at_ms = receipt.checked_at_ms;
            complete.phase = StockFundingPlanPhase::DepositPending;
            complete.transfer.as_mut().unwrap().receipt = Some(receipt);
            validate(&complete).unwrap();
            let mut wrong = complete.clone();
            wrong
                .transfer
                .as_mut()
                .unwrap()
                .receipt
                .as_mut()
                .unwrap()
                .account_creation_lamports += 1;
            assert!(
                validate(&wrong).is_err(),
                "journal cannot fabricate rent or alter wallet debit"
            );
        }
        *mock.bad.lock() = "missing_init".into();
        assert!(
            chain::receipt_with(&client(), &rpc, &plan).await.is_err(),
            "missing old token balance is not assumed zero"
        );
        *mock.bad.lock() = "receipt_rent".into();
        let receipt = chain::receipt_with(&client(), &rpc, &plan).await.unwrap();
        assert!(!receipt.within_plan);
        assert_eq!(
            receipt.account_creation_lamports,
            mock.rent() + 1000,
            "actual expense is preserved even outside budget"
        );
        assert_eq!(mock.sends.load(Ordering::SeqCst), 0);
    }
}
