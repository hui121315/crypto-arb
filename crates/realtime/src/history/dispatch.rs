use super::*;
use shared_types::{
    ArbitrageOpportunityDto, FundingDiffStatsRow, FundingRateData, IndexCompositionSnapshot,
};

impl HistoryStore {
    pub async fn append_funding_rates(&self, rows: &[FundingRateData]) -> Result<(), HistoryError> {
        let result = match &self.backend {
            HistoryBackend::Memory(memory) => memory.append_funding_rates(rows).await,
            HistoryBackend::Postgres(pg) => pg.append_funding_rates(rows).await,
            HistoryBackend::Disabled => Ok(()),
        };
        self.record_append(result)
    }

    pub async fn append_funding_diffs(&self, rows: &[FundingDiffRow]) -> Result<(), HistoryError> {
        let result = match &self.backend {
            HistoryBackend::Memory(memory) => memory.append_funding_diffs(rows).await,
            HistoryBackend::Postgres(pg) => pg.append_funding_diffs(rows).await,
            HistoryBackend::Disabled => Ok(()),
        };
        self.record_append(result)
    }

    pub fn derive_funding_diffs(rows: &[FundingRateData]) -> Vec<FundingDiffRow> {
        funding_diff::derive_rows(rows, common::time::now_ms())
    }

    pub async fn append_opportunities(
        &self,
        rows: &[ArbitrageOpportunityDto],
    ) -> Result<(), HistoryError> {
        let result = match &self.backend {
            HistoryBackend::Memory(memory) => memory.append_opportunities(rows).await,
            HistoryBackend::Postgres(pg) => pg.append_opportunities(rows).await,
            HistoryBackend::Disabled => Ok(()),
        };
        self.record_append(result)
    }

    pub async fn append_index_compositions(
        &self,
        rows: &[IndexCompositionSnapshot],
    ) -> Result<(), HistoryError> {
        let result = match &self.backend {
            HistoryBackend::Memory(memory) => memory.append_index_compositions(rows).await,
            HistoryBackend::Postgres(pg) => pg.append_index_compositions(rows).await,
            HistoryBackend::Disabled => Ok(()),
        };
        self.record_append(result)
    }

    pub async fn append_api_health(&self, rows: &[ApiHealthSampleRow]) -> Result<(), HistoryError> {
        let result = match &self.backend {
            HistoryBackend::Memory(memory) => memory.append_api_health(rows).await,
            HistoryBackend::Postgres(pg) => pg.append_api_health(rows).await,
            HistoryBackend::Disabled => Ok(()),
        };
        self.record_append(result)
    }

    pub async fn append_events(&self, rows: &[LedgerEventRow]) -> Result<(), HistoryError> {
        let result = match &self.backend {
            HistoryBackend::Memory(memory) => memory.append_events(rows).await,
            HistoryBackend::Postgres(pg) => pg.append_events(rows).await,
            HistoryBackend::Disabled => Ok(()),
        };
        self.record_append(result)
    }

    pub async fn query_api_health(
        &self,
        query: ApiHealthQuery,
    ) -> Result<Vec<ApiHealthSampleRow>, HistoryError> {
        let result = match &self.backend {
            HistoryBackend::Memory(memory) => memory.query_api_health(query).await,
            HistoryBackend::Postgres(pg) => pg.query_api_health(query).await,
            HistoryBackend::Disabled => Ok(Vec::new()),
        };
        self.record_query(result)
    }

    pub async fn query_events(
        &self,
        query: EventQuery,
    ) -> Result<Vec<LedgerEventRow>, HistoryError> {
        let result = match &self.backend {
            HistoryBackend::Memory(memory) => memory.query_events(query).await,
            HistoryBackend::Postgres(pg) => pg.query_events(query).await,
            HistoryBackend::Disabled => Ok(Vec::new()),
        };
        self.record_query(result)
    }

    pub async fn query_funding(
        &self,
        query: FundingQuery,
    ) -> Result<Vec<FundingRow>, HistoryError> {
        let result = match &self.backend {
            HistoryBackend::Memory(memory) => memory.query_funding(query).await,
            HistoryBackend::Postgres(pg) => pg.query_funding(query).await,
            HistoryBackend::Disabled => Ok(Vec::new()),
        };
        self.record_query(result)
    }

    pub async fn query_funding_diffs(
        &self,
        query: FundingDiffQuery,
    ) -> Result<Vec<FundingDiffRow>, HistoryError> {
        let result = match &self.backend {
            HistoryBackend::Memory(memory) => memory.query_funding_diffs(query).await,
            HistoryBackend::Postgres(pg) => pg.query_funding_diffs(query).await,
            HistoryBackend::Disabled => Ok(Vec::new()),
        };
        self.record_query(result)
    }

    pub async fn query_funding_diff_stats(
        &self,
        query: FundingDiffStatsQuery,
    ) -> Result<Vec<FundingDiffStatsRow>, HistoryError> {
        let rows = self.query_funding_diffs(diff_query(query)).await?;
        Ok(funding_stats::build_rows(rows, common::time::now_ms()))
    }

    pub async fn query_opportunities(
        &self,
        query: OpportunityQuery,
    ) -> Result<Vec<OpportunityRow>, HistoryError> {
        let result = match &self.backend {
            HistoryBackend::Memory(memory) => memory.query_opportunities(query).await,
            HistoryBackend::Postgres(pg) => pg.query_opportunities(query).await,
            HistoryBackend::Disabled => Ok(Vec::new()),
        };
        self.record_query(result)
    }

    pub async fn query_index_compositions(
        &self,
        query: IndexCompositionQuery,
    ) -> Result<Vec<IndexCompositionHistoryRow>, HistoryError> {
        let result = match &self.backend {
            HistoryBackend::Memory(memory) => memory.query_index_compositions(query).await,
            HistoryBackend::Postgres(pg) => pg.query_index_compositions(query).await,
            HistoryBackend::Disabled => Ok(Vec::new()),
        };
        self.record_query(result)
    }
}
