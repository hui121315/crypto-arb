use crate::services::onchain_comparison;
use crate::state::AppState;
use axum::extract::{Query, State};
use axum::Json;
use common::AppError;
use shared_types::{OnchainCexPairCatalog, OnchainCexPairQuery};

pub(super) async fn list(
    State(state): State<AppState>,
    Query(query): Query<OnchainCexPairQuery>,
) -> Result<Json<OnchainCexPairCatalog>, AppError> {
    Ok(Json(
        onchain_comparison::cex_pair_catalog(&state, &query).await?,
    ))
}
