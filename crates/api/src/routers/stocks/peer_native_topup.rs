use super::*;

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/stocks/peer/plans/native-topup", post(prepare))
        .route("/api/stocks/peer/plans/native-topup/cancel", post(cancel))
        .route("/api/stocks/peer/plans/native-topup/submit", post(submit))
        .route("/api/stocks/peer/plans/native-topup/recheck", post(recheck))
}
async fn prepare(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(r): Json<StockPeerNativeTopupRequest>,
) -> Result<Json<StockMarketSnapshot>, AppError> {
    let id = r.plan_id.clone();
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(40),
        s.backpack_stocks().prepare_peer_native_topup(r, s.ws_hub()),
    )
    .await
    .map_err(|_| "SOL 补回构建超时，没有签名或发送".to_owned())
    .and_then(|r| r);
    plan_result(&headers, "stock_peer.native_topup.prepare", &id, result)
}
async fn cancel(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(r): Json<StockRecoveryActionRequest>,
) -> Result<Json<StockMarketSnapshot>, AppError> {
    let id = r.plan_id.clone();
    plan_result(
        &headers,
        "stock_peer.native_topup.cancel",
        &id,
        s.backpack_stocks().cancel_peer_native_topup(r, s.ws_hub()),
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
        "stock_peer.native_topup.submit",
        &id,
        "requested",
        serde_json::json!({"fundAction":true,"revision":r.revision,"index":r.index,"confirmedLive":r.confirm_live,"automaticRetry":false}),
    );
    let result = s
        .backpack_stocks()
        .submit_peer_native_topup(r, s.ws_hub().clone(), s.trading_service().clone())
        .await;
    audit::record_http_event(
        &headers,
        "stock_peer.native_topup.submit",
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
            "STOCK_PEER_NATIVE_TOPUP_NOT_CONFIRMED",
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
        std::time::Duration::from_secs(20),
        s.backpack_stocks().recheck_peer_native_topup(r, s.ws_hub()),
    )
    .await
    .map_err(|_| "SOL 原补回核对超时，保留占用，不重发".to_owned())
    .and_then(|r| r);
    plan_result(&headers, "stock_peer.native_topup.recheck", &id, result)
}
