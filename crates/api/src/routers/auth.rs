//! Authentication helper routes.

use crate::state::AppState;
use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use shared_types::WsTicketResponse;

pub(crate) fn router() -> Router<AppState> {
    Router::new().route("/api/auth/ws-ticket", post(ws_ticket))
}

async fn ws_ticket(State(state): State<AppState>) -> Json<WsTicketResponse> {
    Json(state.ws_tickets().issue(common::time::now_ms()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::config::AppConfig;

    #[tokio::test]
    async fn ws_ticket_route_issues_single_use_ticket() -> Result<(), anyhow::Error> {
        let mut config = AppConfig::default();
        config.history.enabled = false;
        let state = AppState::new(config).await?;

        let Json(response) = ws_ticket(State(state.clone())).await;

        assert_eq!(
            state
                .ws_tickets()
                .consume(Some(&response.ticket), common::time::now_ms()),
            crate::services::ws_auth::WsTicketConsume::Accepted
        );
        assert_eq!(
            state
                .ws_tickets()
                .consume(Some(&response.ticket), common::time::now_ms()),
            crate::services::ws_auth::WsTicketConsume::NotFound
        );
        Ok(())
    }
}
