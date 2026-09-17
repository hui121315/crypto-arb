use super::*;

impl PostgresHistoryStore {
    pub(in crate::history) async fn query_api_health(
        &self,
        query: ApiHealthQuery,
    ) -> Result<Vec<ApiHealthSampleRow>, HistoryError> {
        let exchange = query.exchange.as_deref();
        let endpoint = query.endpoint.as_deref();
        let outcome = query.outcome.as_deref();
        let from_ms = query.from_ms;
        let to_ms = query.to_ms;
        let limit = query.limit.clamp(1, 5_000) as i64;
        let rows = self
            .pool
            .client()
            .await?
            .query(
                API_HEALTH_QUERY,
                &[&exchange, &endpoint, &outcome, &from_ms, &to_ms, &limit],
            )
            .await
            .map_err(|error| postgres_history_error(&error))?;
        Ok(rows.iter().map(api_health_from_row).collect())
    }

    pub(in crate::history) async fn query_events(
        &self,
        query: EventQuery,
    ) -> Result<Vec<LedgerEventRow>, HistoryError> {
        let category = query.category.as_deref();
        let action = query.action.as_deref();
        let request_id = query.request_id.as_deref();
        let run_id = query.run_id.as_deref();
        let ticket_id = query.ticket_id.as_deref();
        let client_order_id = query.client_order_id.as_deref();
        let exchange_order_id = query.exchange_order_id.as_deref();
        let from_ms = query.from_ms;
        let to_ms = query.to_ms;
        let limit = query.limit.clamp(1, 5_000) as i64;
        let rows = self
            .pool
            .client()
            .await?
            .query(
                EVENT_QUERY,
                &[
                    &category,
                    &action,
                    &request_id,
                    &run_id,
                    &ticket_id,
                    &client_order_id,
                    &exchange_order_id,
                    &from_ms,
                    &to_ms,
                    &limit,
                ],
            )
            .await
            .map_err(|error| postgres_history_error(&error))?;
        Ok(rows.iter().map(event_from_row).collect())
    }

    pub(in crate::history) async fn query_funding(
        &self,
        query: FundingQuery,
    ) -> Result<Vec<FundingRow>, HistoryError> {
        let symbol = query.symbol.as_deref();
        let exchange = query.exchange.as_deref();
        let from_ms = query.from_ms;
        let to_ms = query.to_ms;
        let limit = query.limit.clamp(1, 5_000) as i64;
        let rows = self
            .pool
            .client()
            .await?
            .query(
                FUNDING_QUERY,
                &[&symbol, &exchange, &from_ms, &to_ms, &limit],
            )
            .await
            .map_err(|error| postgres_history_error(&error))?;
        Ok(rows.iter().map(funding_from_row).collect())
    }

    pub(in crate::history) async fn query_funding_diffs(
        &self,
        query: FundingDiffQuery,
    ) -> Result<Vec<FundingDiffRow>, HistoryError> {
        let symbol = query.symbol.as_deref();
        let long_exchange = query.long_exchange.as_deref();
        let short_exchange = query.short_exchange.as_deref();
        let from_ms = query.from_ms;
        let to_ms = query.to_ms;
        let limit = query.limit.clamp(1, 5_000) as i64;
        let rows = self
            .pool
            .client()
            .await?
            .query(
                FUNDING_DIFF_QUERY,
                &[
                    &symbol,
                    &long_exchange,
                    &short_exchange,
                    &from_ms,
                    &to_ms,
                    &limit,
                ],
            )
            .await
            .map_err(|error| postgres_history_error(&error))?;
        Ok(rows.iter().map(funding_diff_from_row).collect())
    }

    pub(in crate::history) async fn query_opportunities(
        &self,
        query: OpportunityQuery,
    ) -> Result<Vec<OpportunityRow>, HistoryError> {
        let symbol = query.symbol.as_deref();
        let min_yield = query.min_yield;
        let from_ms = query.from_ms;
        let to_ms = query.to_ms;
        let limit = query.limit.clamp(1, 5_000) as i64;
        let rows = self
            .pool
            .client()
            .await?
            .query(
                OPPORTUNITY_QUERY,
                &[&symbol, &min_yield, &from_ms, &to_ms, &limit],
            )
            .await
            .map_err(|error| postgres_history_error(&error))?;
        rows.iter().map(opportunity_from_row).collect()
    }

    pub(in crate::history) async fn query_index_compositions(
        &self,
        query: IndexCompositionQuery,
    ) -> Result<Vec<IndexCompositionHistoryRow>, HistoryError> {
        let venue = query.venue.as_deref();
        let symbol = query.symbol.as_deref();
        let from_ms = query.from_ms;
        let to_ms = query.to_ms;
        let limit = query.limit.clamp(1, 5_000) as i64;
        let rows = self
            .pool
            .client()
            .await?
            .query(
                INDEX_COMPOSITION_QUERY,
                &[&venue, &symbol, &from_ms, &to_ms, &limit],
            )
            .await
            .map_err(|error| postgres_history_error(&error))?;
        rows.iter().map(index_composition_from_row).collect()
    }
}

pub(super) const FUNDING_QUERY: &str =
    "SELECT occurred_at_ms, exchange, symbol, rate, interval_hours, next_funding_ms, volume_24h \
 FROM funding_rates \
 WHERE ($1::TEXT IS NULL OR lower(symbol) = lower($1)) \
   AND ($2::TEXT IS NULL OR lower(exchange) = lower($2)) \
   AND ($3::BIGINT IS NULL OR occurred_at_ms >= $3) \
   AND ($4::BIGINT IS NULL OR occurred_at_ms <= $4) \
 ORDER BY occurred_at_ms DESC \
 LIMIT $5";

pub(super) const FUNDING_DIFF_QUERY: &str =
    "SELECT occurred_at_ms, symbol, long_exchange, short_exchange, long_rate_8h, short_rate_8h, gross_diff_bps, long_next_funding_ms, short_next_funding_ms, window_alignment_minutes, long_interval_hours, short_interval_hours, min_volume_24h \
 FROM funding_diffs \
 WHERE ($1::TEXT IS NULL OR lower(symbol) = lower($1)) \
   AND ($2::TEXT IS NULL OR lower(long_exchange) = lower($2)) \
   AND ($3::TEXT IS NULL OR lower(short_exchange) = lower($3)) \
   AND ($4::BIGINT IS NULL OR occurred_at_ms >= $4) \
   AND ($5::BIGINT IS NULL OR occurred_at_ms <= $5) \
 ORDER BY occurred_at_ms DESC \
 LIMIT $6";

pub(super) const OPPORTUNITY_QUERY: &str = "SELECT occurred_at_ms, id, symbol, long_exchange, short_exchange, spread_8h, net_yield, volume_24h_min, payload \
 FROM opportunities \
 WHERE ($1::TEXT IS NULL OR lower(symbol) = lower($1)) \
   AND ($2::DOUBLE PRECISION IS NULL OR net_yield >= $2) \
   AND ($3::BIGINT IS NULL OR occurred_at_ms >= $3) \
   AND ($4::BIGINT IS NULL OR occurred_at_ms <= $4) \
 ORDER BY occurred_at_ms DESC \
 LIMIT $5";

pub(super) const INDEX_COMPOSITION_QUERY: &str =
    "SELECT occurred_at_ms, venue, symbol, index_id, quality, component_count, source, payload \
 FROM index_compositions \
 WHERE ($1::TEXT IS NULL OR lower(venue) = lower($1)) \
   AND ($2::TEXT IS NULL OR lower(symbol) = lower($2)) \
   AND ($3::BIGINT IS NULL OR occurred_at_ms >= $3) \
   AND ($4::BIGINT IS NULL OR occurred_at_ms <= $4) \
 ORDER BY occurred_at_ms DESC \
 LIMIT $5";

pub(super) const API_HEALTH_QUERY: &str =
    "SELECT occurred_at_ms, exchange, endpoint, method, outcome, status_code, latency_ms, retry_after_ms, circuit_state, error_code, payload \
 FROM api_health \
 WHERE ($1::TEXT IS NULL OR lower(exchange) = lower($1)) \
   AND ($2::TEXT IS NULL OR lower(endpoint) = lower($2)) \
   AND ($3::TEXT IS NULL OR lower(outcome) = lower($3)) \
   AND ($4::BIGINT IS NULL OR occurred_at_ms >= $4) \
   AND ($5::BIGINT IS NULL OR occurred_at_ms <= $5) \
 ORDER BY occurred_at_ms DESC \
 LIMIT $6";

pub(super) const EVENT_QUERY: &str =
    "SELECT occurred_at_ms, event_id, category, action, actor, resource, outcome, severity, request_id, run_id, ticket_id, client_order_id, exchange_order_id, payload \
 FROM events \
 WHERE ($1::TEXT IS NULL OR lower(category) = lower($1)) \
   AND ($2::TEXT IS NULL OR lower(action) = lower($2)) \
   AND ($3::TEXT IS NULL OR request_id = $3) \
   AND ($4::TEXT IS NULL OR run_id = $4) \
   AND ($5::TEXT IS NULL OR ticket_id = $5) \
   AND ($6::TEXT IS NULL OR client_order_id = $6) \
   AND ($7::TEXT IS NULL OR exchange_order_id = $7) \
   AND ($8::BIGINT IS NULL OR occurred_at_ms >= $8) \
   AND ($9::BIGINT IS NULL OR occurred_at_ms <= $9) \
 ORDER BY occurred_at_ms DESC \
 LIMIT $10";
