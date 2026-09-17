use crate::middleware::audit;
use crate::services::onchain_comparison;
use crate::state::AppState;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use common::AppError;
use serde_json::json;
use shared_types::{OnchainTokenIdentityRequest, OnchainTokenResolution};

pub(super) async fn resolve(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<OnchainTokenIdentityRequest>,
) -> Result<Json<OnchainTokenResolution>, AppError> {
    let resolution = onchain_comparison::resolve_token_identity(&state, request).await?;
    audit::record_http_event(
        &headers,
        "onchain_token.identity.resolve",
        "onchain-token-identity",
        "success",
        json!({
            "chain": resolution.chain,
            "address": resolution.address,
            "symbol": resolution.identity.as_ref().map(|identity| identity.symbol.as_str()),
            "decimals": resolution.decimals,
            "precisionSource": resolution.precision_source,
            "identityComplete": resolution.is_complete(),
            "verified": resolution.identity.as_ref().is_some_and(|identity| identity.verified),
            "readOnly": true,
        }),
    );
    Ok(Json(resolution))
}
