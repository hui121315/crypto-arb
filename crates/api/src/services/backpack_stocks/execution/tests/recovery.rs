use super::*;
use crate::services::backpack_stocks::{native_topup::tests::valuation, recovery as logic};
use chain::tests::{attach_variant, finalized_native};

fn quote(plan: &StockExecutionPlan, salt: u8) -> StockRecovery {
    let now = common::time::now_ms();
    let target = plan.recovery_target().unwrap();
    let mut cost = plan.terms.chain_cost.clone();
    cost.direction = target.direction;
    cost.mint.checked_at_ms = now;
    cost.mint.slot = 13;
    cost.quote.requested_at_ms = now;
    cost.quote.received_at_ms = now;
    cost.quote.expires_at_ms = None;
    cost.checked_at_ms = now;
    cost.valid_until_ms = now + 5000;
    if target.direction == StockChainDirection::Buy {
        cost.quote.input_mint = shared_types::stocks::comparison::SOLANA_USDC.into();
        cost.quote.output_mint = cost.mint.address.clone();
        cost.quote.input_raw = "13000000".into();
        cost.quote.output_raw = target.stock_raw.clone();
        cost.quote.minimum_output_raw = target.stock_raw.clone();
    } else {
        cost.quote.input_mint = cost.mint.address.clone();
        cost.quote.output_mint = shared_types::stocks::comparison::SOLANA_USDC.into();
        cost.quote.input_raw = target.stock_raw.clone();
        cost.quote.output_raw = "9000000".into();
        cost.quote.minimum_output_raw = "9000000".into();
    }
    attach_variant(&mut cost, salt);
    cost.simulation_slot = Some(13);
    cost.native_valuation = Some(valuation(&cost, 7000, 10_000, now, salt + 30));
    let cash = if target.direction == StockChainDirection::Buy {
        logic::number("-13").unwrap()
    } else {
        logic::number("9").unwrap()
    };
    let minimum = logic::number(plan.accounting().net_usdc_change.as_deref().unwrap()).unwrap()
        + cash
        - logic::native_budget(plan).unwrap()
        - logic::number("0.01").unwrap();
    StockRecovery {
        source_revision: plan.revision,
        prepared_at_ms: now,
        max_loss_usdc: "5".into(),
        target,
        cost,
        wallet: StockWalletEvidence {
            owner: plan.request.wallet_address.clone(),
            mint: plan.terms.chain_cost.mint.address.clone(),
            stock_raw: Some("10000000".into()),
            usdc_raw: Some("100000000".into()),
            sol_lamports: Some("1000000000".into()),
            checked_at_ms: now,
            problems: vec![],
        },
        minimum_net_usdc: minimum.normalize().to_string(),
        cancelled_at_ms: None,
        submission: None,
    }
}

#[derive(Clone)]
struct RecoveryMock {
    cost: StockChainCost,
    path: std::path::PathBuf,
    posts: Arc<AtomicUsize>,
    failed: bool,
}
async fn recovery_server(mock: RecoveryMock) -> (String, Server) {
    let app =
        Router::new()
            .route(
                "/execute",
                post(
                    |State(m): State<RecoveryMock>, Json(body): Json<Value>| async move {
                        let journal: Value = serde_json::from_str(
                            std::fs::read_to_string(&m.path)
                                .unwrap()
                                .lines()
                                .last()
                                .unwrap(),
                        )
                        .unwrap();
                        let key = if m.cost.quote.output_mint == STOCK_WRAPPED_SOL {
                            "nativeTopups"
                        } else {
                            "recoveries"
                        };
                        assert!(journal["plan"][key].as_array().unwrap().last().unwrap()
                            ["submission"]["walletSignature"]
                            .is_string());
                        assert_eq!(body["signedTransaction"], signed(&m.cost).unwrap());
                        m.posts.fetch_add(1, Ordering::SeqCst);
                        "reply lost after recovery"
                    },
                ),
            )
            .route(
                "/rpc",
                post(
                    |State(m): State<RecoveryMock>, Json(body): Json<Value>| async move {
                        let (id, receipt) = if m.cost.quote.output_mint == STOCK_WRAPPED_SOL {
                            finalized_native(&m.cost, m.failed)
                        } else {
                            finalized(&m.cost, m.failed)
                        };
                        let result = match body["method"].as_str().unwrap() {
                            "getGenesisHash" => json!("5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d"),
                            "getTransaction" => {
                                assert_eq!(body["params"][0], id);
                                receipt
                            }
                            _ => panic!("unexpected recovery request {body}"),
                        };
                        Json(json!({"jsonrpc":"2.0","id":body["id"],"result":result}))
                    },
                ),
            )
            .with_state(mock);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let root = format!("http://{}", listener.local_addr().unwrap());
    (
        root,
        Server(tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap()
        })),
    )
}

#[tokio::test]
async fn stock_recovery_four_single_leg_failures_cap_cancel_restart_no_resend_and_native_settlement(
) {
    for (sell, chain_failed, cex_rejected) in [
        (false, true, false),
        (false, false, true),
        (true, true, false),
        (true, false, true),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("plans.jsonl");
        let now = common::time::now_ms();
        let (mut service, original) = fixture(path.clone(), now, false, sell);
        let mock = Mock {
            plan: original.clone(),
            path: path.clone(),
            posts: Default::default(),
            chain_posts: Default::default(),
            reads: Default::default(),
            missing_fee: Default::default(),
            chain_failed,
            cex_rejected,
            rfq_client: 0,
        };
        let (root, _server) = server(mock.clone()).await;
        service.root = root.clone();
        let hub = realtime::WsHub::new(16);
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let http = client.clone();
        let url = format!("{root}/execute");
        let sent = service
            .dispatch_stock_pair(&original.plan_id, &hub, signed, move |cost, sig| {
                Box::pin(async move { chain::submit_with(&http, &url, None, cost, sig).await })
            })
            .await
            .unwrap();
        assert!(service
            .prepare_recovery(
                StockRecoveryBuildRequest {
                    plan_id: sent.plan_id.clone(),
                    revision: sent.revision,
                    max_loss_usdc: "5".into()
                },
                &hub
            )
            .await
            .is_err());
        let found = chain::lookup_with(
            &client,
            &format!("{root}/rpc"),
            &sent.terms.chain_cost,
            sent.chain_submission.as_ref().unwrap(),
        )
        .await;
        service.finish_chain_recheck(&sent.plan_id, found).unwrap();
        if !cex_rejected {
            service
                .reconcile_stock_order(
                    &sent.plan_id,
                    &(service.credential_loader)().unwrap(),
                    false,
                )
                .await
                .unwrap();
        }
        let id = sent.plan_id.clone();
        let mut current = service.plan_store.get(&id).unwrap();
        assert_eq!(
            current.accounting().status,
            StockAccountingStatus::NeedsReview
        );
        let row = quote(&current, 2);
        assert_eq!(
            row.target.direction,
            if sell == chain_failed {
                StockChainDirection::Sell
            } else {
                StockChainDirection::Buy
            }
        );
        for change in [0, 1, 2, 3, 4, 5] {
            let mut bad = row.clone();
            match change {
                0 => bad.max_loss_usdc = "-1".into(),
                1 => bad.wallet.sol_lamports = Some("0".into()),
                2 => bad.cost.simulation_slot = Some(12),
                3 => bad.cost.mint.ui_multiplier = "99".into(),
                4 => bad.minimum_net_usdc = "999".into(),
                _ => bad.target.stock_raw = "1".into(),
            }
            let before = std::fs::read(&path).unwrap();
            assert!(
                service.plan_store.prepare_recovery(&id, bad).is_err(),
                "accepted unsafe case {change}"
            );
            assert_eq!(std::fs::read(&path).unwrap(), before);
        }
        if row.minimum_net_usdc.starts_with('-') {
            let mut bad = row.clone();
            bad.max_loss_usdc = "0".into();
            assert!(service.plan_store.prepare_recovery(&id, bad).is_err());
        }
        current = service
            .plan_store
            .prepare_recovery(&id, row.clone())
            .unwrap();
        assert!(service
            .plan_store
            .prepare_recovery(&id, quote(&current, 3))
            .is_err());
        let cancel = StockRecoveryActionRequest {
            plan_id: id.clone(),
            revision: current.revision,
            index: 0,
        };
        service.cancel_recovery(cancel.clone(), &hub).unwrap();
        service.cancel_recovery(cancel, &hub).unwrap();
        current = service.plan_store.get(&id).unwrap();
        assert!(current.holds_funds(now + 100000));
        let row = quote(&current, 3);
        current = service.plan_store.prepare_recovery(&id, row).unwrap();
        let service = Arc::new(service);
        assert!(service
            .send_recovery_with(
                &id,
                0,
                &hub,
                |_| panic!("cancelled signing"),
                |_, _| panic!("cancelled send")
            )
            .await
            .is_err());
        let cost = current.recoveries[1].cost.clone();
        let fail_first = !sell && chain_failed;
        let posts = Arc::new(AtomicUsize::new(0));
        let (recovery_root, _recovery_server) = recovery_server(RecoveryMock {
            cost: cost.clone(),
            path: path.clone(),
            posts: posts.clone(),
            failed: fail_first,
        })
        .await;
        let request = StockPlanExecutionRequest {
            plan_id: id.clone(),
            revision: current.revision,
            action: StockExecutionAction::Recovery { index: 1 },
            confirm_live: true,
        };
        let http = client.clone();
        let url = format!("{recovery_root}/execute");
        service
            .execute_owned(request.clone(), hub.clone(), move |s, r, h| async move {
                s.send_recovery_with(&r.plan_id, 1, &h, signed, move |c, sig| {
                    Box::pin(async move { chain::submit_with(&http, &url, None, c, sig).await })
                })
                .await
            })
            .await
            .unwrap();
        assert_eq!(posts.load(Ordering::SeqCst), 1);
        let unknown = service.plan_store.get(&id).unwrap();
        assert!(unknown.recovery_target().is_err());
        assert!(!unknown.accounting().can_settle());
        drop(service);
        let (restored, _) = BackpackStocks::stock_plan_fixture(path.clone(), now);
        let service = Arc::new(restored);
        assert!(
            service.plan_store.problem().is_none(),
            "{:?}",
            service.plan_store.problem()
        );
        service
            .execute_owned(request, hub.clone(), |_, _, _| async {
                panic!("restart must not resubmit")
            })
            .await
            .unwrap();
        let found = chain::lookup_with(
            &client,
            &format!("{recovery_root}/rpc"),
            &cost,
            unknown.recoveries[1].submission.as_ref().unwrap(),
        )
        .await;
        current = service.finish_recovery_recheck(&id, 1, found).unwrap();
        if fail_first {
            assert_eq!(
                current.accounting().status,
                StockAccountingStatus::NeedsReview
            );
            assert_eq!(
                current.accounting().net_sol_change.as_deref(),
                Some("-0.000007")
            );
            let retry = quote(&current, 4);
            current = service.plan_store.prepare_recovery(&id, retry).unwrap();
            let retry_cost = current.recoveries[2].cost.clone();
            let (retry_root, _retry_server) = recovery_server(RecoveryMock {
                cost: retry_cost.clone(),
                path: path.clone(),
                posts: posts.clone(),
                failed: false,
            })
            .await;
            let http = client.clone();
            let url = format!("{retry_root}/execute");
            current = service
                .send_recovery_with(&id, 2, &hub, signed, move |c, sig| {
                    Box::pin(async move { chain::submit_with(&http, &url, None, c, sig).await })
                })
                .await
                .unwrap();
            let found = chain::lookup_with(
                &client,
                &format!("{retry_root}/rpc"),
                &retry_cost,
                current.recoveries[2].submission.as_ref().unwrap(),
            )
            .await;
            current = service.finish_recovery_recheck(&id, 2, found).unwrap();
        }
        let report = current.accounting();
        assert_eq!(
            report.status,
            StockAccountingStatus::LegsReconciled,
            "{report:?}"
        );
        assert!(
            current.recovery_target().is_err(),
            "successful rounding surplus must not start another recovery"
        );
        assert_eq!(
            report.net_sol_change.as_deref(),
            Some(if fail_first { "-0.000014" } else { "-0.000007" })
        );
        assert!(!report.can_settle());
        let at = common::time::now_ms();
        let row = StockNativeTopup {
            source_revision: current.revision,
            prepared_at_ms: at,
            valuation: valuation(&cost, if fail_first { 14000 } else { 7000 }, 5000, at, 20),
            wallet: StockWalletEvidence {
                checked_at_ms: at,
                ..current.recoveries[1].wallet.clone()
            },
            submission: None,
        };
        current = service.plan_store.prepare_topup(&id, row).unwrap();
        let topup = settlement::native_cost(&current, &current.native_topups[0]).unwrap();
        let (top_root, _top_server) = recovery_server(RecoveryMock {
            cost: topup.clone(),
            path: path.clone(),
            posts: posts.clone(),
            failed: false,
        })
        .await;
        let http = client.clone();
        let url = format!("{top_root}/execute");
        current = service
            .send_topup_with(&id, 0, &hub, signed, move |c, sig| {
                Box::pin(async move { chain::submit_with(&http, &url, None, c, sig).await })
            })
            .await
            .unwrap();
        let found = chain::lookup_with(
            &client,
            &format!("{top_root}/rpc"),
            &topup,
            current.native_topups[0].submission.as_ref().unwrap(),
        )
        .await;
        current = service.finish_topup_recheck(&id, 0, found).unwrap();
        assert!(
            current.accounting().can_settle(),
            "{:?}",
            current.accounting()
        );
        service
            .settle_plan(
                StockPlanRevisionRequest {
                    plan_id: id.clone(),
                    revision: current.revision,
                },
                &hub,
            )
            .unwrap();
        let done = service.plan_store.get(&id).unwrap();
        assert!(!done.holds_funds(now + 100000));
        assert_eq!(posts.load(Ordering::SeqCst), if fail_first { 3 } else { 2 });
        assert_eq!(mock.posts.load(Ordering::SeqCst), 1);
        assert_eq!(mock.chain_posts.load(Ordering::SeqCst), 1);
        drop(service);
        let loaded = BackpackStocks::new().unwrap().with_plan_store(path);
        assert_eq!(loaded.plan_store.get(&id).unwrap(), done);
        assert!(loaded.plan_store.problem().is_none());
    }
}
