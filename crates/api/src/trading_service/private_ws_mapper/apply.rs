use super::*;

pub(crate) async fn apply_events(
    service: &TradingService,
    events: Vec<PrivateWsEvent>,
) -> Vec<PrivateWsApplyOutcome> {
    let mut outcomes = Vec::with_capacity(events.len());
    for event in events {
        outcomes.push(service.apply_private_ws_event(event).await);
    }
    outcomes
}
