use super::*;
use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use shared_types::stocks::*;
use tower::ServiceExt;

#[tokio::test]
async fn stock_rfq_finish_unsent_http_auth_and_late_submission_use_one_terminal_record() {
    let temp = tempfile::tempdir().unwrap();
    let mut config = AppConfig::default();
    config.history.enabled = false;
    config.storage.data_dir = temp.path().display().to_string();
    config.security.auth_token = Some("local-stock-plan-test".into());
    let mut state = AppState::new(config).await.unwrap();
    let journal = temp.path().join("stocks/rfq-history.jsonl");
    Arc::get_mut(&mut state.inner).unwrap().market.backpack_stocks = Arc::new(
        crate::services::backpack_stocks::BackpackStocks::new().unwrap().with_rfq_store(journal.clone())
    );
    let router = crate::app::build_router(state.clone());
    let path = "/api/stocks/rfq/finish-unsent";
    let body = serde_json::json!({"requestId":"stock-finish-unsent-http-001","asset":"MU.US","side":"Ask","quantity":"1.00"});
    assert_eq!(post(&router, path, body.clone(), false).await.0, StatusCode::UNAUTHORIZED);
    for field in ["confirmLive", "rfqId", "signedTransaction"] {
        let mut injected = body.clone(); injected[field] = true.into();
        assert_eq!(post(&router, path, injected, true).await.0, StatusCode::UNPROCESSABLE_ENTITY);
    }
    assert!(state.backpack_stocks().snapshot().rfqs.is_empty());
    for route in [path, path, "/api/stocks/rfq"] {
        let (status, bytes) = post(&router, route, body.clone(), true).await;
        assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&bytes));
        let snapshot: StockMarketSnapshot = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(snapshot.rfqs.len(), 1);
        assert_eq!(snapshot.rfqs[0].phase, StockRfqPhase::NotSent);
        assert_eq!(snapshot.rfqs[0].request.quantity, "1");
        assert_eq!(snapshot.rfqs[0].client_id, 0);
        assert!(!snapshot.rfq_connected);
    }
    assert_eq!(std::fs::read_to_string(journal).unwrap().lines().count(), 1);
}

#[tokio::test]
async fn stock_exchange_conversion_http_auth_and_no_injected_orders() {
    let mut config=AppConfig::default();config.history.enabled=false;config.security.auth_token=Some("local-stock-plan-test".into());
    let state=AppState::new(config).await.unwrap();let router=crate::app::build_router(state);
    for (suffix,body) in [
        ("",serde_json::json!({"requestId":"exchange-conversion-http","inputUsdt":"10","minimumUsdc":"9.98"})),
        ("/submit",serde_json::json!({"planId":"missing","revision":1,"confirmLive":true})),
        ("/cancel",serde_json::json!({"planId":"missing","revision":1})),
        ("/recheck",serde_json::json!({"planId":"missing"})),
    ] {
        let path=format!("/api/stocks/funding/exchange-conversions{suffix}");
        assert_eq!(post(&router,&path,body.clone(),false).await.0,StatusCode::UNAUTHORIZED);
        let mut invalid=body.clone();invalid["order"]=serde_json::json!({"symbol":"BTC_USDC"});
        assert_eq!(post(&router,&path,invalid,true).await.0,StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(post(&router,&path,body,true).await.0,StatusCode::CONFLICT);
    }
}

#[tokio::test]
async fn stock_peer_http_catalog_and_selection_require_auth_without_execution() {
    let temp=tempfile::tempdir().unwrap();
    let mut config=AppConfig::default();config.history.enabled=false;config.security.auth_token=Some("local-stock-plan-test".into());
    let mut state=AppState::new(config).await.unwrap();
    let (service,_)=crate::services::backpack_stocks::BackpackStocks::stock_plan_fixture(temp.path().join("plans.jsonl"),common::time::now_ms());
    let service=service.with_peer_markets(state.instrument_registry().clone(),state.market_data().clone());
    Arc::get_mut(&mut state.inner).unwrap().market.backpack_stocks=Arc::new(service);
    let router=crate::app::build_router(state);
    let preflight=serde_json::json!({"asset":"MU.US","selection":{"venue":"kraken","product":"spot","nativeSymbol":"MUx/USD"},"walletAddress":null});
    assert_eq!(post(&router,"/api/stocks/peer/preflight",preflight.clone(),false).await.0,StatusCode::UNAUTHORIZED);
    let mut injected=preflight.clone();injected["account"]=serde_json::json!({"stockAvailable":"999"});
    assert_eq!(post(&router,"/api/stocks/peer/preflight",injected,true).await.0,StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(post(&router,"/api/stocks/peer/preflight",preflight,true).await.0,StatusCode::BAD_REQUEST);
    let funding=serde_json::json!({"asset":"MU.US","selection":{"venue":"kraken","product":"spot","nativeSymbol":"MUx/USD"}});
    assert_eq!(post(&router,"/api/stocks/peer/funding",funding.clone(),false).await.0,StatusCode::UNAUTHORIZED);
    let mut injected=funding.clone();injected["methods"]=serde_json::json!([]);
    assert_eq!(post(&router,"/api/stocks/peer/funding",injected,true).await.0,StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(post(&router,"/api/stocks/peer/funding",funding,true).await.0,StatusCode::BAD_REQUEST);
    let check=serde_json::json!({"asset":"MU.US","selection":{"venue":"kraken","product":"spot","nativeSymbol":"MUx/USD"},"direction":"buy"});
    assert_eq!(post(&router,"/api/stocks/peer/order-check",check.clone(),false).await.0,StatusCode::UNAUTHORIZED);
    for field in ["validate","order","quantity"] {
        let mut injected=check.clone();injected[field]=serde_json::json!(false);
        assert_eq!(post(&router,"/api/stocks/peer/order-check",injected,true).await.0,StatusCode::UNPROCESSABLE_ENTITY);
    }
    assert_eq!(post(&router,"/api/stocks/peer/order-check",check,true).await.0,StatusCode::BAD_REQUEST);
    for authorized in [false,true] {
        let mut request=Request::builder().uri("/api/stocks/peer-markets?venue=kraken&product=spot&search=MU");
        if authorized {request=request.header("authorization","Bearer local-stock-plan-test");}
        let response=router.clone().oneshot(request.body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(),if authorized{StatusCode::OK}else{StatusCode::UNAUTHORIZED});
        if authorized {let body=to_bytes(response.into_body(),65536).await.unwrap();let c:StockPeerCatalog=serde_json::from_slice(&body).unwrap();assert!(c.rows.is_empty());assert_eq!(c.matched,0);}
    }
    let request=serde_json::json!({"asset":"MU.US","selection":null});
    assert_eq!(post(&router,"/api/stocks/peer",request.clone(),false).await.0,StatusCode::UNAUTHORIZED);
    let (status,body)=post(&router,"/api/stocks/peer",request,true).await;
    assert_eq!(status,StatusCode::OK,"{}",String::from_utf8_lossy(&body));
    let s:StockMarketSnapshot=serde_json::from_slice(&body).unwrap();assert!(s.peer.is_none() && s.plans.is_empty());
    assert!(!temp.path().join("plans.jsonl").exists());
}

async fn post(
    router: &Router,
    path: &str,
    body: serde_json::Value,
    authorized: bool,
) -> (StatusCode, Vec<u8>) {
    let mut request = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json");
    if authorized {
        request = request.header("authorization", "Bearer local-stock-plan-test")
    }
    let response = router
        .clone()
        .oneshot(
            request
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    (
        response.status(),
        to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap()
            .to_vec(),
    )
}

#[tokio::test]
async fn stock_stablecoin_http_requires_auth_and_rejects_fund_fields_before_reads() {
    let mut config=AppConfig::default();config.history.enabled=false;
    config.security.auth_token=Some("local-stock-plan-test".into());
    let state=AppState::new(config).await.unwrap();
    let router=crate::app::build_router(state.clone());
    let request=serde_json::json!({"asset":"MU.US","walletAddress":bs58::encode([3u8;32]).into_string(),
        "inputUsdt":"10","targetUsdc":"10","keyed":false});
    let path="/api/stocks/funding/stablecoin-preview";
    assert_eq!(post(&router,path,request.clone(),false).await.0,StatusCode::UNAUTHORIZED);
    // No stock is selected, so an authenticated call still cannot reach an external API.
    assert_eq!(post(&router,path,request.clone(),true).await.0,StatusCode::BAD_REQUEST);
    let mut invalid=request;invalid["confirmLive"]=true.into();
    assert_eq!(post(&router,path,invalid,true).await.0,StatusCode::UNPROCESSABLE_ENTITY);
    assert!(state.backpack_stocks().snapshot().plans.is_empty());
    assert!(state.backpack_stocks().snapshot().funding_plans.is_empty());
}

#[tokio::test]
async fn stock_stablecoin_http_save_retry_ws_and_cancel_use_one_durable_plan() {
    let temp = tempfile::tempdir().unwrap(); let path = temp.path().join("stablecoin.jsonl");
    let mut config = AppConfig::default(); config.history.enabled = false;
    config.security.auth_token = Some("local-stock-plan-test".into());
    let mut state = AppState::new(config).await.unwrap();
    let (service, request) = crate::services::backpack_stocks::BackpackStocks::stock_stablecoin_fixture(path.clone(), common::time::now_ms());
    Arc::get_mut(&mut state.inner).unwrap().market.backpack_stocks = Arc::new(service);
    let mut frames = state.ws_hub().subscribe(realtime::channels::STOCKS);
    let router = crate::app::build_router(state.clone());
    let route = "/api/stocks/funding/stablecoin-plans";
    let body = serde_json::to_value(&request).unwrap();
    assert_eq!(post(&router, route, body.clone(), false).await.0, StatusCode::UNAUTHORIZED);
    let mut injected = body.clone(); injected["confirmLive"] = true.into();
    assert_eq!(post(&router, route, injected, true).await.0, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(!path.exists());
    let (status, response) = post(&router, route, body.clone(), true).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&response));
    let s: StockMarketSnapshot = serde_json::from_slice(&response).unwrap();
    assert_eq!(s.stablecoin_plans.len(), 1);
    assert!(s.plans.is_empty() && s.funding_plans.is_empty());
    let p = &s.stablecoin_plans[0];
    assert_eq!(p.phase, shared_types::stocks::StockStablecoinPlanPhase::Reserved);
    let frame = tokio::time::timeout(Duration::from_secs(2), frames.recv()).await.unwrap().unwrap();
    assert_eq!(frame.payload_json().unwrap()["stablecoinPlans"][0]["planId"], p.plan_id);
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(post(&router, route, body, true).await.0, StatusCode::OK);
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    for auth in [false,true] {
        let mut req=Request::builder().uri(route);
        if auth {req=req.header("authorization","Bearer local-stock-plan-test");}
        let response=router.clone().oneshot(req.body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(),if auth{StatusCode::OK}else{StatusCode::UNAUTHORIZED});
        if auth {let body=to_bytes(response.into_body(),1024*1024).await.unwrap();
            assert_eq!(serde_json::from_slice::<StockMarketSnapshot>(&body).unwrap().stablecoin_plans,s.stablecoin_plans);}
    }
    let submit_route="/api/stocks/funding/stablecoin-plans/submit";
    let submit=serde_json::json!({"planId":p.plan_id,"revision":p.revision,"confirmLive":true});
    assert_eq!(post(&router,submit_route,submit.clone(),false).await.0,StatusCode::UNAUTHORIZED);
    let mut injected=submit.clone();injected["signedTransaction"]="injected".into();
    assert_eq!(post(&router,submit_route,injected,true).await.0,StatusCode::UNPROCESSABLE_ENTITY);
    let (status,body)=post(&router,submit_route,submit,true).await;
    assert_eq!(status,StatusCode::CONFLICT);assert!(String::from_utf8_lossy(&body).contains("模拟环境"));
    let recheck=serde_json::json!({"planId":p.plan_id});
    let recheck_route="/api/stocks/funding/stablecoin-plans/recheck";
    assert_eq!(post(&router,recheck_route,recheck.clone(),false).await.0,StatusCode::UNAUTHORIZED);
    assert_eq!(post(&router,recheck_route,recheck,true).await.0,StatusCode::CONFLICT);
    for (suffix,body) in [
        ("",serde_json::json!({"planId":p.plan_id,"revision":p.revision})),
        ("/submit",serde_json::json!({"planId":p.plan_id,"revision":p.revision,"index":0,"confirmLive":false})),
        ("/cancel",serde_json::json!({"planId":p.plan_id,"index":0})),
        ("/recheck",serde_json::json!({"planId":p.plan_id,"index":0})),
    ] {
        let route=format!("/api/stocks/funding/stablecoin-plans/native-topup{suffix}");
        assert_eq!(post(&router,&route,body.clone(),false).await.0,StatusCode::UNAUTHORIZED);
        let mut injected=body.clone();injected["signedTransaction"]="injected".into();
        assert_eq!(post(&router,&route,injected,true).await.0,StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(post(&router,&route,body,true).await.0,StatusCode::CONFLICT);
    }
    assert_eq!(std::fs::read(&path).unwrap(),bytes,"read/rejected routes must not sign, write or send");
    let cancel_route = "/api/stocks/funding/stablecoin-plans/cancel";
    let cancel = serde_json::json!({"planId":p.plan_id,"revision":1});
    assert_eq!(post(&router, cancel_route, cancel.clone(), false).await.0, StatusCode::UNAUTHORIZED);
    let (status, response) = post(&router, cancel_route, cancel, true).await;
    assert_eq!(status, StatusCode::OK);
    let cancelled: StockMarketSnapshot = serde_json::from_slice(&response).unwrap();
    assert_eq!(cancelled.stablecoin_plans[0].phase, shared_types::stocks::StockStablecoinPlanPhase::Cancelled);
    let frame = tokio::time::timeout(Duration::from_secs(2), frames.recv()).await.unwrap().unwrap();
    assert_eq!(frame.payload_json().unwrap()["stablecoinPlans"][0]["phase"], "cancelled");
    if let Ok(output) = std::env::var("STOCK_STABLECOIN_PLAN_CAPTURE_PATH") {
        std::fs::write(output, serde_json::to_vec(&s).unwrap()).unwrap();
    }
}

#[tokio::test]
async fn stock_plan_http_auth_reserve_retry_ws_cancel_and_restart_use_same_journal() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("plans.jsonl");
    let mut config = AppConfig::default();
    config.history.enabled = false;
    config.security.auth_token = Some("local-stock-plan-test".into());
    let mut state = AppState::new(config).await.unwrap();
    let (service, request) = crate::services::backpack_stocks::BackpackStocks::stock_plan_fixture(
        path.clone(),
        common::time::now_ms(),
    );
    Arc::get_mut(&mut state.inner)
        .unwrap()
        .market
        .backpack_stocks = Arc::new(service);
    let mut frames = state.ws_hub().subscribe(realtime::channels::STOCKS);
    let router = crate::app::build_router(state.clone());
    let request_json = serde_json::to_value(&request).unwrap();
    assert_eq!(post(&router,"/api/stocks/funding/address",serde_json::json!({"asset":"MU.US"}),false).await.0,StatusCode::UNAUTHORIZED);
    assert_eq!(post(&router,"/api/stocks/funding/address",serde_json::json!({"asset":"NOT_SELECTED.US"}),true).await.0,StatusCode::BAD_REQUEST);
    let funding=serde_json::json!({"requestId":"local-route-funding-0001","securityAsset":"MU.US","fundingAsset":"USDC","direction":"buy","target":"solana","walletAddress":request.wallet_address,"preflightAtMs":1});
    assert_eq!(post(&router,"/api/stocks/funding/plans",funding.clone(),false).await.0,StatusCode::UNAUTHORIZED);
    assert_eq!(post(&router,"/api/stocks/funding/plans",funding,true).await.0,StatusCode::CONFLICT);
    let cancel=serde_json::json!({"planId":"stock-funding-missing","revision":1});
    assert_eq!(post(&router,"/api/stocks/funding/plans/cancel",cancel.clone(),false).await.0,StatusCode::UNAUTHORIZED);
    assert_eq!(post(&router,"/api/stocks/funding/plans/cancel",cancel,true).await.0,StatusCode::CONFLICT);
    for (path,body) in [
        ("/api/stocks/funding/plans/submit",serde_json::json!({"planId":"missing","revision":1,"confirmLive":false})),
        ("/api/stocks/funding/plans/recheck",serde_json::json!({"planId":"missing"})),
        ("/api/stocks/funding/plans/prepare-transfer",serde_json::json!({"planId":"missing","revision":1})),
    ] {
        assert_eq!(post(&router,path,body.clone(),false).await.0,StatusCode::UNAUTHORIZED);
        assert_eq!(post(&router,path,body,true).await.0,StatusCode::CONFLICT);
    }
    assert_eq!(
        post(&router, "/api/stocks/plans", request_json.clone(), false)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert!(!path.exists());
    let (status, body) = post(&router, "/api/stocks/plans", request_json.clone(), true).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let snapshot: StockMarketSnapshot = serde_json::from_slice(&body).unwrap();
    let plan = snapshot.plans.first().unwrap().clone();
    assert_eq!(plan.phase, StockPlanPhase::Reserved);
    let frame = tokio::time::timeout(Duration::from_secs(2), frames.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        frame.payload_json().unwrap()["plans"][0]["planId"],
        plan.plan_id
    );
    let bytes = std::fs::read(&path).unwrap();
    let build = serde_json::json!({"requestId":request.request_id,"asset":request.asset,"direction":request.direction,
        "walletAddress":request.wallet_address,"inputRaw":plan.terms.chain_cost.quote.input_raw,"keyed":false});
    assert_eq!(post(&router,"/api/stocks/plans/build",build.clone(),false).await.0,StatusCode::UNAUTHORIZED);
    let (status,body)=post(&router,"/api/stocks/plans/build",build,true).await;
    assert_eq!(status,StatusCode::CONFLICT);
    assert!(String::from_utf8_lossy(&body).contains("不同参数"));
    assert_eq!(std::fs::read(&path).unwrap(),bytes);
    let mut execute=serde_json::json!({"planId":plan.plan_id,"revision":plan.revision,"action":{"kind":"pair"},"confirmLive":true});
    assert_eq!(post(&router,"/api/stocks/plans/execute",execute.clone(),false).await.0,StatusCode::UNAUTHORIZED);
    let (status,body)=post(&router,"/api/stocks/plans/execute",execute.clone(),true).await;
    assert_eq!(status,StatusCode::CONFLICT);
    assert!(String::from_utf8_lossy(&body).contains("模拟环境"));
    state.trading_service().update_risk_config(|r|r.live_trading_enabled=true);
    execute["confirmLive"]=false.into();
    let (status,body)=post(&router,"/api/stocks/plans/execute",execute.clone(),true).await;
    assert_eq!(status,StatusCode::CONFLICT);
    assert!(String::from_utf8_lossy(&body).contains("确认本次"));
    execute["confirmLive"]=true.into();execute["revision"]=0.into();
    let (status,body)=post(&router,"/api/stocks/plans/execute",execute.clone(),true).await;
    assert_eq!(status,StatusCode::CONFLICT);
    assert!(String::from_utf8_lossy(&body).contains("版本已变化"));
    state.trading_service().update_risk_config(|r|r.kill_switch_active=true);
    let (status,body)=post(&router,"/api/stocks/plans/execute",execute,true).await;
    assert_eq!(status,StatusCode::CONFLICT);
    assert!(String::from_utf8_lossy(&body).contains("全局急停"));
    assert_eq!(std::fs::read(&path).unwrap(),bytes);
    for path in ["/api/stocks/plans/settle","/api/stocks/plans/native-topup","/api/stocks/plans/native-topup/recheck"] {
        let request=if path.ends_with("/recheck") {serde_json::json!({"planId":plan.plan_id,"index":0})}
            else {serde_json::json!({"planId":plan.plan_id,"revision":plan.revision})};
        assert_eq!(post(&router,path,request.clone(),false).await.0,StatusCode::UNAUTHORIZED);
        assert_eq!(post(&router,path,request,true).await.0,StatusCode::CONFLICT);
    }
    assert_eq!(std::fs::read(&path).unwrap(),bytes);
    let recheck=serde_json::json!({"planId":plan.plan_id});
    for route in ["/api/stocks/plans/recovery","/api/stocks/plans/recovery/cancel","/api/stocks/plans/recovery/recheck"] {
        let body=if route.ends_with("/recovery"){serde_json::json!({"planId":plan.plan_id,"revision":plan.revision,"maxLossUsdc":"1"})}
            else{serde_json::json!({"planId":plan.plan_id,"revision":plan.revision,"index":0})};
        assert_eq!(post(&router,route,body.clone(),false).await.0,StatusCode::UNAUTHORIZED);
        assert_eq!(post(&router,route,body,true).await.0,StatusCode::CONFLICT);
    }
    assert_eq!(post(&router,"/api/stocks/plans/recheck",recheck.clone(),false).await.0,StatusCode::UNAUTHORIZED);
    assert_eq!(post(&router,"/api/stocks/plans/recheck",recheck,true).await.0,StatusCode::CONFLICT);
    assert_eq!(std::fs::read(&path).unwrap(),bytes);
    assert_eq!(
        post(&router, "/api/stocks/plans", request_json.clone(), true)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    let mut changed = request_json.clone();
    changed["direction"] = "sell".into();
    assert_eq!(
        post(&router, "/api/stocks/plans", changed, true).await.0,
        StatusCode::CONFLICT
    );
    let (status, body) = post(
        &router,
        "/api/stocks/plans/cancel",
        serde_json::json!({"planId":plan.plan_id}),
        true,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let snapshot: StockMarketSnapshot = serde_json::from_slice(&body).unwrap();
    assert_eq!(snapshot.plans[0].phase, StockPlanPhase::Cancelled);
    assert_eq!(snapshot.plans[0].revision, 2);
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(
        post(&router, "/api/stocks/plans", request_json, true)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert!(!String::from_utf8_lossy(&bytes).contains("signature"));
    drop(router);
    drop(state);
    drop(frames);
    let restored = crate::services::backpack_stocks::BackpackStocks::new()
        .unwrap()
        .with_plan_store(path);
    assert_eq!(restored.snapshot().plans, snapshot.plans);
    assert!(restored.snapshot().plan_problem.is_none());
}
