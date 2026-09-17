//! Authentication DTOs shared by REST and browser WebSocket clients.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WsTicketRequest {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WsTicketResponse {
    pub ticket: String,
    pub expires_at_ms: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ws_ticket_response_uses_camel_case_contract() -> Result<(), serde_json::Error> {
        let json = serde_json::to_value(WsTicketResponse {
            ticket: "ticket-1".to_owned(),
            expires_at_ms: 123,
        })?;

        assert_eq!(json["ticket"], "ticket-1");
        assert_eq!(json["expiresAtMs"], 123);
        Ok(())
    }
}
