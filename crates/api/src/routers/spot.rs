use crate::services::spot;
use crate::state::AppState;
use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use shared_types::{MarketDataEnvelope, SpotTicksPage, SpotTicksQuery};

pub(crate) fn router() -> Router<AppState> {
    Router::new().route("/api/v1/spot/ticks", get(ticks))
}

async fn ticks(
    State(state): State<AppState>,
    Query(query): Query<SpotTicksQuery>,
) -> Json<MarketDataEnvelope<SpotTicksPage>> {
    Json(spot::filtered_ticks(&state, &query).await)
}
