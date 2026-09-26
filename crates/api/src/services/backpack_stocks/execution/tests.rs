use super::super::rfq_tests::{params, signed as check_signature};
use super::*;
use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use chain::tests::{attach, finalized, signed};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicBool, AtomicUsize},
};

mod recovery;
mod restock;

#[derive(Clone)]
struct Mock {
    plan: StockExecutionPlan,
    path: std::path::PathBuf,
    posts: Arc<AtomicUsize>,
    chain_posts: Arc<AtomicUsize>,
    reads: Arc<AtomicUsize>,
    missing_fee: Arc<AtomicBool>,
    chain_failed: bool,
    cex_rejected: bool,
    rfq_client: u32,
}

impl Mock {
    fn intent(&self) {
        let text = std::fs::read_to_string(&self.path).unwrap();
        let row: Value = serde_json::from_str(text.lines().last().unwrap()).unwrap();
        assert!(row["plan"]["twoLegStartedAtMs"].as_i64().is_some());
        assert!(row["plan"]["chainSubmission"]["walletSignature"]
            .as_str()
            .is_some());
        assert!(!row["plan"][if self.plan.terms.rfq.is_some() {
            "rfqAcceptance"
        } else {
            "cexOrder"
        }]
        .is_null());
    }
    fn order(&self) -> Value {
        let mut v = self
            .plan
            .terms
            .cex_instruction
            .as_ref()
            .unwrap()
            .request_body();
        v["id"] = json!("original-stock-order-999");
        v["status"] = json!("Filled");
        v["executedQuantity"] = json!(self.plan.terms.cex_shares);
        v["executedQuoteQuantity"] = json!(self.plan.terms.cex_notional_usdc);
        v
    }
    fn fills(&self) -> Value {
        let order = self.order();
        json!([{"orderId":order["id"],"clientId":order["clientId"],"symbol":order["symbol"],"side":order["side"],
            "tradeId":"original-fill-999","quantity":order["quantity"],"price":order["price"],
            "fee":if self.missing_fee.load(Ordering::SeqCst) { Value::Null } else {json!(self.plan.terms.cex_fee_budget.as_ref().unwrap().additional_fee)},"feeSymbol":"USDC"}])
    }
    fn rfq(&self) -> Value {
        let r = self.plan.terms.rfq.as_ref().unwrap();
        json!({"rfqId":r.rfq_id,"clientId":self.rfq_client,"symbol":"MU.US_USDC_RFQ","side":"Ask","quantity":"0.02",
            "executionMode":"AwaitAccept","status":"Filled","createdAt":self.plan.terms.created_at_ms,
            "submissionTime":self.plan.terms.created_at_ms,"expiryTime":r.expiry_time_ms,
            "executedQuantity":"0.02","executedQuoteQuantity":"12","deferredSettlementQuoteId":r.candidate.quote_id})
    }
}

struct Server(JoinHandle<()>);
impl Drop for Server {
    fn drop(&mut self) {
        self.0.abort();
    }
}
async fn server(m: Mock) -> (String, Server) {
    let app = Router::new()
        .route("/api/v1/account/limits/withdrawal", get(|h:HeaderMap,Query(p):Query<BTreeMap<String,String>>|async move {
            check_signature(&h,"maxWithdrawalQuantity",p.clone());
            assert_eq!(p["autoBorrow"],"false");assert_eq!(p["autoLendRedeem"],"false");
            Json(json!({"symbol":p["symbol"],"autoBorrow":false,"autoLendRedeem":false,"maxWithdrawalQuantity":"100"}))
        }))
        .route("/api/v1/order", post(|State(m):State<Mock>,h:HeaderMap,Json(body):Json<Value>|async move {
            check_signature(&h,"orderExecute",params(&body)); m.intent();
            assert_eq!(body,m.plan.terms.cex_instruction.as_ref().unwrap().request_body());
            m.posts.fetch_add(1,Ordering::SeqCst);
            if m.cex_rejected { return (StatusCode::BAD_REQUEST,Json(json!({"code":"INSUFFICIENT_FUNDS","message":"local rejection"}))); }
            (StatusCode::OK,Json(json!({"truncated":"after fill"})))
        }).get(|State(m):State<Mock>,h:HeaderMap,Query(p):Query<BTreeMap<String,String>>|async move {
            check_signature(&h,"orderQuery",p); m.reads.fetch_add(1,Ordering::SeqCst); Json(m.order())
        }))
        .route("/wapi/v1/history/orders",get(|State(m):State<Mock>,h:HeaderMap,Query(p):Query<BTreeMap<String,String>>|async move {
            check_signature(&h,"orderHistoryQueryAll",p.clone()); assert_eq!(p["orderId"],"original-stock-order-999");
            m.reads.fetch_add(1,Ordering::SeqCst); Json(json!([m.order()]))
        }))
        .route("/wapi/v1/history/fills",get(|State(m):State<Mock>,h:HeaderMap,Query(p):Query<BTreeMap<String,String>>|async move {
            check_signature(&h,"fillHistoryQueryAll",p.clone()); assert_eq!(p["orderId"],"original-stock-order-999"); m.reads.fetch_add(1,Ordering::SeqCst); Json(m.fills())
        }))
        .route("/api/v1/rfq/accept",post(|State(m):State<Mock>,h:HeaderMap,Json(body):Json<Value>|async move {
            check_signature(&h,"quoteAccept",params(&body));m.intent();
            assert_eq!(body,m.plan.terms.cex_instruction.as_ref().unwrap().request_body());m.posts.fetch_add(1,Ordering::SeqCst);
            Json(json!({"truncated":"after acceptance"}))
        }))
        .route("/api/v1/rfqs",get(|State(m):State<Mock>,h:HeaderMap,Query(p):Query<BTreeMap<String,String>>|async move {
            check_signature(&h,"rfqQuery",p.clone());assert_eq!(p["rfqId"],m.plan.terms.rfq.as_ref().unwrap().rfq_id);Json(json!([]))
        }))
        .route("/wapi/v1/history/rfq",get(|State(m):State<Mock>,h:HeaderMap,Query(p):Query<BTreeMap<String,String>>|async move {
            check_signature(&h,"rfqHistoryQueryAll",p);m.reads.fetch_add(1,Ordering::SeqCst);Json(json!([m.rfq()]))
        }))
        .route("/wapi/v1/history/rfq/fill",get(|State(m):State<Mock>,h:HeaderMap,Query(p):Query<BTreeMap<String,String>>|async move {
            check_signature(&h,"rfqFillHistoryQueryAll",p);m.reads.fetch_add(1,Ordering::SeqCst);let r=m.rfq();
            Json(json!([{"rfqId":r["rfqId"],"clientId":r["clientId"],"quoteId":r["deferredSettlementQuoteId"],"symbol":r["symbol"],"side":"Ask","quantity":"0.02","fillQuantity":"0.02","fillQuoteQuantity":"12","fillPrice":"600"}]))
        }))
        .route("/execute",post(|State(m):State<Mock>,Json(body):Json<Value>|async move {
            m.intent();assert_eq!(body["signedTransaction"],signed(&m.plan.terms.chain_cost).unwrap());
            m.chain_posts.fetch_add(1,Ordering::SeqCst); "truncated-after-chain-broadcast"
        }))
        .route("/rpc",post(|State(m):State<Mock>,Json(body):Json<Value>|async move {
            let (id,receipt)=finalized(&m.plan.terms.chain_cost,m.chain_failed);
            let result=match body["method"].as_str().unwrap() {
                "getGenesisHash"=>json!("5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d"),
                "getSignaturesForAddress"=>json!([{"signature":id,"slot":13}]),
                "getTransaction"=>{assert_eq!(body["params"][0],id);receipt},
                _=>panic!("unexpected request: {body}"),
            };Json(json!({"jsonrpc":"2.0","id":body["id"],"result":result}))
        })).with_state(m);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let root = format!("http://{}", listener.local_addr().unwrap());
    (
        root,
        Server(tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap()
        })),
    )
}

fn fixture(
    path: std::path::PathBuf,
    now: i64,
    rfq: bool,
    sell: bool,
) -> (BackpackStocks, StockExecutionPlan) {
    fixture_with_costs(path, now, rfq, sell, false)
}

fn fixture_with_costs(
    path: std::path::PathBuf,
    now: i64,
    rfq: bool,
    sell: bool,
    with_costs: bool,
) -> (BackpackStocks, StockExecutionPlan) {
    let (mut s, mut request) = BackpackStocks::stock_plan_fixture(path.clone(), now);
    if rfq {
        s = s.with_rfq_store(path.with_file_name("rfq.jsonl"));
        let (mut snapshot, a, r) = plans::tests::rfq_fixture(now);
        snapshot.rfqs[0].expiry_time_ms = Some(now + 3000);
        let mut row = snapshot.rfqs[0].clone();
        let (claimed, _) = s
            .rfq_store
            .claim(
                row.request.clone(),
                &row.account_fingerprint,
                row.symbol.clone(),
                now,
            )
            .unwrap();
        row.client_id = claimed.client_id;
        snapshot.rfqs[0] = row.clone();
        s.rfq_store
            .change(&row.request.request_id, true, |r| {
                *r = row.clone();
                Ok(true)
            })
            .unwrap();
        s.rfq_subscription.send_replace(Some(a.fingerprint.clone()));
        *s.snapshot.write() = snapshot;
        *s.account.write() = account::AccountCache::default();
        s.account.write().fingerprint = a.fingerprint.clone();
        s.account.write().evidence = Some(a);
        request = r;
    }
    if sell {
        request.direction = StockChainDirection::Sell;
    }
    {
        let mut snapshot = s.snapshot.write();
        let index = usize::from(sell);
        attach(&mut snapshot.chain_costs[index], true);
        let q = snapshot.chain_costs[index].quote.clone();
        if sell {
            snapshot.comparison.as_mut().unwrap().sell = Some(q);
        } else {
            snapshot.comparison.as_mut().unwrap().buy = q;
        }
        plans::tests::refresh_report(
            &mut snapshot,
            s.account.read().evidence.as_ref().unwrap(),
            now,
        );
    }
    if with_costs {
        s = s.with_exchange_conversion_store(path.with_file_name("costs.jsonl"));
        let source = exchange_conversion::tests::completed_cost(&s, now);
        let snapshot = s.snapshot();
        request.build = Some(StockPlanBuildRequest {
            request_id: request.request_id.clone(),
            asset: request.asset.clone(),
            direction: request.direction,
            wallet_address: request.wallet_address.clone(),
            input_raw: snapshot.chain_costs[usize::from(sell)]
                .quote
                .input_raw
                .clone(),
            keyed: snapshot.comparison.as_ref().unwrap().keyed,
            conversion_cost_ids: vec![source.plan_id],
        });
    }
    s.reserve_plan(request, &realtime::WsHub::new(16)).unwrap();
    let p = s.plan_store.records().remove(0);
    (s, p)
}

#[tokio::test]
async fn stock_pair_uses_original_payload_during_monitor_refresh_and_blocks_bad_price() {
    for (rfq, sell) in [(false, false), (false, true), (true, false)] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("plans.jsonl");
        let now = common::time::now_ms();
        let (mut service, plan) = fixture(path.clone(), now, rfq, sell);
        let mock = Mock {
            plan: plan.clone(),
            path,
            posts: Default::default(),
            chain_posts: Default::default(),
            reads: Default::default(),
            missing_fee: Default::default(),
            chain_failed: false,
            cex_rejected: false,
            rfq_client: 0,
        };
        let (root, _server) = server(mock.clone()).await;
        service.root = root.clone();
        let hub = realtime::WsHub::new(16);
        let original = service.snapshot.read().clone();
        {
            let mut s = service.snapshot.write();
            if rfq {
                s.rfq_problem = Some("local stream problem".into());
                service
                    .rfq_problem
                    .write()
                    .replace("local stream problem".into());
            } else if sell {
                s.books[0].ask = Some("999".into());
            } else {
                s.books[0].bid = Some("1".into());
            }
        }
        assert!(service
            .dispatch_stock_pair(&plan.plan_id, &hub, signed, |_, _| Box::pin(async {
                panic!("bad market must not broadcast")
            }))
            .await
            .is_err());
        assert_eq!(mock.posts.load(Ordering::SeqCst), 0);
        assert_eq!(mock.chain_posts.load(Ordering::SeqCst), 0);
        assert_eq!(service.plan_store.get(&plan.plan_id).unwrap(), plan);
        *service.snapshot.write() = original;
        *service.rfq_problem.write() = None;
        {
            let mut s = service.snapshot.write();
            s.books[0].update_id += 999;
            s.comparison.as_mut().unwrap().buy.input_raw = "50000000".into();
            s.chain_costs.clear();
            s.preflight = None;
        }
        // An in-flight observation quote/preflight must not hold up a previously reserved plan.
        let _quote = service.quote_lock.lock().await;
        let _preflight = service.preflight_lock.lock().await;
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let sent = service
            .dispatch_stock_pair(&plan.plan_id, &hub, signed, move |cost, signature| {
                Box::pin(async move {
                    chain::submit_with(&client, &format!("{root}/execute"), None, cost, signature)
                        .await
                })
            })
            .await
            .unwrap();
        assert_eq!(sent.terms, plan.terms);
        assert!(sent.two_leg_started_at_ms.is_some());
        assert_eq!(mock.posts.load(Ordering::SeqCst), 1);
        assert_eq!(mock.chain_posts.load(Ordering::SeqCst), 1);
        let replay = service
            .dispatch_stock_pair(
                &plan.plan_id,
                &hub,
                |_| panic!("must not sign twice"),
                |_, _| Box::pin(async { panic!("must not resend") }),
            )
            .await
            .unwrap();
        assert_eq!(replay, sent);
    }
}

#[tokio::test]
async fn stock_pair_owned_submission_survives_http_drop_and_never_resends() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("plans.jsonl");
    let now = common::time::now_ms();
    let (mut service, plan) = fixture(path.clone(), now, false, false);
    let mock = Mock {
        plan: plan.clone(),
        path: path.clone(),
        posts: Default::default(),
        chain_posts: Default::default(),
        reads: Default::default(),
        missing_fee: Default::default(),
        chain_failed: false,
        cex_rejected: false,
        rfq_client: 0,
    };
    let (root, _server) = server(mock.clone()).await;
    service.root = root.clone();
    let service = Arc::new(service);
    let hub = realtime::WsHub::new(16);
    let request = StockPlanExecutionRequest {
        plan_id: plan.plan_id.clone(),
        revision: plan.revision,
        action: StockExecutionAction::Pair,
        confirm_live: true,
    };
    for bad in [
        StockPlanExecutionRequest {
            confirm_live: false,
            ..request.clone()
        },
        StockPlanExecutionRequest {
            revision: 0,
            ..request.clone()
        },
        StockPlanExecutionRequest {
            action: StockExecutionAction::NativeTopup { index: 99 },
            ..request.clone()
        },
    ] {
        assert!(service
            .execute_owned(bad, hub.clone(), |_, _, _| async {
                panic!("must not sign or send")
            })
            .await
            .is_err());
    }
    assert!(service
        .send_stock_pair(&plan.plan_id, hub.clone())
        .await
        .unwrap_err()
        .contains("WS 未就绪"));
    assert_eq!(mock.posts.load(Ordering::SeqCst), 0);
    let entered = Arc::new(tokio::sync::Notify::new());
    let proceed = Arc::new(tokio::sync::Notify::new());
    let started = entered.clone();
    let release = proceed.clone();
    let caller = service.clone();
    let accepted = request.clone();
    let task_hub = hub.clone();
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let task = tokio::spawn(async move {
        caller
            .execute_owned(accepted, task_hub, move |s, r, h| async move {
                s.dispatch_stock_pair(&r.plan_id, &h, signed, move |cost, signature| {
                    Box::pin(async move {
                        started.notify_one();
                        release.notified().await;
                        chain::submit_with(
                            &client,
                            &format!("{root}/execute"),
                            None,
                            cost,
                            signature,
                        )
                        .await
                    })
                })
                .await
            })
            .await
    });
    tokio::time::timeout(Duration::from_secs(3), entered.notified())
        .await
        .unwrap();
    task.abort();
    let _ = task.await;
    mock.intent();
    assert!(service
        .plan_store
        .get(&plan.plan_id)
        .unwrap()
        .holds_funds(now + 100000));
    assert_eq!(mock.chain_posts.load(Ordering::SeqCst), 0);
    proceed.notify_one();
    let guard = tokio::time::timeout(
        Duration::from_secs(5),
        service.submission_lock.clone().lock_owned(),
    )
    .await
    .unwrap();
    drop(guard);
    assert_eq!(mock.posts.load(Ordering::SeqCst), 1);
    assert_eq!(mock.chain_posts.load(Ordering::SeqCst), 1);
    let sent = service.plan_store.get(&plan.plan_id).unwrap();
    let replay = service
        .execute_owned(request.clone(), hub.clone(), |_, _, _| async {
            panic!("duplicate task")
        })
        .await
        .unwrap();
    assert_eq!(replay.plans[0], sent);
    drop(service);
    let (restored, _) = BackpackStocks::stock_plan_fixture(path, now);
    let restored = Arc::new(restored);
    assert_eq!(
        restored
            .execute_owned(request, hub, |_, _, _| async {
                panic!("restart must not send")
            })
            .await
            .unwrap()
            .plans[0],
        sent
    );
    assert_eq!(mock.posts.load(Ordering::SeqCst), 1);
    assert_eq!(mock.chain_posts.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn stock_pair_two_durable_intents_lost_replies_recover_both_directions_and_rfq_without_resend(
) {
    for (rfq, sell, chain_failed, cex_rejected) in [
        (false, false, false, false),
        (false, true, false, false),
        (true, false, false, false),
        (false, false, true, false),
        (false, false, false, true),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("plans.jsonl");
        let now = common::time::now_ms();
        let (mut service, plan) = fixture_with_costs(path.clone(), now, rfq, sell, !rfq);
        let mock = Mock {
            plan: plan.clone(),
            path: path.clone(),
            posts: Default::default(),
            chain_posts: Default::default(),
            reads: Default::default(),
            missing_fee: Arc::new(AtomicBool::new(!rfq)),
            chain_failed,
            cex_rejected,
            rfq_client: plan
                .terms
                .rfq
                .as_ref()
                .and_then(|r| service.stock_rfq(&r.request_id))
                .map(|r| r.client_id)
                .unwrap_or(0),
        };
        let (root, _server) = server(mock.clone()).await;
        service.root = root.clone();
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let post_client = client.clone();
        let url = format!("{root}/execute");
        let hub = realtime::WsHub::new(16);
        let sent = service
            .dispatch_stock_pair(&plan.plan_id, &hub, signed, move |cost, signed| {
                Box::pin(
                    async move { chain::submit_with(&post_client, &url, None, cost, signed).await },
                )
            })
            .await
            .unwrap();
        assert_eq!(mock.posts.load(Ordering::SeqCst), 1);
        assert_eq!(mock.chain_posts.load(Ordering::SeqCst), 1);
        assert!(sent.two_leg_started_at_ms.is_some() && sent.chain_submission.is_some());
        assert!(sent.holds_funds(now + 100000));
        assert!(service.cancel_plan(&plan.plan_id, &hub).is_err());
        drop(service);
        let (mut restored, _) = BackpackStocks::stock_plan_fixture(path.clone(), now);
        restored = restored.with_funding_store(path.with_file_name("funding.jsonl"));
        if !rfq {
            restored = restored.with_exchange_conversion_store(path.with_file_name("costs.jsonl"));
        }
        restored.root = root.clone();
        assert!(
            restored.plan_store.problem().is_none(),
            "{:?}",
            restored.plan_store.problem()
        );
        let duplicate = restored
            .dispatch_stock_pair(
                &plan.plan_id,
                &hub,
                |_| panic!("must not sign twice"),
                |_, _| panic!("must not broadcast twice"),
            )
            .await
            .unwrap();
        assert_eq!(duplicate, sent);
        let r = restored
            .start_chain_recheck(
                &plan.plan_id,
                sent.chain_submission.as_ref().unwrap().next_recheck_at_ms,
            )
            .unwrap();
        let lookup = chain::lookup_with(
            &client,
            &format!("{root}/rpc"),
            &r.terms.chain_cost,
            r.chain_submission.as_ref().unwrap(),
        )
        .await;
        restored
            .finish_chain_recheck(&plan.plan_id, lookup)
            .unwrap();
        let keys = (restored.credential_loader)().unwrap();
        if rfq {
            let id = &plan.terms.rfq.as_ref().unwrap().request_id;
            restored.reconcile_rfq(id, &keys).await.unwrap();
        } else if !cex_rejected {
            let _ = restored
                .reconcile_stock_order(&plan.plan_id, &keys, false)
                .await;
            assert!(restored
                .plan_store
                .get(&plan.plan_id)
                .unwrap()
                .accounting()
                .net_usdc_change
                .is_none());
            mock.missing_fee.store(false, Ordering::SeqCst);
            restored
                .reconcile_stock_order(&plan.plan_id, &keys, false)
                .await
                .unwrap();
        }
        let mut complete = restored.plan_store.get(&plan.plan_id).unwrap();
        let report = complete.accounting();
        if rfq {
            assert!(complete
                .rfq_acceptance
                .as_ref()
                .is_some_and(|r| r.phase == StockRfqPhase::Filled && !r.settlement_pending()));
            assert!(report.net_usdc_change.is_none());
            assert!(report.problems.iter().any(|p| p.contains("实际扣费")));
        } else if chain_failed || cex_rejected {
            assert_eq!(report.status, StockAccountingStatus::NeedsReview);
            assert!(report.movements.iter().any(|m| m.location == "Solana"));
        } else {
            assert_eq!(
                report.status,
                StockAccountingStatus::LegsReconciled,
                "{:?}",
                report.problems
            );
            assert_eq!(
                report.net_usdc_change.as_deref(),
                Some(if sell { "1.96798" } else { "1.988" })
            );
            assert_eq!(
                report.net_stock_shares.as_deref(),
                Some(if sell { "0" } else { "0.00375" })
            );
            assert_eq!(report.net_sol_change.as_deref(), Some("0"));
            assert_eq!(report.fee_basis_matched, Some(true));
        }
        assert!(complete.holds_funds(now + 100000));
        if !rfq {
            assert_eq!(report.conversion_fee_usdc.as_deref(), Some("0.009997"));
            if let Some(cash) = &report.net_usdc_change {
                let expected = order_protocol::decimal(cash).unwrap()
                    - plan.terms.conversion_fee_usdc().unwrap();
                assert_eq!(
                    report.after_conversion_costs_usdc.as_deref(),
                    Some(expected.normalize().to_string().as_str())
                );
            }
        }
        let restored = Arc::new(restored);
        let before_reads = mock.reads.load(Ordering::SeqCst);
        let journal = std::fs::read(&path).unwrap();
        restored
            .recheck_stock_order(&plan.plan_id, hub.clone())
            .await
            .unwrap();
        assert_eq!(
            mock.reads.load(Ordering::SeqCst),
            before_reads + if cex_rejected { 0 } else { 2 }
        );
        assert_eq!(restored.plan_store.get(&plan.plan_id).unwrap(), complete);
        assert_eq!(
            std::fs::read(&path).unwrap(),
            journal,
            "unchanged manual reads must not rewrite receipts"
        );
        let worker = restored.rfq_worker.lock().take();
        if let Some(worker) = worker {
            worker.abort();
            let _ = worker.await;
        }
        if report.can_settle() {
            restored
                .settle_plan(
                    StockPlanRevisionRequest {
                        plan_id: plan.plan_id.clone(),
                        revision: complete.revision,
                    },
                    &hub,
                )
                .unwrap();
            complete = restored.plan_store.get(&plan.plan_id).unwrap();
            assert_eq!(complete.phase, StockPlanPhase::Settled);
            assert!(!complete.holds_funds(now + 100000));
            restock::roundtrip(restored.clone(), &complete, &hub).await;
            let before_reads = mock.reads.load(Ordering::SeqCst);
            restored
                .recheck_stock_order(&plan.plan_id, hub.clone())
                .await
                .unwrap();
            assert_eq!(
                mock.reads.load(Ordering::SeqCst),
                before_reads,
                "archived plans stay read-only"
            );
        } else {
            assert!(restored
                .plan_store
                .settle(&plan.plan_id, complete.revision, common::time::now_ms())
                .is_err());
        }
        drop(restored);
        let restored = BackpackStocks::new().unwrap().with_plan_store(path);
        assert_eq!(restored.plan_store.get(&plan.plan_id).unwrap(), complete);
        assert!(restored.plan_store.problem().is_none());
        if !rfq {
            assert_eq!(
                restored
                    .plan_store
                    .claimed_conversion_cost_ids(now + 1_000_000),
                plan.conversion_cost_ids()
            );
            assert!(
                complete.claims_conversion_cost(&plan.terms.conversion_costs[0], now + 1_000_000)
            );
            if complete.phase == StockPlanPhase::Settled {
                let next_at = now + 1_000_000;
                let (mut snapshot, account, mut request) = plans::tests::fixture(next_at);
                request.request_id = "next-plan-same-cost-source".into();
                request.build = Some(StockPlanBuildRequest {
                    request_id: request.request_id.clone(),
                    asset: request.asset.clone(),
                    direction: request.direction,
                    wallet_address: request.wallet_address.clone(),
                    input_raw: snapshot.chain_costs[0].quote.input_raw.clone(),
                    keyed: snapshot.comparison.as_ref().unwrap().keyed,
                    conversion_cost_ids: plan.conversion_cost_ids(),
                });
                snapshot.exchange_conversions = plan.terms.conversion_costs.clone();
                let next = plans::prepare(request, &snapshot, &account, next_at).unwrap();
                let problem = restored.plan_store.reserve(next, next_at).unwrap_err();
                assert!(problem.contains("费用已经归入"), "{problem}");
            }
        }
        assert_eq!(mock.posts.load(Ordering::SeqCst), 1);
        assert_eq!(mock.chain_posts.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn stock_pair_fee_budget_allocation_and_legacy_identity_are_checked_before_any_send() {
    let now = common::time::now_ms();
    let temp = tempfile::tempdir().unwrap();
    let (s, p) = fixture(temp.path().join("plans.jsonl"), now, false, true);
    assert_eq!(
        p.terms.cex_fee_budget.as_ref().unwrap().net_quote_change,
        "-12.03202"
    );
    assert_eq!(p.terms.allocations[0].quantity, "12.03202");
    for mutate in [0, 1, 2] {
        let mut bad = p.clone();
        bad.request.request_id = format!("local-invalid-fee-plan-{mutate}");
        match mutate {
            0 => bad.terms.cex_fee_budget.as_mut().unwrap().additional_fee = "0".into(),
            1 => bad.terms.allocations[0].quantity = "12.02".into(),
            _ => bad.terms.cex_fee_budget.as_mut().unwrap().asset = "USD".into(),
        }
        bad.terms.cex_instruction = Some(order_compile::compile(&bad.request, &bad.terms).unwrap());
        bad.plan_id = plan_store::plan_id(&bad.request, &bad.terms).unwrap();
        assert!(s
            .plan_store
            .reserve(bad, now)
            .unwrap_err()
            .contains("费用或交易所资金预留"));
    }
    let mut old = p.clone();
    old.terms.cex_fee_budget = None;
    old.two_leg_started_at_ms = None;
    let before = serde_json::to_value(&old).unwrap();
    assert!(before["terms"].get("cexFeeBudget").is_none());
    assert!(before.get("twoLegStartedAtMs").is_none());
    let restored: StockExecutionPlan = serde_json::from_value(before).unwrap();
    assert_eq!(
        plan_store::plan_id(&old.request, &old.terms).unwrap(),
        plan_store::plan_id(&restored.request, &restored.terms).unwrap()
    );
    assert!(s
        .plan_store
        .begin_pair(
            &p.plan_id,
            &p.terms.account_fingerprint,
            &signed(&p.terms.chain_cost).unwrap(),
            None,
            p.terms.market_valid_until_ms
        )
        .is_err());
    assert!(s
        .plan_store
        .get(&p.plan_id)
        .unwrap()
        .two_leg_started_at_ms
        .is_none());
}
