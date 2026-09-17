use shared_types::{AlertRulesEnvelope, WatchlistEnvelope};

use super::{ApiClient, ApiError};

impl ApiClient {
    pub(crate) async fn watchlist_quiet(&self) -> Result<WatchlistEnvelope, ApiError> {
        self.get_json_quiet("/api/watchlist").await
    }

    pub async fn watchlist(&self) -> Result<WatchlistEnvelope, ApiError> {
        self.get_json("/api/watchlist").await
    }

    pub(crate) async fn alert_rules_quiet(&self) -> Result<AlertRulesEnvelope, ApiError> {
        self.get_json_quiet("/api/alerts/rules").await
    }

    pub async fn alert_rules(&self) -> Result<AlertRulesEnvelope, ApiError> {
        self.get_json("/api/alerts/rules").await
    }
}
