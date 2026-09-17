use crate::{middleware::audit, state::AppState};
use axum::{
    extract::{State,Query},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use common::AppError;
use shared_types::stocks::*;
mod exchange_conversion;
mod peer_recovery;
mod peer_conversion;
mod peer_inventory;
mod peer_native_topup;

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .merge(exchange_conversion::router())
        .merge(peer_recovery::router())
        .merge(peer_conversion::router())
        .merge(peer_inventory::router())
        .merge(peer_native_topup::router())
        .route("/api/stocks/catalog", get(catalog))
        .route("/api/stocks/peer-markets", get(peer_markets))
        .route("/api/stocks/peer", post(peer))
        .route("/api/stocks/peer/preflight", post(peer_preflight))
        .route("/api/stocks/peer/funding", post(peer_funding))
        .route("/api/stocks/peer/order-check", post(peer_order_check))
        .route("/api/stocks/peer/plans", get(peer_plans).post(build_peer_plan))
        .route("/api/stocks/peer/plans/cancel", post(cancel_peer_plan))
        .route("/api/stocks/peer/plans/execute", post(execute_peer_plan))
        .route("/api/stocks/peer/plans/recheck", post(recheck_peer_plan))
        .route("/api/stocks/watch", post(watch))
        .route("/api/stocks/quote", post(quote))
        .route("/api/stocks/monitor", post(monitor))
        .route("/api/stocks/rfq", post(rfq))
        .route("/api/stocks/rfq/recheck", post(rfq_recheck))
        .route("/api/stocks/rfq/finish-unsent", post(rfq_finish_unsent))
        .route("/api/stocks/rfq/cancel", post(rfq_cancel))
        .route("/api/stocks/preflight", post(preflight))
        .route("/api/stocks/funding/address", post(deposit_address))
        .route("/api/stocks/funding/stablecoin-preview", post(stablecoin_preview))
        .route("/api/stocks/funding/stablecoin-plans", post(build_stablecoin_plan).get(stablecoin_plans))
        .route("/api/stocks/funding/stablecoin-plans/cancel", post(cancel_stablecoin_plan))
        .route("/api/stocks/funding/stablecoin-plans/submit", post(submit_stablecoin))
        .route("/api/stocks/funding/stablecoin-plans/recheck", post(recheck_stablecoin))
        .route("/api/stocks/funding/stablecoin-plans/native-topup", post(prepare_stablecoin_topup))
        .route("/api/stocks/funding/stablecoin-plans/native-topup/submit", post(submit_stablecoin_topup))
        .route("/api/stocks/funding/stablecoin-plans/native-topup/recheck", post(recheck_stablecoin_topup))
        .route("/api/stocks/funding/stablecoin-plans/native-topup/cancel", post(cancel_stablecoin_topup))
        .route("/api/stocks/funding/plans", post(build_funding_plan))
        .route("/api/stocks/funding/plans/cancel", post(cancel_funding_plan))
        .route("/api/stocks/funding/plans/prepare-transfer", post(prepare_funding_transfer))
        .route("/api/stocks/funding/plans/submit", post(submit_funding))
        .route("/api/stocks/funding/plans/recheck", post(recheck_funding))
        .route("/api/stocks/chain-cost", post(chain_cost))
        .route("/api/stocks/plans", post(reserve_plan))
        .route("/api/stocks/plans/build", post(build_plan))
        .route("/api/stocks/plans/execute", post(execute_plan))
        .route("/api/stocks/plans/cancel", post(cancel_plan))
        .route("/api/stocks/plans/recheck", post(recheck_plan))
        .route("/api/stocks/plans/settle", post(settle_plan))
        .route("/api/stocks/plans/native-topup", post(prepare_native_topup))
        .route("/api/stocks/plans/native-topup/recheck", post(recheck_native_topup))
        .route("/api/stocks/plans/recovery", post(prepare_recovery))
        .route("/api/stocks/plans/recovery/cancel", post(cancel_recovery))
        .route("/api/stocks/plans/recovery/recheck", post(recheck_recovery))
}

async fn peer_markets(State(state):State<AppState>,Query(request):Query<StockPeerCatalogRequest>) -> Result<Json<StockPeerCatalog>,AppError> {
    state.backpack_stocks().peer_catalog(request).map(Json)
        .map_err(|e|AppError::domain(StatusCode::BAD_REQUEST,"STOCK_PEER_CATALOG_FAILED",e))
}

async fn peer_plans(State(state): State<AppState>) -> Json<StockMarketSnapshot> {
    Json(state.backpack_stocks().snapshot())
}

async fn build_peer_plan(State(state): State<AppState>, headers: HeaderMap, Json(request): Json<StockPeerPlanRequest>) -> Result<Json<StockMarketSnapshot>, AppError> {
    let id = request.request_id.clone();
    let result = tokio::time::timeout(std::time::Duration::from_secs(40), state.backpack_stocks().build_peer_plan(request, state.ws_hub())).await
        .map_err(|_| "股票计划构建未取得完整回复；请使用原请求编号核对，不要重复建立计划".to_string()).and_then(|r|r);
    plan_result(&headers, "stock_peer.plan.reserve", &id, result)
}

async fn cancel_peer_plan(State(state): State<AppState>, headers: HeaderMap, Json(request): Json<StockPlanRevisionRequest>) -> Result<Json<StockMarketSnapshot>, AppError> {
    let id = request.plan_id.clone();
    plan_result(&headers, "stock_peer.plan.cancel", &id, state.backpack_stocks().cancel_peer_plan(request, state.ws_hub()))
}

async fn execute_peer_plan(State(state): State<AppState>, headers: HeaderMap, Json(request): Json<StockPeerExecutionRequest>) -> Result<Json<StockMarketSnapshot>, AppError> {
    let id = request.plan_id.clone();
    audit::record_http_event(&headers, "stock_peer.plan.execute", &id, "requested",
        serde_json::json!({"fundAction":true,"revision":request.revision,"confirmedLive":request.confirm_live,"automaticRetry":false}));
    let result = state.backpack_stocks().execute_peer_plan(request, state.ws_hub().clone(), state.trading_service().clone()).await;
    audit::record_http_event(&headers, "stock_peer.plan.execute", &id, if result.is_ok(){"recorded"}else{"rejected_or_unresolved"},
        serde_json::json!({"fundAction":true,"automaticRetry":false}));
    result.map(Json).map_err(|e| AppError::domain(StatusCode::CONFLICT, "STOCK_PEER_EXECUTION_NOT_CONFIRMED", e))
}

async fn recheck_peer_plan(State(state): State<AppState>, headers: HeaderMap, Json(request): Json<StockPlanRevisionRequest>) -> Result<Json<StockMarketSnapshot>, AppError> {
    let id = request.plan_id.clone();
    let result = tokio::time::timeout(std::time::Duration::from_secs(20), state.backpack_stocks().recheck_peer_plan(request, state.ws_hub())).await
        .map_err(|_| "原交易核对超时，没有重新提交或释放资金".to_string()).and_then(|r|r);
    plan_result(&headers, "stock_peer.plan.recheck", &id, result)
}

async fn stablecoin_preview(State(state):State<AppState>, headers:HeaderMap,
    Json(request):Json<StockStablecoinRequest>) -> Result<Json<StockStablecoinPreview>,AppError> {
    let asset = request.asset.clone();
    let result = tokio::time::timeout(std::time::Duration::from_secs(30),
        state.backpack_stocks().preview_stablecoin(request)).await
        .map_err(|_| "兑换试算超时，未签名或发送".to_owned()).and_then(|r| r);
    audit::record_http_event(&headers,"backpack_stock.stablecoin.preview",&asset,
        if result.is_ok(){"read"}else{"rejected"},
        serde_json::json!({"fundAction":false,"signed":false,"broadcast":false,"localReservation":false}));
    result.map(Json).map_err(|e|AppError::domain(StatusCode::BAD_REQUEST,"STOCK_STABLECOIN_PREVIEW_FAILED",e))
}

async fn build_stablecoin_plan(State(state):State<AppState>, headers:HeaderMap, Json(request):Json<StockStablecoinPlanRequest>) -> Result<Json<StockMarketSnapshot>,AppError> {
    let id = request.request_id.clone();
    plan_result(&headers, "backpack_stock.stablecoin.reserve", &id, state.backpack_stocks().build_stablecoin_plan(request, state.ws_hub()))
}

async fn stablecoin_plans(State(state):State<AppState>) -> Json<StockMarketSnapshot> {
    Json(state.backpack_stocks().snapshot())
}

async fn cancel_stablecoin_plan(State(state):State<AppState>, headers:HeaderMap, Json(request):Json<StockPlanRevisionRequest>) -> Result<Json<StockMarketSnapshot>,AppError> {
    let id = request.plan_id.clone();
    plan_result(&headers, "backpack_stock.stablecoin.cancel", &id, state.backpack_stocks().cancel_stablecoin_plan(request, state.ws_hub()))
}

async fn submit_stablecoin(State(state):State<AppState>, headers:HeaderMap, Json(request):Json<StockStablecoinSubmitRequest>) -> Result<Json<StockMarketSnapshot>,AppError> {
    let id = request.plan_id.clone();
    let context = serde_json::json!({"fundAction":true,"revision":request.revision,"confirmedLive":request.confirm_live,"automaticRetry":false});
    audit::record_http_event(&headers,"backpack_stock.stablecoin.submit",&id,"requested",context.clone());
    let result = state.backpack_stocks().submit_stablecoin(request,state.ws_hub().clone(),state.trading_service().clone()).await;
    audit::record_http_event(&headers,"backpack_stock.stablecoin.submit",&id,if result.is_ok(){"recorded"}else{"rejected_or_unresolved"},context);
    result.map(Json).map_err(|e|AppError::domain(StatusCode::CONFLICT,"STOCK_STABLECOIN_NOT_CONFIRMED",e))
}

async fn recheck_stablecoin(State(state):State<AppState>, headers:HeaderMap, Json(request):Json<StockPlanCancelRequest>) -> Result<Json<StockMarketSnapshot>,AppError> {
    let result = tokio::time::timeout(std::time::Duration::from_secs(18),
        state.backpack_stocks().recheck_stablecoin(&request.plan_id,state.ws_hub())).await
        .map_err(|_|"原兑换回执查询超时，未重发或释放占用".to_owned()).and_then(|r|r);
    plan_result(&headers,"backpack_stock.stablecoin.recheck",&request.plan_id,result)
}

async fn prepare_stablecoin_topup(State(state):State<AppState>,headers:HeaderMap,Json(request):Json<StockPlanRevisionRequest>)->Result<Json<StockMarketSnapshot>,AppError> {
    let id=request.plan_id.clone();
    let result=tokio::time::timeout(std::time::Duration::from_secs(30),state.backpack_stocks().prepare_stablecoin_topup(request,state.ws_hub())).await
        .map_err(|_|"SOL 补回试算超时，未签名或发送".to_owned()).and_then(|r|r);
    plan_result(&headers,"backpack_stock.stablecoin.topup.prepare",&id,result)
}

async fn submit_stablecoin_topup(State(state):State<AppState>,headers:HeaderMap,Json(request):Json<StockStablecoinTopupSubmitRequest>)->Result<Json<StockMarketSnapshot>,AppError> {
    let id=request.plan_id.clone();
    let context=serde_json::json!({"fundAction":true,"revision":request.revision,"index":request.index,"confirmedLive":request.confirm_live,"automaticRetry":false});
    audit::record_http_event(&headers,"backpack_stock.stablecoin.topup.submit",&id,"requested",context.clone());
    let result=state.backpack_stocks().submit_stablecoin_topup(request,state.ws_hub().clone(),state.trading_service().clone()).await;
    audit::record_http_event(&headers,"backpack_stock.stablecoin.topup.submit",&id,if result.is_ok(){"recorded"}else{"rejected_or_unresolved"},context);
    result.map(Json).map_err(|e|AppError::domain(StatusCode::CONFLICT,"STOCK_STABLECOIN_TOPUP_NOT_CONFIRMED",e))
}

async fn recheck_stablecoin_topup(State(state):State<AppState>,headers:HeaderMap,Json(request):Json<StockTopupRecheckRequest>)->Result<Json<StockMarketSnapshot>,AppError> {
    let id=request.plan_id.clone();
    let result=tokio::time::timeout(std::time::Duration::from_secs(18),state.backpack_stocks().recheck_stablecoin_topup(request,state.ws_hub())).await
        .map_err(|_|"原补回回执查询超时，未重发或释放占用".to_owned()).and_then(|r|r);
    plan_result(&headers,"backpack_stock.stablecoin.topup.recheck",&id,result)
}

async fn cancel_stablecoin_topup(State(state):State<AppState>,headers:HeaderMap,Json(request):Json<StockTopupRecheckRequest>)->Result<Json<StockMarketSnapshot>,AppError> {
    let id=request.plan_id.clone();
    plan_result(&headers,"backpack_stock.stablecoin.topup.cancel",&id,state.backpack_stocks().cancel_stablecoin_topup(request,state.ws_hub()))
}

async fn peer(State(state):State<AppState>,headers:HeaderMap,Json(request):Json<StockPeerWatchRequest>) -> Result<Json<StockMarketSnapshot>,AppError> {
    audit::record_http_event(&headers,"backpack_stock.peer.watch",&request.asset,"requested",serde_json::json!({"fundAction":false,"remoteMutation":false}));
    state.backpack_stocks().watch_peer(request,state.ws_hub()).map(Json)
        .map_err(|e|AppError::domain(StatusCode::BAD_REQUEST,"STOCK_PEER_SELECTION_FAILED",e))
}

async fn prepare_recovery(State(state):State<AppState>, headers:HeaderMap, Json(request):Json<StockRecoveryBuildRequest>) -> Result<Json<StockMarketSnapshot>,AppError> {
    let id=request.plan_id.clone();
    let result=tokio::time::timeout(std::time::Duration::from_secs(40),state.backpack_stocks().prepare_recovery(request,state.ws_hub())).await
        .map_err(|_|"补偿试算超时，没有签名或发送".to_owned()).and_then(|r|r);
    plan_result(&headers,"backpack_stock.plan.recovery.prepare",&id,result)
}

async fn cancel_recovery(State(state):State<AppState>,headers:HeaderMap,Json(request):Json<StockRecoveryActionRequest>) -> Result<Json<StockMarketSnapshot>,AppError> {
    let id=request.plan_id.clone();
    plan_result(&headers,"backpack_stock.plan.recovery.cancel",&id,state.backpack_stocks().cancel_recovery(request,state.ws_hub()))
}

async fn recheck_recovery(State(state):State<AppState>,headers:HeaderMap,Json(request):Json<StockRecoveryActionRequest>) -> Result<Json<StockMarketSnapshot>,AppError> {
    let id=request.plan_id.clone();
    let result=tokio::time::timeout(std::time::Duration::from_secs(18),state.backpack_stocks().recheck_recovery(request,state.ws_hub())).await
        .map_err(|_|"原补偿核对超时，没有重发或释放占用".to_owned()).and_then(|r|r);
    plan_result(&headers,"backpack_stock.plan.recovery.recheck",&id,result)
}

async fn execute_plan(State(state):State<AppState>, headers:HeaderMap, Json(request):Json<StockPlanExecutionRequest>) -> Result<Json<StockMarketSnapshot>,AppError> {
    let id = request.plan_id.clone();
    let action = request.action;
    let revision = request.revision;
    let confirmed = request.confirm_live;
    audit::record_http_event(&headers, "backpack_stock.plan.execute", &id, "requested",
        serde_json::json!({"fundAction":true,"action":action,"revision":revision,"confirmedLive":confirmed,"automaticRetry":false}));
    let result = state.backpack_stocks().execute_plan(request, state.ws_hub().clone(), state.trading_service().clone()).await;
    audit::record_http_event(&headers, "backpack_stock.plan.execute", &id, if result.is_ok(){"recorded"}else{"rejected_or_unresolved"},
        serde_json::json!({"fundAction":true,"action":action,"revision":revision,"confirmedLive":confirmed,"automaticRetry":false}));
    result.map(Json).map_err(|e|AppError::domain(StatusCode::CONFLICT,"STOCK_EXECUTION_NOT_CONFIRMED",e))
}

async fn settle_plan(State(state):State<AppState>, headers:HeaderMap, Json(request):Json<StockPlanRevisionRequest>) -> Result<Json<StockMarketSnapshot>,AppError> {
    let id = request.plan_id.clone();
    plan_result(&headers, "backpack_stock.plan.settle", &id, state.backpack_stocks().settle_plan(request, state.ws_hub()))
}

async fn prepare_native_topup(State(state):State<AppState>, headers:HeaderMap, Json(request):Json<StockPlanRevisionRequest>) -> Result<Json<StockMarketSnapshot>,AppError> {
    let id = request.plan_id.clone();
    let result = tokio::time::timeout(std::time::Duration::from_secs(30), state.backpack_stocks().prepare_native_topup(request, state.ws_hub())).await
        .map_err(|_| "SOL 补回试算超时，没有签名或广播".to_owned()).and_then(|r| r);
    plan_result(&headers, "backpack_stock.plan.native_topup.prepare", &id, result)
}

async fn recheck_native_topup(State(state):State<AppState>, headers:HeaderMap, Json(request):Json<StockTopupRecheckRequest>) -> Result<Json<StockMarketSnapshot>,AppError> {
    let result = tokio::time::timeout(std::time::Duration::from_secs(18), state.backpack_stocks().recheck_native_topup(&request.plan_id, request.index, state.ws_hub())).await
        .map_err(|_| "SOL 补回核对超时，没有重发或释放占用".to_owned()).and_then(|r|r);
    plan_result(&headers, "backpack_stock.plan.native_topup.recheck", &request.plan_id, result)
}

async fn recheck_plan(State(state):State<AppState>,headers:HeaderMap,Json(request):Json<StockPlanCancelRequest>)->Result<Json<StockMarketSnapshot>,AppError> {
    let result=tokio::time::timeout(std::time::Duration::from_secs(36),state.backpack_stocks().recheck_stock_order(&request.plan_id,state.ws_hub().clone())).await
        .map_err(|_|"原股票订单核对超时，未重发或释放占用".to_owned()).and_then(|r|r);
    audit::record_http_event(&headers,"backpack_stock.plan.recheck",&request.plan_id,if result.is_ok(){"read"}else{"unresolved"},
        serde_json::json!({"readOnly":true,"fundAction":false,"remoteMutation":false,"broadcast":false}));
    result.map(Json).map_err(|e|AppError::domain(StatusCode::CONFLICT,"STOCK_ORDER_RECHECK_FAILED",e))
}

async fn build_plan(State(state):State<AppState>,headers:HeaderMap,Json(request):Json<StockPlanBuildRequest>)->Result<Json<StockMarketSnapshot>,AppError> {
    let id = request.request_id.clone();
    let result = tokio::time::timeout(std::time::Duration::from_secs(36), state.backpack_stocks().build_plan(request, state.ws_hub())).await
        .map_err(|_|"计划构建超时；重试将核对同一请求，没有提交订单或广播".to_owned()).and_then(|r|r);
    plan_result(&headers, "backpack_stock.plan.build", &id, result)
}

async fn reserve_plan(State(state):State<AppState>,headers:HeaderMap,Json(request):Json<StockPlanRequest>)->Result<Json<StockMarketSnapshot>,AppError> {
    let id=request.request_id.clone();
    let result=state.backpack_stocks().reserve_plan(request,state.ws_hub());
    plan_result(&headers,"backpack_stock.plan.reserve",&id,result)
}
async fn cancel_plan(State(state):State<AppState>,headers:HeaderMap,Json(request):Json<StockPlanCancelRequest>)->Result<Json<StockMarketSnapshot>,AppError> {
    let result=state.backpack_stocks().cancel_plan(&request.plan_id,state.ws_hub());
    plan_result(&headers,"backpack_stock.plan.cancel",&request.plan_id,result)
}
fn plan_result(headers:&HeaderMap,operation:&'static str,id:&str,result:Result<StockMarketSnapshot,String>)->Result<Json<StockMarketSnapshot>,AppError> {
    audit::record_http_event(headers,operation,id,if result.is_ok(){"recorded"}else{"rejected"},
        serde_json::json!({"localReservation":true,"fundAction":false,"remoteMutation":false,"broadcast":false}));
    result.map(Json).map_err(|e|AppError::domain(StatusCode::CONFLICT,"STOCK_PLAN_REJECTED",e))
}

async fn chain_cost(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<StockChainCostRequest>,
) -> Result<Json<StockMarketSnapshot>, AppError> {
    let asset = request.asset.clone();
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(18),
        state
            .backpack_stocks()
            .chain_cost(request, state.ws_hub().clone()),
    )
    .await;
    audit::record_http_event(
        &headers,
        "backpack_stock.chain_cost",
        &asset,
        if matches!(&result, Ok(Ok(_))) {
            "simulated"
        } else {
            "rejected"
        },
        serde_json::json!({"readOnly":true,"signed":false,"broadcast":false,"fundAction":false}),
    );
    result
        .map_err(|_| {
            AppError::domain(
                StatusCode::GATEWAY_TIMEOUT,
                "STOCK_CHAIN_COST_TIMEOUT",
                "链上费用试算超时，没有签名或广播",
            )
        })?
        .map(Json)
        .map_err(|e| AppError::domain(StatusCode::BAD_REQUEST, "STOCK_CHAIN_COST_FAILED", e))
}

async fn deposit_address(State(state):State<AppState>,headers:HeaderMap,Json(request):Json<StockDepositAddressRequest>) -> Result<Json<StockMarketSnapshot>,AppError> {
    let asset=request.asset.clone();
    let result=state.backpack_stocks().read_deposit_address(request,state.ws_hub()).await;
    audit::record_http_event(&headers,"backpack_stock.funding.address",&asset,if result.is_ok(){"read"}else{"rejected"},
        serde_json::json!({"readOnly":true,"fundAction":false}));
    result.map(Json).map_err(|e|AppError::domain(StatusCode::BAD_REQUEST,"STOCK_DEPOSIT_ADDRESS_FAILED",e))
}

async fn build_funding_plan(State(state):State<AppState>,headers:HeaderMap,Json(request):Json<StockFundingPlanRequest>)->Result<Json<StockMarketSnapshot>,AppError> {
    let id=request.request_id.clone();
    let result=tokio::time::timeout(std::time::Duration::from_secs(30),state.backpack_stocks().build_funding_plan(request,state.ws_hub())).await
        .map_err(|_|"补库准备超时，未转账；请使用原请求核对是否已保存".to_owned()).and_then(|r|r);
    plan_result(&headers,"backpack_stock.funding.prepare",&id,result)
}

async fn cancel_funding_plan(State(state):State<AppState>,headers:HeaderMap,Json(request):Json<StockPlanRevisionRequest>)->Result<Json<StockMarketSnapshot>,AppError> {
    let id=request.plan_id.clone();
    plan_result(&headers,"backpack_stock.funding.cancel",&id,state.backpack_stocks().cancel_funding_plan(request,state.ws_hub()))
}

async fn submit_funding(State(state):State<AppState>, headers:HeaderMap, Json(request):Json<StockFundingSubmitRequest>) -> Result<Json<StockMarketSnapshot>,AppError> {
    let id=request.plan_id.clone();
    let context=serde_json::json!({"fundAction":true,"revision":request.revision,"confirmedLive":request.confirm_live,"automaticRetry":false});
    audit::record_http_event(&headers,"backpack_stock.funding.submit",&id,"requested",context.clone());
    let result=state.backpack_stocks().submit_funding(request,state.ws_hub().clone(),state.trading_service().clone()).await;
    audit::record_http_event(&headers,"backpack_stock.funding.submit",&id,if result.is_ok(){"recorded"}else{"rejected_or_unresolved"},context);
    result.map(Json).map_err(|e|AppError::domain(StatusCode::CONFLICT,"STOCK_FUNDING_NOT_CONFIRMED",e))
}

async fn prepare_funding_transfer(State(state):State<AppState>,headers:HeaderMap,Json(request):Json<StockPlanRevisionRequest>)->Result<Json<StockMarketSnapshot>,AppError>{
    let id=request.plan_id.clone();
    let result=tokio::time::timeout(std::time::Duration::from_secs(30),state.backpack_stocks().prepare_funding_transfer(request,state.ws_hub().clone())).await
        .map_err(|_|"补库转账核算超时，未签名或广播；请查看原计划是否已保存费用".to_owned()).and_then(|r|r);
    plan_result(&headers,"backpack_stock.funding.prepare_transfer",&id,result)
}

async fn recheck_funding(State(state):State<AppState>,headers:HeaderMap,Json(request):Json<StockPlanCancelRequest>)->Result<Json<StockMarketSnapshot>,AppError> {
    let id=request.plan_id.clone();
    let result=state.backpack_stocks().recheck_funding(request,state.ws_hub().clone()).await;
    audit::record_http_event(&headers,"backpack_stock.funding.recheck",&id,if result.is_ok(){"read"}else{"unresolved"},
        serde_json::json!({"readOnly":true,"fundAction":false,"remoteMutation":false}));
    result.map(Json).map_err(|e|AppError::domain(StatusCode::CONFLICT,"STOCK_FUNDING_RECHECK_FAILED",e))
}

async fn peer_preflight(State(state):State<AppState>, headers:HeaderMap, Json(request):Json<StockPeerPreflightRequest>) -> Result<Json<StockMarketSnapshot>,AppError> {
    let asset=request.asset.clone();
    let result=state.backpack_stocks().peer_preflight(request,state.ws_hub()).await;
    audit::record_http_event(&headers,"stock_peer.preflight",&asset,if result.is_ok(){"read"}else{"rejected"},
        serde_json::json!({"readOnly":true,"fundAction":false,"webhookDelivery":false}));
    result.map(Json).map_err(|e|AppError::domain(StatusCode::BAD_REQUEST,"STOCK_PEER_PREFLIGHT_FAILED",e))
}

async fn peer_funding(State(state):State<AppState>, headers:HeaderMap, Json(request):Json<StockPeerFundingRequest>) -> Result<Json<StockMarketSnapshot>,AppError> {
    let asset=request.asset.clone();
    let result=state.backpack_stocks().peer_funding(request,state.ws_hub()).await;
    audit::record_http_event(&headers,"stock_peer.funding",&asset,if result.is_ok(){"read"}else{"rejected"},
        serde_json::json!({"readOnly":true,"fundAction":false,"addressCreated":false,"webhookDelivery":false}));
    result.map(Json).map_err(|e|AppError::domain(StatusCode::BAD_REQUEST,"STOCK_PEER_FUNDING_FAILED",e))
}

async fn peer_order_check(State(state):State<AppState>, headers:HeaderMap, Json(request):Json<StockPeerOrderCheckRequest>) -> Result<Json<StockMarketSnapshot>,AppError> {
    let asset=request.asset.clone();
    let result=state.backpack_stocks().check_peer_order(request,state.ws_hub()).await;
    audit::record_http_event(&headers,"stock_peer.order_check",&asset,if result.is_ok(){"checked"}else{"unresolved"},
        serde_json::json!({"validateOnly":true,"fundAction":false,"matchingEngine":false,"webhookDelivery":false}));
    result.map(Json).map_err(|e|AppError::domain(StatusCode::BAD_REQUEST,"STOCK_PEER_ORDER_CHECK_FAILED",e))
}

async fn preflight(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<StockPreflightRequest>,
) -> Result<Json<StockMarketSnapshot>, AppError> {
    let asset = request.asset.clone();
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(18),
        state
            .backpack_stocks()
            .preflight(request, state.ws_hub().clone()),
    )
    .await
    .map_err(|_| "股票预检超时，没有提交订单或资金动作".to_owned())
    .and_then(|r| r);
    audit::record_http_event(
        &headers,
        "backpack_stock.preflight",
        &asset,
        if result.is_ok() { "read" } else { "rejected" },
        serde_json::json!({"readOnly":true,"fundAction":false,"webhookDelivery":false}),
    );
    result
        .map(Json)
        .map_err(|e| AppError::domain(StatusCode::BAD_REQUEST, "STOCK_PREFLIGHT_FAILED", e))
}

async fn rfq(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<StockRfqRequest>,
) -> Result<Json<StockMarketSnapshot>, AppError> {
    let id = request.request_id.clone();
    let result = state
        .backpack_stocks()
        .request_rfq(request, state.ws_hub().clone())
        .await;
    rfq_result(&headers, "backpack_stock.rfq.request", &id, result, true)
}
async fn rfq_recheck(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<StockRfqActionRequest>,
) -> Result<Json<StockMarketSnapshot>, AppError> {
    let result = state
        .backpack_stocks()
        .recheck_rfq(&request.request_id, state.ws_hub().clone())
        .await;
    rfq_result(
        &headers,
        "backpack_stock.rfq.recheck",
        &request.request_id,
        result,
        false,
    )
}
async fn rfq_finish_unsent(
    State(state): State<AppState>, headers: HeaderMap, Json(request): Json<StockRfqRequest>,
) -> Result<Json<StockMarketSnapshot>, AppError> {
    let id = request.request_id.clone();
    let result = state.backpack_stocks().finish_unsent_rfq(request, state.ws_hub().clone()).await;
    rfq_result(&headers, "backpack_stock.rfq.finish_unsent", &id, result, false)
}
async fn rfq_cancel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<StockRfqActionRequest>,
) -> Result<Json<StockMarketSnapshot>, AppError> {
    let result = state
        .backpack_stocks()
        .cancel_rfq(&request.request_id, state.ws_hub().clone())
        .await;
    rfq_result(
        &headers,
        "backpack_stock.rfq.cancel",
        &request.request_id,
        result,
        true,
    )
}
fn rfq_result(
    headers: &HeaderMap,
    operation: &'static str,
    id: &str,
    result: Result<StockMarketSnapshot, String>,
    remote_mutation: bool,
) -> Result<Json<StockMarketSnapshot>, AppError> {
    audit::record_http_event(
        headers,
        operation,
        id,
        if result.is_ok() {
            "recorded"
        } else {
            "rejected"
        },
        serde_json::json!({"remoteMutation":remote_mutation,"executionMode":"AwaitAccept","fundAction":false,"quoteAccept":false}),
    );
    result
        .map(Json)
        .map_err(|e| AppError::domain(StatusCode::BAD_REQUEST, "STOCK_RFQ_REJECTED", e))
}

async fn monitor(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<StockMonitorRequest>,
) -> Result<Json<StockMarketSnapshot>, AppError> {
    let enabled = request.enabled;
    let webhook_enabled = request.enabled && request.alerts.enabled;
    let asset = request.quote.asset.clone();
    let result = state
        .backpack_stocks()
        .set_monitor(request, state.ws_hub().clone());
    audit::record_http_event(
        &headers,
        "backpack_stock.monitor",
        &asset,
        if result.is_ok() {
            "success"
        } else {
            "rejected"
        },
        serde_json::json!({"enabled":enabled,"fundAction":false,"webhookEnabled":webhook_enabled}),
    );
    result
        .map(Json)
        .map_err(|e| AppError::domain(StatusCode::BAD_REQUEST, "STOCK_MONITOR_REJECTED", e))
}

async fn quote(
    State(state): State<AppState>,
    Json(request): Json<StockQuoteRequest>,
) -> Result<Json<StockMarketSnapshot>, AppError> {
    state
        .backpack_stocks()
        .compare(request, state.ws_hub().clone())
        .await
        .map(Json)
        .map_err(|e| AppError::domain(StatusCode::BAD_REQUEST, "STOCK_QUOTE_REJECTED", e))
}

async fn catalog(State(state): State<AppState>) -> Result<Json<StockCatalog>, AppError> {
    state
        .backpack_stocks()
        .catalog()
        .await
        .map(Json)
        .map_err(|e| AppError::domain(StatusCode::BAD_GATEWAY, "BACKPACK_STOCK_CATALOG_FAILED", e))
}

async fn watch(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<StockWatchRequest>,
) -> Result<Json<StockMarketSnapshot>, AppError> {
    let asset = request.asset.clone().unwrap_or_default();
    let result = state
        .backpack_stocks()
        .watch(request, state.ws_hub().clone())
        .await;
    audit::record_http_event(
        &headers,
        "backpack_stock.watch",
        &asset,
        if result.is_ok() {
            "success"
        } else {
            "rejected"
        },
        serde_json::json!({"fundAction":false}),
    );
    result
        .map(Json)
        .map_err(|e| AppError::domain(StatusCode::BAD_REQUEST, "BACKPACK_STOCK_WATCH_REJECTED", e))
}
