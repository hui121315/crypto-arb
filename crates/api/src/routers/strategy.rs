use crate::services::strategy;
use crate::state::AppState;
use axum::routing::get;
use axum::{Json, Router};
use shared_types::StrategyKindInfo;

pub(crate) fn main_router() -> Router<AppState> {
    Router::new().route("/api/strategy/main-kinds", get(list_main_kinds))
}

pub(crate) fn v1_router() -> Router<AppState> {
    Router::new().route("/api/v1/strategy/kinds", get(list_kinds))
}

async fn list_main_kinds() -> Json<Vec<StrategyKindInfo>> {
    Json(strategy::list_main_kinds())
}

async fn list_kinds() -> Json<Vec<StrategyKindInfo>> {
    Json(strategy::list_kinds())
}
