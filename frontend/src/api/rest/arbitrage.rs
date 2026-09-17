use super::*;

#[path = "arbitrage/paths.rs"]
mod paths;
use paths::{
    hedge_confirm_path, hedge_preview_path, market_data_problem, opportunity_detail_path,
    opportunity_list_path, opportunity_list_path_for_strategy, orderbook_path, orderbook_problem,
    INDEX_COMPOSITIONS_ENVELOPE_PATH,
};

impl ApiClient {
    pub async fn futures_opportunity_list_response(
        &self,
        page_size: usize,
    ) -> Result<OpportunityListResponse, ApiError> {
        self.futures_opportunity_list_page(None, page_size).await
    }

    pub async fn futures_opportunity_list_page(
        &self,
        cursor: Option<&str>,
        page_size: usize,
    ) -> Result<OpportunityListResponse, ApiError> {
        self.get_json(&opportunity_list_path(page_size, None, cursor))
            .await
    }

    pub async fn futures_opportunity_list_for_strategy_page(
        &self,
        strategy: shared_types::StrategyKind,
        cursor: Option<&str>,
        page_size: usize,
    ) -> Result<OpportunityListResponse, ApiError> {
        self.get_json(&opportunity_list_path_for_strategy(
            page_size, None, cursor, strategy,
        ))
        .await
    }

    pub async fn futures_opportunity_list_for_symbol_response(
        &self,
        symbol: &str,
        page_size: usize,
    ) -> Result<OpportunityListResponse, ApiError> {
        self.futures_opportunity_list_for_symbol_page(symbol, None, page_size)
            .await
    }

    pub async fn futures_opportunity_list_for_symbol_page(
        &self,
        symbol: &str,
        cursor: Option<&str>,
        page_size: usize,
    ) -> Result<OpportunityListResponse, ApiError> {
        self.get_json::<OpportunityListResponse>(&opportunity_list_path(
            page_size,
            Some(symbol),
            cursor,
        ))
        .await
    }

    pub async fn scan_opportunity_list_response(
        &self,
        page_size: usize,
    ) -> Result<OpportunityListResponse, ApiError> {
        self.futures_opportunity_list_response(page_size).await
    }

    pub async fn scan_opportunity_list_page(
        &self,
        cursor: Option<&str>,
        page_size: usize,
    ) -> Result<OpportunityListResponse, ApiError> {
        self.futures_opportunity_list_page(cursor, page_size).await
    }

    pub async fn scan_opportunity_list_scoped_page(
        &self,
        strategy: Option<shared_types::StrategyKind>,
        symbol: Option<&str>,
        cursor: Option<&str>,
        page_size: usize,
    ) -> Result<OpportunityListResponse, ApiError> {
        let path = match strategy {
            Some(strategy) => {
                opportunity_list_path_for_strategy(page_size, symbol, cursor, strategy)
            }
            None => opportunity_list_path(page_size, symbol, cursor),
        };
        self.get_json(&path).await
    }

    pub async fn scan_opportunity_list_for_symbol_response(
        &self,
        symbol: &str,
        page_size: usize,
    ) -> Result<OpportunityListResponse, ApiError> {
        self.futures_opportunity_list_for_symbol_response(symbol, page_size)
            .await
    }

    pub async fn scan_opportunity_list_for_symbol_page(
        &self,
        symbol: &str,
        cursor: Option<&str>,
        page_size: usize,
    ) -> Result<OpportunityListResponse, ApiError> {
        self.futures_opportunity_list_for_symbol_page(symbol, cursor, page_size)
            .await
    }

    pub async fn main_strategy_kinds(
        &self,
    ) -> Result<Vec<shared_types::StrategyKindInfo>, ApiError> {
        self.get_json("/api/strategy/main-kinds").await
    }

    pub async fn funding_rates(&self) -> Result<FundingRatesResponse, ApiError> {
        self.get_json("/api/arbitrage/funding-rates").await
    }

    pub async fn preview_hedge(
        &self,
        request: &shared_types::HedgePreviewRequest,
    ) -> Result<shared_types::HedgePreviewResponse, ApiError> {
        self.post_json(&hedge_preview_path(&request.opportunity_id), request)
            .await
    }

    pub async fn confirm_hedge(
        &self,
        opportunity_id: &str,
        request: &shared_types::HedgeConfirmRequest,
    ) -> Result<shared_types::HedgeConfirmResponse, ApiError> {
        self.confirm_hedge_with_context(opportunity_id, request, &MutationRequestContext::new())
            .await
    }

    pub(crate) async fn confirm_hedge_with_context(
        &self,
        opportunity_id: &str,
        request: &shared_types::HedgeConfirmRequest,
        context: &MutationRequestContext,
    ) -> Result<shared_types::HedgeConfirmResponse, ApiError> {
        self.post_json_with_context(&hedge_confirm_path(opportunity_id), request, context)
            .await
    }

    pub async fn opportunity_history(
        &self,
        symbol: &str,
        limit: usize,
    ) -> Result<HistoryResponse<OpportunityHistoryRow>, ApiError> {
        let symbol = encode_query_component(symbol);
        self.get_json(&format!(
            "/api/history/opportunities?symbol={symbol}&limit={}",
            limit.max(1)
        ))
        .await
    }

    pub async fn opportunity_detail_read(
        &self,
        id: &str,
    ) -> Result<OpportunityDetailResponse, ApiError> {
        self.get_json(&opportunity_detail_path(id)).await
    }

    pub async fn orderbook(
        &self,
        venue: &str,
        symbol: &str,
        depth: usize,
    ) -> Result<shared_types::OrderBookInfo, ApiError> {
        let envelope = self.orderbook_read(venue, symbol, depth).await?;
        envelope
            .data
            .ok_or_else(|| orderbook_problem(envelope.health, venue, symbol))
    }

    pub async fn orderbook_read(
        &self,
        venue: &str,
        symbol: &str,
        depth: usize,
    ) -> Result<shared_types::MarketDataEnvelope<Option<shared_types::OrderBookInfo>>, ApiError>
    {
        self.get_json(&orderbook_path(venue, symbol, depth)).await
    }

    pub async fn index_composition(
        &self,
        venue: &str,
        symbol: &str,
    ) -> Result<shared_types::IndexCompositionSnapshot, ApiError> {
        let envelope = self.index_composition_read(venue, symbol).await?;
        envelope
            .data
            .ok_or_else(|| market_data_problem(envelope.health, venue, symbol, "index composition"))
    }

    pub async fn index_composition_read(
        &self,
        venue: &str,
        symbol: &str,
    ) -> Result<
        shared_types::MarketDataEnvelope<Option<shared_types::IndexCompositionSnapshot>>,
        ApiError,
    > {
        let venue = encode_query_component(venue);
        let symbol = encode_query_component(symbol);
        self.get_json(&format!(
            "/api/venues/index-compositions/fetch?venue={venue}&symbol={symbol}"
        ))
        .await
    }

    pub async fn index_compositions_read(
        &self,
    ) -> Result<shared_types::IndexCompositionListEnvelope, ApiError> {
        self.get_json(INDEX_COMPOSITIONS_ENVELOPE_PATH).await
    }
}
