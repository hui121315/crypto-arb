use super::*;
pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/stocks/peer/plans/recovery", post(prepare))
        .route("/api/stocks/peer/plans/recovery/cancel", post(cancel))
        .route("/api/stocks/peer/plans/recovery/submit", post(submit))
        .route("/api/stocks/peer/plans/recovery/recheck", post(recheck))
}
async fn prepare(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(r): Json<StockPeerRecoveryRequest>,
) -> Result<Json<StockMarketSnapshot>, AppError> {
    let id = r.plan_id.clone();
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(40),
        s.backpack_stocks().prepare_peer_recovery(r, s.ws_hub()),
    )
    .await
    .map_err(|_| "补偿构建未取得完整回复，请核对原计划，没有签名或发送".to_owned())
    .and_then(|r| r);
    plan_result(&headers, "stock_peer.recovery.prepare", &id, result)
}
async fn cancel(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(r): Json<StockRecoveryActionRequest>,
) -> Result<Json<StockMarketSnapshot>, AppError> {
    let id = r.plan_id.clone();
    plan_result(
        &headers,
        "stock_peer.recovery.cancel",
        &id,
        s.backpack_stocks().cancel_peer_recovery(r, s.ws_hub()),
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
        "stock_peer.recovery.submit",
        &id,
        "requested",
        serde_json::json!({"fundAction":true,"revision":r.revision,"index":r.index,"confirmedLive":r.confirm_live,"automaticRetry":false}),
    );
    let result = s
        .backpack_stocks()
        .submit_peer_recovery(r, s.ws_hub().clone(), s.trading_service().clone())
        .await;
    audit::record_http_event(
        &headers,
        "stock_peer.recovery.submit",
        &id,
        if result.is_ok() {
            "recorded"
        } else {
            "rejected_or_unresolved"
        },
        serde_json::json!({"fundAction":true,"automaticRetry":false}),
    );
    result
        .map(Json)
        .map_err(|e| AppError::domain(StatusCode::CONFLICT, "STOCK_PEER_RECOVERY_NOT_CONFIRMED", e))
}
async fn recheck(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(r): Json<StockRecoveryActionRequest>,
) -> Result<Json<StockMarketSnapshot>, AppError> {
    let id = r.plan_id.clone();
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        s.backpack_stocks().recheck_peer_recovery(r, s.ws_hub()),
    )
    .await
    .map_err(|_| "原补偿核对超时，保留占用，不重发".to_owned())
    .and_then(|r| r);
    plan_result(&headers, "stock_peer.recovery.recheck", &id, result)
}
