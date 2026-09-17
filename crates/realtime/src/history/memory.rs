use super::types::{
    match_opt, trim_to_max, ApiHealthQuery, ApiHealthSampleRow, EventQuery, FundingDiffQuery,
    FundingDiffRow, FundingQuery, FundingRow, HistoryError, IndexCompositionHistoryRow,
    IndexCompositionQuery, LedgerEventRow, OpportunityQuery, OpportunityRow,
};
use shared_types::{ArbitrageOpportunityDto, FundingRateData, IndexCompositionSnapshot};
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::RwLock;

mod opportunity;
mod query;

use opportunity::OpportunityMemory;
use query::latest_matching_rows;

#[derive(Debug, Clone)]
pub(super) struct MemoryHistoryStore {
    funding: Arc<RwLock<VecDeque<FundingRow>>>,
    funding_diffs: Arc<RwLock<VecDeque<FundingDiffRow>>>,
    opportunities: OpportunityMemory,
    index_compositions: Arc<RwLock<VecDeque<IndexCompositionHistoryRow>>>,
    api_health: Arc<RwLock<VecDeque<ApiHealthSampleRow>>>,
    events: Arc<RwLock<VecDeque<LedgerEventRow>>>,
    max_rows: usize,
}

impl MemoryHistoryStore {
    pub(super) fn new(max_rows: usize) -> Self {
        let max_rows = max_rows.max(1_000);
        Self {
            funding: Arc::new(RwLock::new(VecDeque::new())),
            funding_diffs: Arc::new(RwLock::new(VecDeque::new())),
            opportunities: OpportunityMemory::new(max_rows),
            index_compositions: Arc::new(RwLock::new(VecDeque::new())),
            api_health: Arc::new(RwLock::new(VecDeque::new())),
            events: Arc::new(RwLock::new(VecDeque::new())),
            max_rows,
        }
    }

    pub(super) async fn append_funding_rates(
        &self,
        rows: &[FundingRateData],
    ) -> Result<(), HistoryError> {
        let now = common::time::now_ms();
        let mut out = self.funding.write().await;
        out.extend(rows.iter().map(|rate| FundingRow {
            occurred_at_ms: now,
            exchange: rate.exchange.clone(),
            symbol: rate.symbol.clone(),
            rate: rate.rate,
            interval_hours: rate.funding_interval,
            next_funding_ms: rate.next_funding_time,
            volume_24h: rate.volume_24h,
        }));
        trim_to_max(&mut out, self.max_rows);
        Ok(())
    }

    pub(super) async fn append_funding_diffs(
        &self,
        rows: &[FundingDiffRow],
    ) -> Result<(), HistoryError> {
        let mut out = self.funding_diffs.write().await;
        out.extend(rows.iter().cloned());
        trim_to_max(&mut out, self.max_rows);
        Ok(())
    }

    pub(super) async fn append_opportunities(
        &self,
        rows: &[ArbitrageOpportunityDto],
    ) -> Result<(), HistoryError> {
        self.opportunities.append(rows).await
    }

    pub(super) async fn append_index_compositions(
        &self,
        rows: &[IndexCompositionSnapshot],
    ) -> Result<(), HistoryError> {
        let now = common::time::now_ms();
        let mut out = self.index_compositions.write().await;
        out.extend(rows.iter().map(|row| IndexCompositionHistoryRow {
            occurred_at_ms: now,
            venue: row.venue.clone(),
            symbol: row.symbol.clone(),
            index_id: row.index_id.clone(),
            quality: row.quality,
            component_count: row.components.len(),
            source: row.source.clone(),
            payload: row.clone(),
        }));
        trim_to_max(&mut out, self.max_rows);
        Ok(())
    }

    pub(super) async fn append_api_health(
        &self,
        rows: &[ApiHealthSampleRow],
    ) -> Result<(), HistoryError> {
        let mut out = self.api_health.write().await;
        out.extend(rows.iter().cloned());
        trim_to_max(&mut out, self.max_rows);
        Ok(())
    }

    pub(super) async fn append_events(&self, rows: &[LedgerEventRow]) -> Result<(), HistoryError> {
        let mut out = self.events.write().await;
        out.extend(rows.iter().cloned());
        trim_to_max(&mut out, self.max_rows);
        Ok(())
    }

    pub(super) async fn row_count(&self) -> usize {
        let funding = self.funding.read().await.len();
        let funding_diffs = self.funding_diffs.read().await.len();
        let opportunities = self.opportunities.row_count().await;
        let index_compositions = self.index_compositions.read().await.len();
        let api_health = self.api_health.read().await.len();
        let events = self.events.read().await.len();
        funding
            .saturating_add(funding_diffs)
            .saturating_add(opportunities)
            .saturating_add(index_compositions)
            .saturating_add(api_health)
            .saturating_add(events)
    }

    pub(super) async fn query_funding(
        &self,
        query: FundingQuery,
    ) -> Result<Vec<FundingRow>, HistoryError> {
        let rows = self.funding.read().await;
        Ok(latest_matching_rows(
            &rows,
            query.limit,
            |row| {
                match_opt(&query.symbol, &row.symbol)
                    && match_opt(&query.exchange, &row.exchange)
                    && in_time_range(row.occurred_at_ms, query.from_ms, query.to_ms)
            },
            |row| row.occurred_at_ms,
        ))
    }

    pub(super) async fn query_funding_diffs(
        &self,
        query: FundingDiffQuery,
    ) -> Result<Vec<FundingDiffRow>, HistoryError> {
        let rows = self.funding_diffs.read().await;
        Ok(latest_matching_rows(
            &rows,
            query.limit,
            |row| {
                match_opt(&query.symbol, &row.symbol)
                    && match_opt(&query.long_exchange, &row.long_exchange)
                    && match_opt(&query.short_exchange, &row.short_exchange)
                    && in_time_range(row.occurred_at_ms, query.from_ms, query.to_ms)
            },
            |row| row.occurred_at_ms,
        ))
    }

    pub(super) async fn query_opportunities(
        &self,
        query: OpportunityQuery,
    ) -> Result<Vec<OpportunityRow>, HistoryError> {
        self.opportunities.query(query).await
    }

    pub(super) async fn query_index_compositions(
        &self,
        query: IndexCompositionQuery,
    ) -> Result<Vec<IndexCompositionHistoryRow>, HistoryError> {
        let rows = self.index_compositions.read().await;
        Ok(latest_matching_rows(
            &rows,
            query.limit,
            |row| {
                match_opt(&query.venue, &row.venue)
                    && match_opt(&query.symbol, &row.symbol)
                    && in_time_range(row.occurred_at_ms, query.from_ms, query.to_ms)
            },
            |row| row.occurred_at_ms,
        ))
    }

    pub(super) async fn query_api_health(
        &self,
        query: ApiHealthQuery,
    ) -> Result<Vec<ApiHealthSampleRow>, HistoryError> {
        let rows = self.api_health.read().await;
        Ok(latest_matching_rows(
            &rows,
            query.limit,
            |row| {
                match_opt(&query.exchange, &row.exchange)
                    && match_opt(&query.endpoint, &row.endpoint)
                    && match_opt(&query.outcome, &row.outcome)
                    && in_time_range(row.occurred_at_ms, query.from_ms, query.to_ms)
            },
            |row| row.occurred_at_ms,
        ))
    }

    pub(super) async fn query_events(
        &self,
        query: EventQuery,
    ) -> Result<Vec<LedgerEventRow>, HistoryError> {
        let rows = self.events.read().await;
        Ok(latest_matching_rows(
            &rows,
            query.limit,
            |row| {
                match_opt(&query.category, &row.category)
                    && match_opt(&query.action, &row.action)
                    && match_opt_some(&query.request_id, &row.request_id)
                    && match_opt_some(&query.run_id, &row.run_id)
                    && match_opt_some(&query.ticket_id, &row.ticket_id)
                    && match_opt_some(&query.client_order_id, &row.client_order_id)
                    && match_opt_some(&query.exchange_order_id, &row.exchange_order_id)
                    && in_time_range(row.occurred_at_ms, query.from_ms, query.to_ms)
            },
            |row| row.occurred_at_ms,
        ))
    }
}

/// Match a requested correlation id against an optional stored id. A `None`
/// filter matches everything; a stored `None` never matches a concrete filter.
fn match_opt_some(filter: &Option<String>, value: &Option<String>) -> bool {
    match filter {
        Some(wanted) => value.as_deref() == Some(wanted.as_str()),
        None => true,
    }
}

fn in_time_range(value: i64, from_ms: Option<i64>, to_ms: Option<i64>) -> bool {
    from_ms.map(|from| value >= from).unwrap_or(true) && to_ms.map(|to| value <= to).unwrap_or(true)
}
