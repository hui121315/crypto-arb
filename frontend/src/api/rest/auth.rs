use super::*;

impl ApiClient {
    pub async fn ws_ticket(&self) -> Result<shared_types::WsTicketResponse, ApiError> {
        self.post_json(
            "/api/auth/ws-ticket",
            &shared_types::WsTicketRequest::default(),
        )
        .await
    }
}
