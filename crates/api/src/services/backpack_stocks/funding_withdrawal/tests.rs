use super::*;
use crate::services::{
    onchain_comparison::read_stock_funding_receipt_with, onchain_wallet_claims::WalletClaims,
};
use axum::{extract::Query, http::HeaderMap, routing::get, Json, Router};
use std::{collections::BTreeMap, sync::atomic::AtomicUsize};

mod reconciliation;

struct Server(tokio::task::JoinHandle<()>);
impl Drop for Server {
    fn drop(&mut self) {
        self.0.abort();
    }
}

fn risk() -> trading::RiskConfig {
    trading::RiskConfig {
        live_trading_enabled: true,
        ..Default::default()
    }
}
fn intent(plan: &StockFundingPlan, now: i64) -> StockFundingWithdrawal {
    StockFundingWithdrawal {
        client_id: client_id(plan),
        submitted_at_ms: now,
        query_count: 0,
        last_query_at_ms: None,
        remote: None,
        receipt: None,
        problem: Some("未确认".into()),
        evidence_conflict: None,
    }
}
fn remote(plan: &StockFundingPlan) -> Value {
    json!({"id":43,"clientId":client_id(plan),"blockchain":"Solana","symbol":plan.request.funding_asset,
        "quantity":plan.terms.quantity,"toAddress":plan.terms.destination,"fee":"0.5","status":"confirmed",
        "transactionHash":bs58::encode([7;64]).into_string(),"isInternal":false,
        "createdAt":chrono::DateTime::from_timestamp_millis(plan.terms.created_at_ms).unwrap().to_rfc3339()})
}
fn transaction(plan: &StockFundingPlan) -> Value {
    let signature = bs58::encode([7; 64]).into_string();
    let payer = bs58::encode([6; 32]).into_string();
    let ata = bs58::encode([8; 32]).into_string();
    let balance = |raw: &str| {
        json!({"accountIndex":1,"mint":plan.terms.token.contract_address,
        "owner":plan.terms.destination,"uiTokenAmount":{"amount":raw,"decimals":6,"uiAmount":null}})
    };
    json!({"slot":plan.terms.mint.slot+1,"blockTime":plan.terms.created_at_ms/1000,
        "transaction":{"signatures":[signature],"message":{"accountKeys":[{"pubkey":payer},{"pubkey":ata}],"instructions":[]}},
        "meta":{"err":null,"fee":5000,"preBalances":[100000,2039280],"postBalances":[95000,2039280],
            "preTokenBalances":[balance("100")],"postTokenBalances":[balance("10000100")],"innerInstructions":[]}})
}

#[test]
fn stock_funding_withdrawal_journal_retains_unknown_and_rejects_cancel_expiry_or_identity_rewrite()
{
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("funding.jsonl");
    let claims = Arc::new(WalletClaims::default());
    let store = funding_store::FundingStore::load(Some(path.clone()), claims.clone());
    let p = funding_plan::tests::fixture(10_000);
    store.insert(p.clone(), 10_000).unwrap();
    let mut late = intent(&p, p.terms.valid_until_ms);
    assert!(store
        .update(&p, late.clone(), p.terms.valid_until_ms)
        .is_err());
    late.submitted_at_ms = 10_001;
    let begun = store.update(&p, late, 10_001).unwrap();
    assert!(store.update(&p, intent(&p, 10_001), 10_001).is_err());
    assert!(store
        .cancel(
            &StockPlanRevisionRequest {
                plan_id: p.plan_id.clone(),
                revision: begun.revision
            },
            10_002
        )
        .is_err());
    assert_eq!(
        begun.phase_at(1_000_000),
        StockFundingPlanPhase::Withdrawing
    );
    assert!(claims
        .check("solana", &p.request.wallet_address, 1_000_000)
        .is_err());
    for case in 0..5 {
        let mut bad = begun.clone();
        bad.revision += 1;
        bad.updated_at_ms += 1;
        match case {
            0 => {
                bad.withdrawal = None;
                bad.phase = StockFundingPlanPhase::Cancelled;
            }
            1 => bad.withdrawal.as_mut().unwrap().submitted_at_ms += 1,
            2 => bad.withdrawal.as_mut().unwrap().client_id = "changed".into(),
            3 => bad.withdrawal.as_mut().unwrap().query_count = 5,
            _ => bad.terms.quantity = "11".into(),
        }
        assert!(transition(&begun, &bad).is_err(), "case {case}");
    }
    drop(store);
    drop(claims);
    let claims = Arc::new(WalletClaims::default());
    let store = funding_store::FundingStore::load(Some(path), claims.clone());
    assert!(store.problem().is_none());
    assert_eq!(store.get(&p.plan_id).unwrap(), begun);
    assert!(claims
        .check("solana", &p.request.wallet_address, 1_000_000)
        .is_err());
}

#[test]
fn stock_funding_withdrawal_protocol_rejects_mismatched_money_and_preserves_unknown_fee() {
    let mut p = funding_plan::tests::fixture(10_000);
    p.withdrawal = Some(intent(&p, 10_000));
    p.phase = StockFundingPlanPhase::Withdrawing;
    let value = remote(&p);
    assert_eq!(
        parse(&p, value.clone()).unwrap().fee.as_deref(),
        Some("0.5")
    );
    let mut unknown = value.clone();
    unknown.as_object_mut().unwrap().remove("fee");
    assert_eq!(parse(&p, unknown).unwrap().fee, None);
    for (key, value) in [
        ("clientId", json!("other")),
        ("symbol", json!("USDT")),
        ("quantity", json!("100")),
        ("toAddress", json!(bs58::encode([5; 32]).into_string())),
        ("blockchain", json!("Ethereum")),
        ("fee", json!("-1")),
        ("createdAt", json!("1970-01-01T00:00:00Z")),
        ("transactionHash", json!("unknown")),
    ] {
        let mut bad = remote(&p);
        bad[key] = value;
        assert!(parse(&p, bad).is_err(), "{key}");
    }
    assert!(payload(&p, Some("bad\ntoken")).is_err());
    assert!(payload(&p, None).unwrap().get("twoFactorToken").is_none());
}

#[tokio::test]
async fn stock_funding_withdrawal_single_post_lost_reply_restart_query_rpc_receipt_and_ws() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("funding.jsonl");
    let now = common::time::now_ms();
    let plan = funding_plan::tests::fixture(now);
    let mut service = BackpackStocks::stock_plan_fixture(dir.path().join("plans.jsonl"), now)
        .0
        .with_funding_store(path.clone());
    service.funding_store.insert(plan.clone(), now).unwrap();
    let post_calls = Arc::new(AtomicUsize::new(0));
    let get_calls = Arc::new(AtomicUsize::new(0));
    let history = Arc::new(Mutex::new(json!([])));
    let tx = Arc::new(Mutex::new(transaction(&plan)));
    let genesis = Arc::new(Mutex::new(json!("5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d")));
    let gate = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let posts = post_calls.clone();
    let gets = get_calls.clone();
    let reply = history.clone();
    let saved_path = path.clone();
    let server_gate = gate.clone();
    let server_release = release.clone();
    let expected = plan.clone();
    let rpc_tx = tx.clone();
    let rpc_genesis = genesis.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let root = format!("http://{}", listener.local_addr().unwrap());
    service.root = root.clone();
    let router=Router::new().route(PATH,get(move |headers:HeaderMap,Query(q):Query<BTreeMap<String,String>>|{
        let gets=gets.clone();let reply=reply.clone();async move {
            assert_eq!(q.get("limit").map(String::as_str),Some("2"));assert_eq!(q.len(),2);
            assert!(q["clientId"].starts_with("bp-stock-funding-"));rfq_tests::signed(&headers,"withdrawalQueryAll",q);
            gets.fetch_add(1,Ordering::SeqCst);Json(reply.lock().clone())
        }
    }).post(move |headers:HeaderMap,Json(body):Json<Value>|{
        let posts=posts.clone();let path=saved_path.clone();let gate=server_gate.clone();let release=server_release.clone();let expected=expected.clone();
        async move {
            posts.fetch_add(1,Ordering::SeqCst);
            assert_eq!(body,payload(&expected,Some("issued-local-2fa-token")).unwrap());
            let params=body.as_object().unwrap().iter().map(|(k,v)|(k.clone(),v.as_str().map(str::to_owned).unwrap_or_else(||v.to_string()))).collect();
            rfq_tests::signed(&headers,"withdraw",params);
            let log=std::fs::read_to_string(path).unwrap();assert!(log.contains("withdrawing"));
            assert!(!log.contains("issued-local-2fa-token") && !log.contains("twoFactorToken"));
            gate.notify_one();release.notified().await;
            "incomplete upstream response"
        }
    })).route("/rpc",axum::routing::post(move |Json(body):Json<Value>|{
        let tx=rpc_tx.clone();let genesis=rpc_genesis.clone();async move {
            let tx=tx.lock().clone();let result=match body["method"].as_str().unwrap(){
                "getGenesisHash"=>genesis.lock().clone(),
                "getSignatureStatuses"=>{assert_eq!(body["params"][1]["searchTransactionHistory"],true);json!({"value":[{"slot":tx["slot"],"err":null,"confirmationStatus":"finalized"}]})},
                "getTransaction"=>{assert_eq!(body["params"][1]["commitment"],"finalized");assert_eq!(body["params"][1]["encoding"],"jsonParsed");tx},
                other=>panic!("unexpected RPC {other}"),
            };Json(json!({"jsonrpc":"2.0","id":body["id"],"result":result}))
        }
    }));
    let _server = Server(tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap()
    }));
    let service = Arc::new(service);
    let hub = realtime::WsHub::new(32);
    let mut frames = hub.subscribe(realtime::channels::STOCKS);
    let request = StockFundingSubmitRequest {
        plan_id: plan.plan_id.clone(),
        revision: 1,
        confirm_live: true,
        two_factor_token: Some("issued-local-2fa-token".into()),
    };
    for case in 0..3 {
        let mut r = request.clone();
        if case == 0 {
            r.confirm_live = false;
        }
        let result = service
            .submit_funding_with(
                r,
                hub.clone(),
                false,
                move || {
                    let mut r = risk();
                    if case == 1 {
                        r.live_trading_enabled = false;
                    }
                    if case == 2 {
                        r.kill_switch_active = true;
                    }
                    r
                },
                |_, _, _| async { panic!("must not refresh") },
            )
            .await;
        assert!(result.is_err());
    }
    assert_eq!(post_calls.load(Ordering::SeqCst), 0);
    let (r, s, a, mut wallet) = funding_plan::tests::inputs(now);
    wallet.usdc_raw = Some("1000000".into());
    let changed = funding_plan::prepare(
        r,
        &s,
        &a,
        &wallet,
        &s.preflight.as_ref().unwrap().directions,
        Some(StockWithdrawalCapacity {
            asset: "USDC".into(),
            quantity: "25".into(),
            checked_at_ms: now,
        }),
        None,
        now,
    )
    .unwrap();
    assert!(service
        .submit_funding_with(
            request.clone(),
            hub.clone(),
            false,
            risk,
            move |_, _, _| async move { Ok(changed) }
        )
        .await
        .unwrap_err()
        .contains("已变化"));
    let switched = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let switch = switched.clone();
    assert!(service
        .submit_funding_with(
            request.clone(),
            hub.clone(),
            false,
            move || {
                let mut r = risk();
                r.kill_switch_active = switched.load(Ordering::SeqCst);
                r
            },
            move |_, p, _| async move {
                switch.store(true, Ordering::SeqCst);
                Ok(p)
            }
        )
        .await
        .unwrap_err()
        .contains("急停"));
    assert_eq!(post_calls.load(Ordering::SeqCst), 0);
    assert!(service
        .funding_store
        .get(&plan.plan_id)
        .unwrap()
        .withdrawal
        .is_none());
    assert!(!format!("{request:?}").contains("issued-local-2fa-token"));
    let svc = service.clone();
    let h = hub.clone();
    let r = request.clone();
    let caller = tokio::spawn(async move {
        svc.submit_funding_with(r, h, false, risk, |_, p, _| async move { Ok(p) })
            .await
    });
    tokio::time::timeout(Duration::from_secs(3), gate.notified())
        .await
        .unwrap();
    caller.abort(); // The bounded submission owner continues after the browser disconnects.
    let _ = caller.await;
    release.notify_one();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if service.submission_lock.try_lock().is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let unresolved = service.snapshot().funding_plans[0].clone();
    assert_eq!(unresolved.phase, StockFundingPlanPhase::Withdrawing);
    assert!(unresolved
        .withdrawal
        .as_ref()
        .unwrap()
        .problem
        .as_ref()
        .unwrap()
        .contains("空历史"));
    assert_eq!(post_calls.load(Ordering::SeqCst), 1);
    assert_eq!(get_calls.load(Ordering::SeqCst), 1);
    let bytes = std::fs::read(&path).unwrap();
    service
        .submit_funding_with(request.clone(), hub.clone(), false, risk, |_, _, _| async {
            panic!("must not refresh duplicate")
        })
        .await
        .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert_eq!(post_calls.load(Ordering::SeqCst), 1);
    assert!(frames.try_recv().is_ok());
    drop(service);
    let mut restored = BackpackStocks::stock_plan_fixture(dir.path().join("plans-2.jsonl"), now)
        .0
        .with_funding_store(path.clone());
    restored.root = root.clone();
    assert_eq!(
        restored.funding_store.get(&plan.plan_id).unwrap(),
        unresolved
    );
    let restored = Arc::new(restored);
    restored
        .submit_funding_with(request, hub.clone(), false, risk, |_, _, _| async {
            panic!("restart must not submit")
        })
        .await
        .unwrap();
    assert!(restored
        .recheck_funding(
            StockPlanCancelRequest {
                plan_id: plan.plan_id.clone()
            },
            hub.clone()
        )
        .await
        .is_err());
    // No waiting for fake exchange time: exercise RPC decoding against the recorded original intent.
    let rpc = format!("{root}/rpc");
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let hash = bs58::encode([7; 64]).into_string();
    let receipt = read_stock_funding_receipt_with(&client, &rpc, &unresolved, &hash)
        .await
        .unwrap();
    assert_eq!(receipt.credited_raw, "10000000");
    assert_eq!(receipt.network_fee_lamports, 5000);
    let valid_tx = tx.lock().clone();
    for case in 0..7 {
        let mut bad = valid_tx.clone();
        match case {
            0 => bad["transaction"]["signatures"][0] = json!(bs58::encode([9; 64]).into_string()),
            1 => bad["meta"]["err"] = json!({"InstructionError":[0,"Custom"]}),
            2 => {
                bad["meta"]["postTokenBalances"][0]["owner"] =
                    json!(bs58::encode([4; 32]).into_string())
            }
            3 => {
                bad["meta"]["postTokenBalances"][0]["mint"] =
                    json!(bs58::encode([4; 32]).into_string())
            }
            4 => bad["meta"]["postTokenBalances"][0]["uiTokenAmount"]["decimals"] = json!(9),
            5 => bad["blockTime"] = json!(0),
            _ => bad["meta"]
                .as_object_mut()
                .unwrap()
                .remove("fee")
                .map(|_| ())
                .unwrap(),
        }
        *tx.lock() = bad;
        assert!(
            read_stock_funding_receipt_with(&client, &rpc, &unresolved, &hash)
                .await
                .is_err(),
            "RPC case {case}"
        );
    }
    *tx.lock() = valid_tx;
    *genesis.lock() = json!("devnet");
    assert!(
        read_stock_funding_receipt_with(&client, &rpc, &unresolved, &hash)
            .await
            .is_err()
    );
    *genesis.lock() = json!("5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d");
    *history.lock() = json!([remote(&plan)]);
    tokio::time::sleep(Duration::from_millis(QUERY_INTERVAL_MS as u64)).await;
    restored
        .recheck_funding_with(
            &plan.plan_id,
            &rfq_tests::keys().unwrap(),
            move |p, h| async move { read_stock_funding_receipt_with(&client, &rpc, &p, &h).await },
        )
        .await
        .unwrap();
    restored.publish_plan(&hub);
    let received = restored.snapshot().funding_plans[0].clone();
    assert_eq!(received.phase, StockFundingPlanPhase::Received);
    assert_eq!(
        received
            .withdrawal
            .as_ref()
            .unwrap()
            .receipt
            .as_ref()
            .unwrap()
            .credited_raw,
        "10000000"
    );
    assert!(received
        .withdrawal
        .as_ref()
        .unwrap()
        .problem
        .as_ref()
        .unwrap()
        .contains("扣账"));
    assert_eq!(post_calls.load(Ordering::SeqCst), 1);
    assert_eq!(get_calls.load(Ordering::SeqCst), 2);
    assert!(restored
        .wallet_claims
        .check("solana", &plan.request.wallet_address, now + 120_000)
        .is_err());
    drop(restored);
    let store = funding_store::FundingStore::load(Some(path), Arc::new(WalletClaims::default()));
    assert!(store.problem().is_none(), "{:?}", store.problem());
    assert_eq!(store.get(&plan.plan_id).unwrap(), received);
}
