use super::*;
use axum::{
    extract::Query,
    http::HeaderMap,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicBool, AtomicUsize},
};

const TOKEN: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
const TOKEN_2022: &str = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb";
#[path = "creation_tests.rs"]
mod creation_tests;
mod reconciliation;
mod pagination;
fn assert_account_held(claims: &crate::services::onchain_wallet_claims::WalletClaims, now: i64) {
    use crate::services::onchain_wallet_claims::{Hold, Module, Owner};
    let account = Hold { wallets: Default::default(), expires_at_ms: None }
        .with_account("backpack_stocks", "configured-account").unwrap();
    let error = claims.commit(Owner::new(Module::Stocks, "other-plan-account-probe"),
        Some(account), now, ||Ok(())).unwrap_err();
    assert!(error.contains("股票补库计划"), "{error}");
}
fn address(seed: u8) -> String {
    bs58::encode([seed; 32]).into_string()
}
fn risk() -> trading::RiskConfig {
    trading::RiskConfig {
        live_trading_enabled: true,
        ..Default::default()
    }
}

fn fixture(now: i64, asset: &str) -> StockFundingPlan {
    let (mut request, mut snapshot, mut account, mut wallet) = funding_plan::tests::inputs(now);
    request.target = StockFundingTarget::Backpack;
    request.funding_asset = asset.into();
    wallet.stock_raw = Some("1000000".into());
    wallet.usdc_raw = Some("25000000".into());
    snapshot.tokens[0].minimum_deposit = Some("0.0006".into());
    if asset == "USDC" {
        request.direction = StockChainDirection::Sell;
    }
    if asset == "SOL" {
        account.balances.insert(
            "SOL".into(),
            StockAccountBalance {
                available: "0".into(),
                locked: "0".into(),
                staked: "0".into(),
                observed_at_ms: now,
                source_at_us: None,
            },
        );
        snapshot.preflight.as_mut().unwrap().directions[0]
            .inventory
            .push(StockInventoryRequirement {
                location: "Backpack".into(),
                asset: "SOL".into(),
                required: Some("0.01".into()),
                available: Some("0".into()),
                sufficient: Some(false),
            });
    } else {
        account.balances.get_mut(asset).unwrap().available = "0".into();
    }
    funding_plan::prepare(
        request,
        &snapshot,
        &account,
        &wallet,
        &snapshot.preflight.as_ref().unwrap().directions,
        None,
        Some(StockDepositAddress {
            asset: "MU.US".into(),
            address: address(9),
            blockchain: "Solana".into(),
            account_fingerprint: account.fingerprint.clone(),
            checked_at_ms: now,
        }),
        now,
    )
    .unwrap()
}

struct Mock {
    plan: StockFundingPlan,
    path: std::path::PathBuf,
    bad: Mutex<String>,
    encoded: Mutex<String>,
    history: Mutex<Value>,
    history_requests: Mutex<Vec<BTreeMap<String, String>>>,
    fail_history_offset: Mutex<Option<usize>>,
    finalized: AtomicBool,
    failed: AtomicBool,
    create_destination: AtomicBool,
    destination_ready: AtomicBool,
    hold_send: AtomicBool,
    sends: AtomicUsize,
    queries: AtomicUsize,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
impl Mock {
    fn result(&self, b: &Value) -> Value {
        let p = &self.plan;
        let now = common::time::now_ms();
        let minimum = p.terms.mint.slot + 10;
        let bad = self.bad.lock().clone();
        match b["method"].as_str().unwrap() {
            "getGenesisHash" => json!(if bad == "genesis" {
                "wrong-genesis"
            } else {
                "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d"
            }),
            "getMultipleAccounts" => {
                assert_eq!(b["params"][0][0], p.terms.mint.address);
                json!({"context":{"slot":minimum},"value":[
                    {"owner":TOKEN_2022,"executable":false,"data":{"parsed":{"type":"mint","info":{"isInitialized":true,"decimals":6,"extensions":[
                        {"extension":"scaledUiAmountConfig","state":{"multiplier":p.terms.mint.ui_multiplier,"newMultiplier":if bad=="multiplier"{"2"}else{&p.terms.mint.ui_multiplier},"newMultiplierEffectiveTimestamp":0}},
                        {"extension":"pausableConfig","state":{"paused":bad=="paused"}},
                        {"extension":"tokenMetadata","state":{"mint":p.terms.mint.address}}
                    ]}}}},
                    {"owner":TOKEN,"executable":false,"data":{"parsed":{"type":"mint","info":{"isInitialized":true,"decimals":6}}}},
                    {"owner":"Sysvar1111111111111111111111111111111111111","data":{"parsed":{"type":"clock","info":{"unixTimestamp":now/1000}}},"executable":false}
                ]})
            }
            "getTokenAccountsByOwner" => {
                let source = b["params"][0].as_str() == Some(&p.request.wallet_address);
                let program = if p.request.funding_asset == "USDC" {
                    TOKEN
                } else {
                    TOKEN_2022
                };
                let target = if self.create_destination.load(Ordering::SeqCst)
                    && bad != "destination_frozen"
                {
                    self.ata()
                } else {
                    address(6)
                };
                let row = json!({"pubkey":if source{address(5)}else{target},"account":{"owner":program,"executable":false,"data":{"parsed":{"type":"account","info":{
                    "owner":if bad=="owner"{json!(address(4))}else{b["params"][0].clone()},"mint":p.terms.token.contract_address,
                    "state":if bad=="frozen" || bad=="destination_frozen" && !source{"frozen"}else{"initialized"},
                    "tokenAmount":{"decimals":if bad=="decimals"{9}else{6},"amount":if source{"1000000000"}else{"0"}},
                    "extensions":if bad=="extension"{json!([{"extension":"memoTransfer","state":{"requireIncomingTransferMemos":true}}])}else{json!([])}
                }}}}});
                json!({"context":{"slot":minimum},"value":if bad=="duplicate"{json!([row.clone(),row])}else if !source && bad!="destination_frozen" && (bad=="missing_destination" || self.create_destination.load(Ordering::SeqCst) && !self.destination_ready.load(Ordering::SeqCst)){json!([])}else{json!([row])}})
            }
            "getAccountInfo" => {
                assert_eq!(b["params"][0], self.ata());
                let value = if bad == "creation_owner" {
                    json!({"owner":address(4),"lamports":1,"executable":false,"data":["","base64"]})
                } else if self.destination_ready.load(Ordering::SeqCst) {
                    self.result(
                        &json!({"method":"getTokenAccountsByOwner","params":[p.terms.destination]}),
                    )["value"][0]["account"]
                        .clone()
                } else if bad == "prefunded" {
                    json!({"owner":"11111111111111111111111111111111","lamports":10000,"executable":false,"data":["","base64"]})
                } else {
                    Value::Null
                };
                json!({"context":{"slot":minimum},"value":value})
            }
            "getLatestBlockhash" => {
                json!({"context":{"slot":minimum},"value":{"blockhash":address(8),"lastValidBlockHeight":400}})
            }
            "getBlockHeight" => json!(if bad == "height" { 401 } else { 390 }),
            "getFeeForMessage" => {
                assert_eq!(b["params"][1]["commitment"], "confirmed");
                json!({"context":{"slot":minimum},"value":if bad=="fee"{6000}else{5000}})
            }
            "getMinimumBalanceForRentExemption" => json!(match b["params"][0].as_u64().unwrap() {
                0 => 890880,
                165 => self.rent(),
                174 => self.rent(),
                _ => panic!("unverified size"),
            }),
            "getBalance" => {
                json!({"context":{"slot":minimum},"value":if bad=="balance"{0}else{1_000_000_000u64}})
            }
            "simulateTransaction" => {
                assert_eq!(b["params"][1]["sigVerify"], false);
                assert_eq!(b["params"][1]["replaceRecentBlockhash"], false);
                let raw = STANDARD.decode(b["params"][0].as_str().unwrap()).unwrap();
                assert!(raw[1..65].iter().all(|v| *v == 0));
                let size_query = raw.ends_with(&[21, 7, 0]);
                let program = if p.request.funding_asset == "USDC" {
                    TOKEN
                } else {
                    TOKEN_2022
                };
                let size: u64 = if bad == "account_size" {
                    169
                } else if program == TOKEN {
                    165
                } else {
                    174
                };
                json!({"context":{"slot":minimum},"value":{"err":if bad=="simulate"{json!({"InstructionError":[0,"InvalidArgument"]})}else{Value::Null},
                    "returnData":if size_query && bad!="missing_size"{json!({"programId":if bad=="size_program"{TOKEN}else{program},"data":[STANDARD.encode(size.to_le_bytes()),"base64"]})}else{Value::Null}}})
            }
            "getSignatureStatuses" => {
                json!({"value":[if self.finalized.load(Ordering::SeqCst){json!({"slot":minimum+1,"confirmationStatus":"finalized","err":self.error()})}else{Value::Null}]})
            }
            "getTransaction" => {
                assert_eq!(b["params"][1]["encoding"], "base64");
                assert_eq!(b["params"][1]["commitment"], "finalized");
                let native = p.request.funding_asset == "SOL";
                let create = STANDARD.decode(self.encoded.lock().as_str()).unwrap()[68] == 8;
                let vacant = create && !self.destination_ready.load(Ordering::SeqCst);
                let success = !self.failed.load(Ordering::SeqCst);
                let raw = if success {
                    chain::amount(p).unwrap()
                } else {
                    0
                };
                let target = if bad == "receipt_amount" {
                    raw + 1
                } else {
                    raw
                };
                let fee = if bad == "receipt_fee" { 6000u64 } else { 5000 };
                let prefund = if vacant {
                    if bad == "prefunded" {
                        10000
                    } else {
                        0
                    }
                } else {
                    2039280
                };
                let creation_cost = if vacant && success {
                    self.rent() - prefund + if bad == "receipt_rent" { 1000 } else { 0 }
                } else {
                    0
                };
                let pre = if native {
                    json!([1_000_000_000u64, 1000000, 1])
                } else if create {
                    json!([1_000_000_000u64, 2039280, prefund, 1, 1, 1, 1, 1])
                } else {
                    json!([1_000_000_000u64, 2039280, 2039280, 1, 1])
                };
                let post = if native {
                    json!([1_000_000_000u64 - raw - fee, 1000000 + target, 1])
                } else if create {
                    json!([
                        1_000_000_000u64 - fee - creation_cost,
                        2039280,
                        prefund + creation_cost,
                        1,
                        1,
                        1,
                        1,
                        1
                    ])
                } else {
                    json!([1_000_000_000u64 - fee, 2039280, 2039280, 1, 1])
                };
                let token = |i: u8, n: u64| {
                    json!({"accountIndex":i,"owner":if bad=="receipt_owner"{address(4)}else if i==1{p.request.wallet_address.clone()}else{p.terms.destination.clone()},"mint":p.terms.token.contract_address,
                    "uiTokenAmount":{"decimals":6,"amount":n.to_string()}})
                };
                let encoded = if bad == "receipt_hash" {
                    STANDARD.encode(vec![1; 300])
                } else {
                    self.encoded.lock().clone()
                };
                let mut init = vec![18];
                init.extend_from_slice(&bs58::decode(&p.terms.destination).into_vec().unwrap());
                json!({"slot":minimum+1,"blockTime":now/1000,"transaction":[encoded,"base64"],"meta":{"err":self.error(),"fee":fee,
                    "preBalances":pre,"postBalances":post,"preTokenBalances":if native{json!([])}else if vacant{json!([token(1,1_000_000_000)])}else{json!([token(1,1_000_000_000),token(2,0)])},
                    "postTokenBalances":if native{json!([])}else if vacant && !success{json!([token(1,1_000_000_000)])}else{json!([token(1,1_000_000_000-raw),token(2,target)])},
                    "innerInstructions":if vacant && success && bad!="missing_init"{json!([{"index":0,"instructions":[{"programIdIndex":4,"accounts":[2,3],"data":bs58::encode(init).into_string(),"stackHeight":2}]}])}else{json!([])},"loadedAddresses":{"readonly":[],"writable":[]}}})
            }
            other => panic!("Unexpected RPC {other}"),
        }
    }
    fn error(&self) -> Value {
        if self.failed.load(Ordering::SeqCst) {
            json!({"InstructionError":[0,"InsufficientFunds"]})
        } else {
            Value::Null
        }
    }
    fn ata(&self) -> String {
        let key = |s: &str| {
            solana_pubkey::Pubkey::new_from_array(
                bs58::decode(s).into_vec().unwrap().try_into().unwrap(),
            )
        };
        let p = &self.plan;
        let program = key(if p.request.funding_asset == "USDC" {
            TOKEN
        } else {
            TOKEN_2022
        });
        solana_pubkey::Pubkey::find_program_address(
            &[
                key(&p.terms.destination).as_ref(),
                program.as_ref(),
                key(p.terms.token.contract_address.as_deref().unwrap()).as_ref(),
            ],
            &key("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL"),
        )
        .0
        .to_string()
    }
    fn rent(&self) -> u64 {
        let n = if self.plan.request.funding_asset == "USDC" {
            2_039_280
        } else {
            2_101_920
        };
        n + if *self.bad.lock() == "rent_increased" {
            1000
        } else {
            0
        }
    }
}
struct Server(tokio::task::JoinHandle<()>);
impl Drop for Server {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn server(plan: StockFundingPlan, path: std::path::PathBuf) -> (Arc<Mock>, String, Server) {
    let state = Arc::new(Mock {
        plan,
        path,
        bad: Mutex::new(String::new()),
        encoded: Mutex::new(String::new()),
        history: Mutex::new(json!([])),
        history_requests: Mutex::new(Vec::new()),
        fail_history_offset: Mutex::new(None),
        finalized: AtomicBool::new(false),
        failed: AtomicBool::new(false),
        create_destination: AtomicBool::new(false),
        destination_ready: AtomicBool::new(false),
        hold_send: AtomicBool::new(false),
        sends: AtomicUsize::new(0),
        queries: AtomicUsize::new(0),
        entered: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    });
    let rpc_state = state.clone();
    let history = state.clone();
    let router=Router::new().route("/rpc",post(move|Json(body):Json<Value>|{let s=rpc_state.clone();async move{
        if body["method"]=="sendTransaction"{
            assert_eq!(body["params"][1]["maxRetries"],0);assert_eq!(body["params"][1]["skipPreflight"],false);
            let encoded=body["params"][0].as_str().unwrap().to_owned();*s.encoded.lock()=encoded;
            let lines=std::fs::read_to_string(&s.path).unwrap();let last:Value=serde_json::from_str(lines.lines().last().unwrap()).unwrap();
            assert_eq!(last["plan"]["phase"],"transferring");assert!(last["plan"]["transfer"]["transactionHash"].is_string());
            s.sends.fetch_add(1,Ordering::SeqCst);s.entered.notify_one();
            if s.hold_send.load(Ordering::SeqCst){s.release.notified().await;}
            return Json(json!({"jsonrpc":"2.0","id":body["id"],"error":{"code":-32000,"message":"lost upstream reply"}}));
        }
        Json(json!({"jsonrpc":"2.0","id":body["id"],"result":s.result(&body)}))
    }})).route("/wapi/v1/capital/deposits",get(move|headers:HeaderMap,Query(params):Query<BTreeMap<String,String>>|{let s=history.clone();async move{
        assert_eq!(params.len(),5);assert_eq!(params["excludePlatform"],"true");assert_eq!(params["limit"],"100");
        let offset:usize=params["offset"].parse().unwrap();assert_eq!(offset%100,0);
        s.history_requests.lock().push(params.clone());
        rfq_tests::signed(&headers,"depositQueryAll",params);s.queries.fetch_add(1,Ordering::SeqCst);
        if *s.fail_history_offset.lock()==Some(offset){return (axum::http::StatusCode::SERVICE_UNAVAILABLE,Json(json!({"error":"temporary local history failure"}))).into_response();}
        let history=s.history.lock();Json(match history.as_array(){Some(rows)=>json!(rows.iter().skip(offset).take(100).collect::<Vec<_>>()),None=>history.clone()}).into_response()
    }}));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let root = format!("http://{}", listener.local_addr().unwrap());
    (
        state,
        root,
        Server(tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap()
        })),
    )
}
fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap()
}
fn sign(p: &StockFundingTransferPreparation) -> String {
    let mut raw = STANDARD.decode(&p.transaction_base64).unwrap();
    let signature = common::signing::ed25519_sign_bytes(&[7; 32], &raw[65..]).unwrap();
    raw[1..65].copy_from_slice(&signature);
    STANDARD.encode(raw)
}
fn remote(plan: &StockFundingPlan, status: &str) -> Value {
    json!({"id":17,"source":"solana","status":status,"symbol":plan.request.funding_asset,"quantity":plan.terms.quantity,
        "createdAt":chrono::DateTime::from_timestamp_millis(plan.transfer.as_ref().unwrap().submitted_at_ms.unwrap()).unwrap().to_rfc3339(),
        "transactionHash":plan.transfer.as_ref().unwrap().transaction_hash,"toAddress":plan.terms.destination,"fromAddress":plan.request.wallet_address})
}

#[tokio::test]
async fn stock_funding_transfer_exact_stock_usdc_sol_and_rpc_rejections() {
    for asset in ["MU.US", "USDC", "SOL"] {
        let now = common::time::now_ms();
        let plan = fixture(now, asset);
        let tmp = tempfile::tempdir().unwrap();
        let (mock, root, _server) = server(plan.clone(), tmp.path().join("funding.jsonl")).await;
        let rpc = format!("{root}/rpc");
        let client = client();
        let prepared = chain::prepare_with(&client, &rpc, &plan, 890880)
            .await
            .unwrap();
        assert_eq!(prepared.network_fee_lamports, 5000);
        assert_eq!(prepared.retained_sol_lamports, 890880);
        chain::check_with(&client, &rpc, &plan, &prepared)
            .await
            .unwrap();
        assert_eq!(mock.sends.load(Ordering::SeqCst), 0);
        assert_eq!(mock.queries.load(Ordering::SeqCst), 0);
        if asset == "MU.US" {
            assert_eq!(plan.terms.quantity, "0.02");
            assert_eq!(chain::amount(&plan).unwrap(), 16000);
            for bad in [
                "genesis",
                "owner",
                "decimals",
                "frozen",
                "balance",
                "paused",
                "multiplier",
                "duplicate",
                "extension",
                "simulate",
            ] {
                *mock.bad.lock() = bad.into();
                assert!(
                    chain::prepare_with(&client, &rpc, &plan, 890880)
                        .await
                        .is_err(),
                    "case {bad}"
                );
            }
            for bad in ["height", "fee", "owner", "frozen", "missing_destination"] {
                *mock.bad.lock() = bad.into();
                assert!(
                    chain::check_with(&client, &rpc, &plan, &prepared)
                        .await
                        .is_err(),
                    "case {bad}"
                );
            }
        }
        *mock.bad.lock() = String::new();
        let claims = Arc::new(crate::services::onchain_wallet_claims::WalletClaims::default());
        let store = funding_store::FundingStore::load(Some(mock.path.clone()), claims.clone());
        store.insert(plan.clone(), now).unwrap();
        let t = StockFundingTransfer {
            deposit_scan: None,
            evidence_conflict: None,
            preparation: prepared,
            submitted_at_ms: None,
            transaction_hash: None,
            acknowledged: false,
            query_count: 0,
            last_query_at_ms: None,
            receipt: None,
            deposit: None,
            problem: None,
        };
        let ready = store
            .update_transfer(&plan, t, common::time::now_ms())
            .unwrap();
        for case in 0..4 {
            let mut bad = ready.clone();
            let p = &mut bad.transfer.as_mut().unwrap().preparation;
            match case {
                0 => p.transaction_base64 = "invalid".into(),
                1 => p.destination_token_account = Some(address(4)),
                2 => p.blockhash = address(4),
                _ => p.prepared_at_ms = bad.terms.valid_until_ms,
            }
            assert!(
                funding_plan::validate(&bad).is_err(),
                "case {case}, asset {asset}"
            );
        }
        let cancelled = store
            .cancel(
                &StockPlanRevisionRequest {
                    plan_id: ready.plan_id.clone(),
                    revision: ready.revision,
                },
                common::time::now_ms(),
            )
            .unwrap();
        assert!(!cancelled.phase.holds_funds());
        assert!(claims
            .check(
                "solana",
                &plan.request.wallet_address,
                common::time::now_ms()
            )
            .is_ok());
        drop(store);
        let restored = funding_store::FundingStore::load(Some(mock.path.clone()), claims);
        assert_eq!(restored.get(&plan.plan_id).unwrap(), cancelled);
        assert!(restored.problem().is_none());
    }
}

#[tokio::test]
async fn stock_funding_transfer_disconnect_restart_receipt_deposit_and_single_broadcast() {
    transfer_roundtrip(false).await;
}

#[tokio::test]
async fn stock_funding_transfer_creation_disconnect_restart_deposit_and_single_broadcast() {
    transfer_roundtrip(true).await;
}

async fn transfer_roundtrip(create_destination: bool) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("funding.jsonl");
    let now = common::time::now_ms();
    let plan = fixture(now, "MU.US");
    let (mock, root, _server) = server(plan.clone(), path.clone()).await;
    mock.create_destination
        .store(create_destination, Ordering::SeqCst);
    let rpc = format!("{root}/rpc");
    let mut service = BackpackStocks::stock_plan_fixture(dir.path().join("plans.jsonl"), now)
        .0
        .with_funding_store(path.clone());
    service.root = root.clone();
    service.funding_store.insert(plan.clone(), now).unwrap();
    let service = Arc::new(service);
    let hub = realtime::WsHub::new(32);
    let mut frames = hub.subscribe(realtime::channels::STOCKS);
    let endpoint = rpc.clone();
    service.prepare_funding_transfer_with(StockPlanRevisionRequest{plan_id:plan.plan_id.clone(),revision:1},hub.clone(),move|_,p,_|async move{chain::prepare_with(&client(),&endpoint,&p,890880).await}).await.unwrap();
    let ready = service.funding_store.get(&plan.plan_id).unwrap();
    assert_eq!(
        ready
            .transfer
            .as_ref()
            .unwrap()
            .preparation
            .account_creation
            .is_some(),
        create_destination
    );
    assert_eq!(ready.revision, 2);
    let frame = frames.recv().await.unwrap();
    assert!(
        frame.payload_json().unwrap()["fundingPlans"][0]["transfer"]["preparation"].is_object()
    );
    let request = StockFundingSubmitRequest {
        plan_id: ready.plan_id.clone(),
        revision: ready.revision,
        confirm_live: true,
        two_factor_token: None,
    };
    for case in 0..5 {
        let mut request = request.clone();
        if case == 0 {
            request.confirm_live = false;
        }
        if case == 3 {
            request.revision = 1;
        }
        if case == 4 {
            request.two_factor_token = Some("not-for-chain".into());
        }
        assert!(service
            .submit_funding_transfer_with(
                request,
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
                |_, _, _| async {
                    panic!("invalid request must not reach signer");
                    #[allow(unreachable_code)]
                    Ok((client(), String::new(), String::new()))
                }
            )
            .await
            .is_err());
    }
    mock.hold_send.store(true, Ordering::SeqCst);
    let endpoint = rpc.clone();
    let s = service.clone();
    let h = hub.clone();
    let req = request.clone();
    let caller = tokio::spawn(async move {
        s.submit_funding_transfer_with(req, h, false, risk, move |_, p, _| async move {
            let c = client();
            let prepared = &p.transfer.as_ref().unwrap().preparation;
            chain::check_with(&c, &endpoint, &p, prepared).await?;
            Ok((c, endpoint, sign(prepared)))
        })
        .await
    });
    tokio::time::timeout(Duration::from_secs(4), mock.entered.notified())
        .await
        .unwrap();
    caller.abort();
    mock.release.notify_one();
    let guard = tokio::time::timeout(Duration::from_secs(4), service.submission_lock.lock())
        .await
        .unwrap();
    drop(guard);
    let original = service.funding_store.get(&plan.plan_id).unwrap();
    assert_eq!(original.phase, StockFundingPlanPhase::Transferring);
    assert_eq!(mock.sends.load(Ordering::SeqCst), 1);
    assert_eq!(mock.queries.load(Ordering::SeqCst), 0);
    assert!(service
        .funding_store
        .cancel(
            &StockPlanRevisionRequest {
                plan_id: plan.plan_id.clone(),
                revision: original.revision
            },
            common::time::now_ms()
        )
        .is_err());
    assert!(service
        .wallet_claims
        .check("solana", &plan.request.wallet_address, now + 86_400_000)
        .is_err());
    service
        .submit_funding_transfer_with(request.clone(), hub.clone(), false, risk, |_, _, _| async {
            panic!("must not sign twice");
            #[allow(unreachable_code)]
            Ok((client(), String::new(), String::new()))
        })
        .await
        .unwrap();
    drop(service);
    let mut restored = BackpackStocks::stock_plan_fixture(dir.path().join("plans.jsonl"), now)
        .0
        .with_funding_store(path);
    restored.root = root;
    let restored = Arc::new(restored);
    assert_eq!(restored.funding_store.get(&plan.plan_id).unwrap(), original);
    restored
        .submit_funding_transfer_with(request, hub.clone(), false, risk, |_, _, _| async {
            panic!("restart must not sign again");
            #[allow(unreachable_code)]
            Ok((client(), String::new(), String::new()))
        })
        .await
        .unwrap();
    mock.finalized.store(true, Ordering::SeqCst);
    for bad in ["receipt_owner", "receipt_hash"] {
        *mock.bad.lock() = bad.into();
        assert!(
            chain::receipt_with(&client(), &rpc, &original)
                .await
                .is_err(),
            "{bad}"
        );
    }
    for bad in ["receipt_amount", "receipt_fee"] {
        *mock.bad.lock() = bad.into();
        let r = chain::receipt_with(&client(), &rpc, &original)
            .await
            .unwrap();
        assert!(!r.within_plan);
        if bad == "receipt_fee" {
            assert_eq!(r.network_fee_lamports, 6000);
        }
    }
    *mock.bad.lock() = String::new();
    *mock.history.lock() = json!([remote(&original, "pending")]);
    // A persisted cooldown also applies after restart.
    restored
        .recheck_funding_transfer_with(&original, &rfq_tests::keys().unwrap(), |_| async {
            panic!("cooldown must not query RPC");
            #[allow(unreachable_code)]
            Err(String::new())
        })
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(5010)).await;
    let endpoint = rpc.clone();
    restored
        .recheck_funding_transfer_with(
            &original,
            &rfq_tests::keys().unwrap(),
            move |p| async move { chain::receipt_with(&client(), &endpoint, &p).await },
        )
        .await
        .unwrap();
    let pending = restored.funding_store.get(&plan.plan_id).unwrap();
    assert_eq!(pending.phase, StockFundingPlanPhase::DepositPending);
    assert_eq!(
        pending
            .transfer
            .as_ref()
            .unwrap()
            .receipt
            .as_ref()
            .unwrap()
            .source_debit_raw,
        16000
    );
    assert!(restored
        .wallet_claims
        .check("solana", &plan.request.wallet_address, now + 86_400_000)
        .is_err());
    *mock.history.lock() = json!([remote(&pending, "confirmed")]);
    restored.resume_funding(hub.clone());
    restored.resume_funding(hub.clone());
    tokio::time::timeout(Duration::from_secs(8),async {loop {
        if restored.funding_store.get(&plan.plan_id).unwrap().phase==StockFundingPlanPhase::Deposited {break;}
        tokio::time::sleep(Duration::from_millis(10)).await;
    }}).await.unwrap();
    let deposited = restored.funding_store.get(&plan.plan_id).unwrap();
    assert_eq!(deposited.phase, StockFundingPlanPhase::Deposited);
    assert_eq!(deposited.followup.as_ref().unwrap().attempts,1);
    assert!(deposited.funding_followup_at().is_none());
    if !create_destination {
        if let Ok(path)=std::env::var("STOCK_FUNDING_FOLLOWUP_DEPOSIT_CAPTURE_PATH") {
            std::fs::write(path,serde_json::to_vec_pretty(&restored.snapshot()).unwrap()).unwrap();
        }
    }
    assert_eq!(
        deposited
            .transfer
            .as_ref()
            .unwrap()
            .receipt
            .as_ref()
            .unwrap()
            .account_creation_lamports,
        if create_destination { 2_101_920 } else { 0 }
    );
    assert!(
        restored.account.read().evidence.is_none(),
        "old pre-transfer balance cannot fund a new plan"
    );
    assert!(
        restored.snapshot().preflight.is_none(),
        "a new inventory check is required after funding"
    );
    assert!(!deposited.phase.holds_funds());
    assert!(restored
        .wallet_claims
        .check(
            "solana",
            &plan.request.wallet_address,
            common::time::now_ms()
        )
        .is_ok());
    assert_eq!(mock.sends.load(Ordering::SeqCst), 1);
    assert_eq!(mock.queries.load(Ordering::SeqCst), 2);
    for bad in ["hash", "network", "asset", "address", "duplicate"] {
        let mut row = remote(&pending, "confirmed");
        match bad {
            "hash" => row["transactionHash"] = json!(address(5)),
            "network" => row["source"] = json!("ethereum"),
            "asset" => row["symbol"] = json!("USDC"),
            "address" => row["toAddress"] = json!(address(1)),
            _ => {}
        }
        let rows = if bad == "duplicate" {
            json!([row.clone(), row])
        } else {
            json!([row])
        };
        let result = deposit_from_rows(&deposited, &rows);
        if bad == "hash" {
            assert!(result.unwrap().is_none());
        } else {
            assert!(result.is_err(), "{bad}");
        }
    }
    let mut missing = remote(&pending, "confirmed");
    missing.as_object_mut().unwrap().remove("fromAddress");
    missing.as_object_mut().unwrap().remove("toAddress");
    assert!(
        deposit_from_rows(&deposited, &json!([missing]))
            .unwrap()
            .is_some(),
        "optional addresses are independently proven by the original signed chain message"
    );
}

#[tokio::test]
async fn stock_funding_transfer_failed_chain_preserves_actual_fee_and_cannot_be_restarted() {
    for (asset, create_destination) in [
        ("MU.US", false),
        ("SOL", false),
        ("MU.US", true),
        ("USDC", true),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("funding.jsonl");
        let now = common::time::now_ms();
        let p = fixture(now, asset);
        let (mock, root, _server) = server(p.clone(), path.clone()).await;
        mock.create_destination
            .store(create_destination, Ordering::SeqCst);
        let rpc = format!("{root}/rpc");
        let prepared = chain::prepare_with(&client(), &rpc, &p, 890880)
            .await
            .unwrap();
        let signed = sign(&prepared);
        let hash = chain::signed_identity(&p, &prepared, &signed).unwrap();
        *mock.encoded.lock() = signed;
        mock.finalized.store(true, Ordering::SeqCst);
        mock.failed.store(true, Ordering::SeqCst);
        let claims = Arc::new(crate::services::onchain_wallet_claims::WalletClaims::default());
        let store = funding_store::FundingStore::load(Some(path.clone()), claims.clone());
        store.insert(p.clone(), now).unwrap();
        let ready = store
            .update_transfer(
                &p,
                StockFundingTransfer {
                    deposit_scan: None,
                    evidence_conflict: None,
                    preparation: prepared,
                    submitted_at_ms: None,
                    transaction_hash: None,
                    acknowledged: false,
                    query_count: 0,
                    last_query_at_ms: None,
                    receipt: None,
                    deposit: None,
                    problem: None,
                },
                common::time::now_ms(),
            )
            .unwrap();
        let mut t = ready.transfer.clone().unwrap();
        t.submitted_at_ms = Some(common::time::now_ms());
        t.transaction_hash = Some(hash);
        let begun = store
            .update_transfer(&ready, t.clone(), t.submitted_at_ms.unwrap())
            .unwrap();
        let r = chain::receipt_with(&client(), &rpc, &begun).await.unwrap();
        assert!(!r.succeeded && r.within_plan);
        assert_eq!(r.source_debit_raw, 0);
        assert_eq!(r.destination_credit_raw, 0);
        assert_eq!(r.network_fee_lamports, 5000);
        assert_eq!(
            r.account_creation_lamports, 0,
            "failed atomic transfer does not retain rent"
        );
        assert_eq!(r.wallet_debit_lamports, 5000);
        t.receipt = Some(r);
        let failed = store
            .update_transfer(&begun, t, common::time::now_ms())
            .unwrap();
        assert_eq!(failed.phase, StockFundingPlanPhase::TransferFailed);
        assert!(claims
            .check("solana", &p.request.wallet_address, common::time::now_ms())
            .is_ok());
        assert!(store
            .update_transfer(&failed, ready.transfer.unwrap(), common::time::now_ms())
            .is_err());
        drop(store);
        let restored = funding_store::FundingStore::load(Some(path), claims);
        assert_eq!(restored.get(&p.plan_id).unwrap(), failed);
        assert!(restored.problem().is_none());
        assert_eq!(mock.sends.load(Ordering::SeqCst), 0);
        assert_eq!(mock.queries.load(Ordering::SeqCst), 0);
    }
}
