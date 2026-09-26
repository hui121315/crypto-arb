use super::AppState;
use crate::trading_service::PrivateWsAccountLease;

#[derive(Clone)]
pub(super) struct PrivateWsSession(PrivateWsAccountLease);

impl PrivateWsSession {
    pub(super) fn capture(state: &AppState, venue: &str) -> Self {
        Self(state.trading_service().private_ws_account_lease(venue))
    }

    pub(super) async fn lock<'a>(
        &self,
        state: &'a AppState,
    ) -> Option<tokio::sync::MutexGuard<'a, ()>> {
        let guard = state.trading_runtime_config_mutation_lock().lock().await;
        self.0.is_current().then_some(guard)
    }
}
