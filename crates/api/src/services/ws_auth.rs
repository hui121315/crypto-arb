//! Browser WebSocket ticket service.

use dashmap::DashMap;
use shared_types::WsTicketResponse;

const WS_TICKET_TTL_MS: i64 = 30_000;
const MAX_WS_TICKETS: usize = 1024;

#[derive(Debug, Default)]
pub(crate) struct WsTicketStore {
    tickets: DashMap<String, WsTicket>,
}

#[derive(Debug, Clone, Copy)]
struct WsTicket {
    expires_at_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WsTicketConsume {
    Accepted,
    Missing,
    NotFound,
    Expired,
}

impl WsTicketStore {
    pub(crate) fn issue(&self, now_ms: i64) -> WsTicketResponse {
        self.prune(now_ms);
        self.prune_capacity();
        let ticket = uuid::Uuid::new_v4().simple().to_string();
        let expires_at_ms = now_ms.saturating_add(WS_TICKET_TTL_MS);
        self.tickets
            .insert(ticket.clone(), WsTicket { expires_at_ms });
        WsTicketResponse {
            ticket,
            expires_at_ms,
        }
    }

    pub(crate) fn consume(&self, ticket: Option<&str>, now_ms: i64) -> WsTicketConsume {
        let Some(ticket) = ticket.map(str::trim).filter(|ticket| !ticket.is_empty()) else {
            return WsTicketConsume::Missing;
        };
        let Some((_, stored)) = self.tickets.remove(ticket) else {
            return WsTicketConsume::NotFound;
        };
        if stored.expires_at_ms < now_ms {
            WsTicketConsume::Expired
        } else {
            WsTicketConsume::Accepted
        }
    }

    fn prune(&self, now_ms: i64) {
        self.tickets
            .retain(|_, ticket| ticket.expires_at_ms >= now_ms);
    }

    fn prune_capacity(&self) {
        while self.tickets.len() >= MAX_WS_TICKETS {
            let Some(key) = self.tickets.iter().next().map(|entry| entry.key().clone()) else {
                return;
            };
            self.tickets.remove(&key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticket_is_one_time_and_expires() {
        let store = WsTicketStore::default();
        let ticket = store.issue(10);

        assert_eq!(
            store.consume(Some(&ticket.ticket), 20),
            WsTicketConsume::Accepted
        );
        assert_eq!(
            store.consume(Some(&ticket.ticket), 21),
            WsTicketConsume::NotFound
        );

        let expired = store.issue(100);
        assert_eq!(
            store.consume(Some(&expired.ticket), expired.expires_at_ms + 1),
            WsTicketConsume::Expired
        );
    }

    #[test]
    fn missing_or_unknown_ticket_is_rejected() {
        let store = WsTicketStore::default();

        assert_eq!(store.consume(None, 1), WsTicketConsume::Missing);
        assert_eq!(store.consume(Some(""), 1), WsTicketConsume::Missing);
        assert_eq!(store.consume(Some("unknown"), 1), WsTicketConsume::NotFound);
    }
}
