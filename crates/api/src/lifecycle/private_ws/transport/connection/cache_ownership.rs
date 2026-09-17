use super::*;

pub(super) fn observe_owned_private_ws_caches(state: &AppState, venue: &str) {
    let health = state.private_ws_health();
    let service = state.trading_service();
    if health.account_session_owns_cache(venue) {
        service.observe_private_ws_account_session_activity(venue);
    }
    if health.order_session_owns_cache(venue) {
        service.observe_private_ws_order_session_activity(venue);
    }
}
