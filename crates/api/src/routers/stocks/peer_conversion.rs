use super::*;
pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/stocks/peer/plans/conversion", post(prepare))
        .route("/api/stocks/peer/plans/conversion/cancel", post(cancel))
        .route("/api/stocks/peer/plans/conversion/submit", post(submit))
        .route("/api/stocks/peer/plans/conversion/recheck", post(recheck))
}
async fn prepare(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(r): Json<StockPeerConversionRequest>,
) -> Result<Json<StockMarketSnapshot>, AppError> {
    let id = r.plan_id.clone();
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        s.backpack_stocks().prepare_peer_conversion(r, s.ws_hub()),
    )
    .await
    .map_err(|_| "换汇准备超时，请核对记录，没有下单".to_owned())
    .and_then(|r| r);
    plan_result(&headers, "stock_peer.conversion.prepare", &id, result)
}
async fn cancel(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(r): Json<StockRecoveryActionRequest>,
) -> Result<Json<StockMarketSnapshot>, AppError> {
    let id = r.plan_id.clone();
    plan_result(
        &headers,
        "stock_peer.conversion.cancel",
        &id,
        s.backpack_stocks().cancel_peer_conversion(r, s.ws_hub()),
    )
}
async fn submit(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(r): Json<StockPeerRecoverySubmitRequest>,
) -> Result<Json<StockMarketSnapshot>, AppError> {
    let id = r.plan_id.clone();
    audit::record_http_event(
        &headers,
        "stock_peer.conversion.submit",
        &id,
        "requested",
        serde_json::json!({"fundAction":true,"index":r.index,"revision":r.revision,"confirmedLive":r.confirm_live,"automaticRetry":false}),
    );
    let result = s
        .backpack_stocks()
        .submit_peer_conversion(r, s.ws_hub().clone(), s.trading_service().clone())
        .await;
    audit::record_http_event(
        &headers,
        "stock_peer.conversion.submit",
        &id,
        if result.is_ok() {
            "recorded"
        } else {
            "rejected_or_unresolved"
        },
        serde_json::json!({"fundAction":true,"automaticRetry":false}),
    );
    result.map(Json).map_err(|e| {
        AppError::domain(
            StatusCode::CONFLICT,
            "STOCK_PEER_CONVERSION_NOT_CONFIRMED",
            e,
        )
    })
}
async fn recheck(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(r): Json<StockRecoveryActionRequest>,
) -> Result<Json<StockMarketSnapshot>, AppError> {
    let id = r.plan_id.clone();
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(18),
        s.backpack_stocks().recheck_peer_conversion(r, s.ws_hub()),
    )
    .await
    .map_err(|_| "原换汇查询超时，保留占用，不重发".to_owned())
    .and_then(|r| r);
    plan_result(&headers, "stock_peer.conversion.recheck", &id, result)
}
