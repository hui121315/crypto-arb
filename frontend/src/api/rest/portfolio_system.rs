use super::*;

#[path = "portfolio_system/envelope.rs"]
mod envelope;
#[path = "portfolio_system/paths.rs"]
mod paths;
pub(crate) use envelope::{portfolio_envelope_degraded_problem, portfolio_envelope_problem};
use envelope::{
    portfolio_snapshot_from_envelope, review_page_path, spot_ticks_path,
    venue_operation_health_path,
};
use paths::{
    close_position_pair_path, close_position_path, close_run_compensation_path,
    close_run_manual_terminal_path,
};

const PORTFOLIO_NAV_HISTORY_LIMIT: usize = 1_000;

impl ApiClient {
    pub async fn portfolio_snapshot_envelope(
        &self,
    ) -> Result<shared_types::PortfolioSnapshotEnvelope, ApiError> {
        self.get_json("/api/trading/portfolio/snapshot").await
    }

    pub async fn portfolio_snapshot(&self) -> Result<shared_types::PortfolioSnapshot, ApiError> {
        let envelope = self.portfolio_snapshot_envelope().await?;
        portfolio_snapshot_from_envelope(envelope)
    }

    pub async fn portfolio_nav_history(
        &self,
    ) -> Result<
        shared_types::HistoryResponse<shared_types::history::PortfolioNavHistoryRow>,
        ApiError,
    > {
        self.get_json(&format!(
            "/api/trading/portfolio/nav-history?limit={PORTFOLIO_NAV_HISTORY_LIMIT}"
        ))
        .await
    }

    pub async fn close_portfolio_position(
        &self,
        venue: &str,
        symbol: &str,
        request: shared_types::ClosePositionRequest,
    ) -> Result<shared_types::CloseRun, ApiError> {
        self.close_portfolio_position_with_context(
            venue,
            symbol,
            request,
            &MutationRequestContext::new(),
        )
        .await
    }

    pub(crate) async fn close_portfolio_position_with_context(
        &self,
        venue: &str,
        symbol: &str,
        request: shared_types::ClosePositionRequest,
        context: &MutationRequestContext,
    ) -> Result<shared_types::CloseRun, ApiError> {
        self.post_json_with_context(&close_position_path(venue, symbol), &request, context)
            .await
    }

    pub async fn close_portfolio_position_pair(
        &self,
        venue: &str,
        symbol: &str,
        request: shared_types::ClosePositionRequest,
    ) -> Result<shared_types::CloseRun, ApiError> {
        self.close_portfolio_position_pair_with_context(
            venue,
            symbol,
            request,
            &MutationRequestContext::new(),
        )
        .await
    }

    pub(crate) async fn close_portfolio_position_pair_with_context(
        &self,
        venue: &str,
        symbol: &str,
        request: shared_types::ClosePositionRequest,
        context: &MutationRequestContext,
    ) -> Result<shared_types::CloseRun, ApiError> {
        self.post_json_with_context(&close_position_pair_path(venue, symbol), &request, context)
            .await
    }

    pub async fn close_all_portfolio_positions(
        &self,
        request: shared_types::CloseAllPositionsRequest,
    ) -> Result<shared_types::CloseRun, ApiError> {
        self.close_all_portfolio_positions_with_context(request, &MutationRequestContext::new())
            .await
    }

    pub(crate) async fn close_all_portfolio_positions_with_context(
        &self,
        request: shared_types::CloseAllPositionsRequest,
        context: &MutationRequestContext,
    ) -> Result<shared_types::CloseRun, ApiError> {
        self.post_json_with_context("/api/trading/portfolio/close-all", &request, context)
            .await
    }

    pub async fn submit_close_run_compensation(
        &self,
        close_run_id: &str,
        request: shared_types::CloseRunCompensationRequest,
    ) -> Result<shared_types::CloseRun, ApiError> {
        self.submit_close_run_compensation_with_context(
            close_run_id,
            request,
            &MutationRequestContext::new(),
        )
        .await
    }

    pub(crate) async fn submit_close_run_compensation_with_context(
        &self,
        close_run_id: &str,
        request: shared_types::CloseRunCompensationRequest,
        context: &MutationRequestContext,
    ) -> Result<shared_types::CloseRun, ApiError> {
        self.post_json_with_context(
            &close_run_compensation_path(close_run_id),
            &request,
            context,
        )
        .await
    }

    pub async fn submit_close_run_manual_terminal(
        &self,
        close_run_id: &str,
        request: shared_types::CloseRunManualTerminalRequest,
    ) -> Result<shared_types::CloseRun, ApiError> {
        self.submit_close_run_manual_terminal_with_context(
            close_run_id,
            request,
            &MutationRequestContext::new(),
        )
        .await
    }

    pub(crate) async fn submit_close_run_manual_terminal_with_context(
        &self,
        close_run_id: &str,
        request: shared_types::CloseRunManualTerminalRequest,
        context: &MutationRequestContext,
    ) -> Result<shared_types::CloseRun, ApiError> {
        self.post_json_with_context(
            &close_run_manual_terminal_path(close_run_id),
            &request,
            context,
        )
        .await
    }

    pub async fn system_health_envelope(
        &self,
    ) -> Result<shared_types::SystemHealthEnvelope, ApiError> {
        self.get_json("/api/system/health").await
    }

    pub async fn system_health(&self) -> Result<shared_types::SystemHealth, ApiError> {
        self.system_health_envelope()
            .await?
            .into_data()
            .map_err(|problem| ApiError::from_problem(*problem))
    }

    pub async fn venue_operation_health(
        &self,
    ) -> Result<shared_types::VenueOperationHealthSnapshot, ApiError> {
        self.get_json("/api/system/venue-operation-health").await
    }

    pub async fn venue_runtime_health(
        &self,
    ) -> Result<shared_types::VenueRuntimeHealthSnapshot, ApiError> {
        self.get_json("/api/system/venue-runtime-health").await
    }

    pub async fn venue_operation_health_for_venue(
        &self,
        venue: &str,
    ) -> Result<shared_types::VenueOperationHealthSnapshot, ApiError> {
        self.get_json(&venue_operation_health_path(venue)).await
    }

    pub async fn market_data_diagnostics(
        &self,
    ) -> Result<shared_types::MarketDataDiagnosticsSnapshot, ApiError> {
        self.get_json("/api/system/market-data-diagnostics").await
    }

    pub async fn market_subscriptions(
        &self,
    ) -> Result<shared_types::MarketSubscriptionsResponse, ApiError> {
        self.get_json("/api/system/market-subscriptions").await
    }

    pub async fn update_market_subscription_with_context(
        &self,
        patch: &shared_types::MarketSubscriptionPatch,
        context: &MutationRequestContext,
    ) -> Result<shared_types::MarketSubscriptionsResponse, ApiError> {
        self.patch_json_with_context("/api/system/market-subscriptions/config", patch, context)
            .await
    }

    pub async fn spot_ticks(
        &self,
        symbol: &str,
    ) -> Result<shared_types::MarketDataEnvelope<shared_types::SpotTicksPage>, ApiError> {
        self.spot_ticks_query(&shared_types::SpotTicksQuery::for_symbol(symbol))
            .await
    }

    pub async fn spot_ticks_query(
        &self,
        query: &shared_types::SpotTicksQuery,
    ) -> Result<shared_types::MarketDataEnvelope<shared_types::SpotTicksPage>, ApiError> {
        let path = spot_ticks_path(query);
        self.get_json(&path).await
    }

    pub async fn venues_quality(&self) -> Result<shared_types::VenueQualityEnvelope, ApiError> {
        self.get_json("/api/trading/venues/quality").await
    }

    pub async fn review_executed(
        &self,
        days: u32,
    ) -> Result<shared_types::ReviewEnvelope<shared_types::ExecutedTrade>, ApiError> {
        self.review_executed_page(days, None).await
    }

    pub async fn review_executed_page(
        &self,
        days: u32,
        cursor: Option<&str>,
    ) -> Result<shared_types::ReviewEnvelope<shared_types::ExecutedTrade>, ApiError> {
        self.get_json(&review_page_path("/api/review/executed", days, cursor))
            .await
    }

    pub async fn review_runtime(&self) -> Result<shared_types::ReviewRuntimeSnapshot, ApiError> {
        self.get_json("/api/review/runtime").await
    }

    pub async fn review_missed(
        &self,
        days: u32,
    ) -> Result<shared_types::ReviewEnvelope<shared_types::MissedOpportunity>, ApiError> {
        self.review_missed_page(days, None).await
    }

    pub async fn review_missed_page(
        &self,
        days: u32,
        cursor: Option<&str>,
    ) -> Result<shared_types::ReviewEnvelope<shared_types::MissedOpportunity>, ApiError> {
        self.get_json(&review_page_path("/api/review/missed", days, cursor))
            .await
    }

    pub async fn strategy_performance(
        &self,
    ) -> Result<shared_types::ReviewEnvelope<shared_types::StrategyPerformance>, ApiError> {
        self.get_json("/api/review/strategy-performance").await
    }
}
