use super::*;

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/stocks/funding/exchange-conversions", post(build))
        .route(
            "/api/stocks/funding/exchange-conversions/cancel",
            post(cancel),
        )
        .route(
            "/api/stocks/funding/exchange-conversions/submit",
            post(submit),
        )
        .route(
            "/api/stocks/funding/exchange-conversions/recheck",
            post(recheck),
        )
}
async fn build(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(r): Json<StockExchangeConversionRequest>,
) -> Result<Json<StockMarketSnapshot>, AppError> {
    let id = r.request_id.clone();
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        state
            .backpack_stocks()
            .build_exchange_conversion(r, state.ws_hub()),
    )
    .await
    .map_err(|_| "账户兑换预检超时，未提交订单".to_owned())
    .and_then(|r| r);
    plan_result(
        &headers,
        "backpack_stock.exchange_conversion.reserve",
        &id,
        result,
    )
}
async fn cancel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(r): Json<StockPlanRevisionRequest>,
) -> Result<Json<StockMarketSnapshot>, AppError> {
    let id = r.plan_id.clone();
    plan_result(
        &headers,
        "backpack_stock.exchange_conversion.cancel",
        &id,
        state
            .backpack_stocks()
            .cancel_exchange_conversion(r, state.ws_hub()),
    )
}
async fn submit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(r): Json<StockStablecoinSubmitRequest>,
) -> Result<Json<StockMarketSnapshot>, AppError> {
    let id = r.plan_id.clone();
    let context = serde_json::json!({"fundAction":true,"revision":r.revision,"confirmedLive":r.confirm_live,"automaticRetry":false});
    audit::record_http_event(
        &headers,
        "backpack_stock.exchange_conversion.submit",
        &id,
        "requested",
        context.clone(),
    );
    let result = state
        .backpack_stocks()
        .submit_exchange_conversion(r, state.ws_hub().clone(), state.trading_service().clone())
        .await;
    audit::record_http_event(
        &headers,
        "backpack_stock.exchange_conversion.submit",
        &id,
        if result.is_ok() {
            "recorded"
        } else {
            "rejected_or_unresolved"
        },
        context,
    );
    result.map(Json).map_err(|e| {
        AppError::domain(
            StatusCode::CONFLICT,
            "STOCK_EXCHANGE_CONVERSION_NOT_CONFIRMED",
            e,
        )
    })
}
async fn recheck(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(r): Json<StockPlanCancelRequest>,
) -> Result<Json<StockMarketSnapshot>, AppError> {
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(25),
        state
            .backpack_stocks()
            .recheck_exchange_conversion(&r.plan_id, state.ws_hub()),
    )
    .await
    .map_err(|_| "原账户兑换查询超时，保留占用且不重发".to_owned())
    .and_then(|r| r);
    plan_result(
        &headers,
        "backpack_stock.exchange_conversion.recheck",
        &r.plan_id,
        result,
    )
}
