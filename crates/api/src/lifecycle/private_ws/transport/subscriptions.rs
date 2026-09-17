use super::super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SubscriptionConfirmation {
    Send,
    ServerAck,
}

pub(in super::super) async fn send_private_ws_subscriptions(
    venue: &'static str,
    state: &AppState,
    manager: &Arc<WsManager>,
    messages_on_connect: &[String],
) -> bool {
    send_private_ws_subscriptions_with_confirmation(
        venue,
        state,
        manager,
        messages_on_connect,
        SubscriptionConfirmation::Send,
    )
    .await
}

pub(super) async fn send_private_ws_subscriptions_with_confirmation(
    venue: &'static str,
    state: &AppState,
    manager: &Arc<WsManager>,
    messages_on_connect: &[String],
    confirmation: SubscriptionConfirmation,
) -> bool {
    state
        .private_ws_health()
        .record_subscribe_attempt(venue, messages_on_connect.len());
    let mut sent = 0usize;
    for message in messages_on_connect {
        if let Err(error) = manager.send_text(message.clone()).await {
            state.private_ws_health().record_subscribe_failed(
                venue,
                messages_on_connect.len(),
                sent,
                &error.to_string(),
            );
            warn!(%error, %venue, "private ws send on-connect failed");
            return false;
        }
        sent = sent.saturating_add(1);
    }
    if confirmation == SubscriptionConfirmation::ServerAck {
        state.private_ws_health().record_subscribe_sent_pending_ack(
            venue,
            messages_on_connect.len(),
            sent,
        );
    } else {
        state
            .private_ws_health()
            .record_subscribe_sent(venue, messages_on_connect.len(), sent);
    }
    true
}
