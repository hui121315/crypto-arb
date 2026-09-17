use super::*;

impl PostgresHistoryStore {
    pub(in crate::history) async fn append_funding_rates(
        &self,
        rows: &[FundingRateData],
    ) -> Result<(), HistoryError> {
        if rows.is_empty() {
            return Ok(());
        }
        let batch = FundingBatch::from_rows(rows);
        self.pool.client().await?
            .execute(
                "INSERT INTO funding_rates \
                 (occurred_at_ms, exchange, symbol, rate, interval_hours, next_funding_ms, volume_24h) \
                 SELECT $1, exchange, symbol, rate, interval_hours, next_funding_ms, volume_24h \
                 FROM unnest($2::TEXT[], $3::TEXT[], $4::DOUBLE PRECISION[], $5::INTEGER[], $6::BIGINT[], $7::DOUBLE PRECISION[]) \
                 AS t(exchange, symbol, rate, interval_hours, next_funding_ms, volume_24h)",
                &[
                    &batch.occurred_at_ms,
                    &batch.exchanges,
                    &batch.symbols,
                    &batch.rates,
                    &batch.interval_hours,
                    &batch.next_funding_ms,
                    &batch.volumes_24h,
                ],
            )
            .await
            .map_err(|error| postgres_history_error(&error))?;
        Ok(())
    }

    pub(in crate::history) async fn append_funding_diffs(
        &self,
        rows: &[FundingDiffRow],
    ) -> Result<(), HistoryError> {
        if rows.is_empty() {
            return Ok(());
        }
        let batch = FundingDiffBatch::from_rows(rows);
        self.pool.client().await?
            .execute(
                "INSERT INTO funding_diffs \
                 (occurred_at_ms, symbol, long_exchange, short_exchange, long_rate_8h, short_rate_8h, gross_diff_bps, long_next_funding_ms, short_next_funding_ms, window_alignment_minutes, long_interval_hours, short_interval_hours, min_volume_24h) \
                 SELECT $1, symbol, long_exchange, short_exchange, long_rate_8h, short_rate_8h, gross_diff_bps, long_next_funding_ms, short_next_funding_ms, window_alignment_minutes, long_interval_hours, short_interval_hours, min_volume_24h \
                 FROM unnest($2::TEXT[], $3::TEXT[], $4::TEXT[], $5::DOUBLE PRECISION[], $6::DOUBLE PRECISION[], $7::DOUBLE PRECISION[], $8::BIGINT[], $9::BIGINT[], $10::INTEGER[], $11::INTEGER[], $12::INTEGER[], $13::DOUBLE PRECISION[]) \
                 AS t(symbol, long_exchange, short_exchange, long_rate_8h, short_rate_8h, gross_diff_bps, long_next_funding_ms, short_next_funding_ms, window_alignment_minutes, long_interval_hours, short_interval_hours, min_volume_24h)",
                &[
                    &batch.occurred_at_ms,
                    &batch.symbols,
                    &batch.long_exchanges,
                    &batch.short_exchanges,
                    &batch.long_rate_8h,
                    &batch.short_rate_8h,
                    &batch.gross_diff_bps,
                    &batch.long_next_funding_ms,
                    &batch.short_next_funding_ms,
                    &batch.window_alignment_minutes,
                    &batch.long_interval_hours,
                    &batch.short_interval_hours,
                    &batch.min_volume_24h,
                ],
            )
            .await
            .map_err(|error| postgres_history_error(&error))?;
        Ok(())
    }

    pub(in crate::history) async fn append_opportunities(
        &self,
        rows: &[ArbitrageOpportunityDto],
    ) -> Result<(), HistoryError> {
        if rows.is_empty() {
            return Ok(());
        }
        let batch = OpportunityBatch::from_rows(rows)?;
        self.pool.client().await?
            .execute(
                "INSERT INTO opportunities \
                 (occurred_at_ms, id, symbol, long_exchange, short_exchange, spread_8h, net_yield, volume_24h_min, payload) \
                 SELECT $1, id, symbol, long_exchange, short_exchange, spread_8h, net_yield, volume_24h_min, payload \
                 FROM unnest($2::TEXT[], $3::TEXT[], $4::TEXT[], $5::TEXT[], $6::DOUBLE PRECISION[], $7::DOUBLE PRECISION[], $8::DOUBLE PRECISION[], $9::JSONB[]) \
                 AS t(id, symbol, long_exchange, short_exchange, spread_8h, net_yield, volume_24h_min, payload)",
                &[
                    &batch.occurred_at_ms,
                    &batch.ids,
                    &batch.symbols,
                    &batch.long_exchanges,
                    &batch.short_exchanges,
                    &batch.spreads_8h,
                    &batch.net_yields,
                    &batch.volumes_24h_min,
                    &batch.payloads,
                ],
            )
            .await
            .map_err(|error| postgres_history_error(&error))?;
        Ok(())
    }

    pub(in crate::history) async fn append_index_compositions(
        &self,
        rows: &[IndexCompositionSnapshot],
    ) -> Result<(), HistoryError> {
        if rows.is_empty() {
            return Ok(());
        }
        let batch = IndexCompositionBatch::from_rows(rows)?;
        self.pool.client().await?
            .execute(
                "INSERT INTO index_compositions \
                 (occurred_at_ms, venue, symbol, index_id, quality, component_count, source, payload) \
                 SELECT $1, venue, symbol, index_id, quality, component_count, source, payload \
                 FROM unnest($2::TEXT[], $3::TEXT[], $4::TEXT[], $5::TEXT[], $6::INTEGER[], $7::TEXT[], $8::JSONB[]) \
                 AS t(venue, symbol, index_id, quality, component_count, source, payload)",
                &[
                    &batch.occurred_at_ms,
                    &batch.venues,
                    &batch.symbols,
                    &batch.index_ids,
                    &batch.qualities,
                    &batch.component_counts,
                    &batch.sources,
                    &batch.payloads,
                ],
            )
            .await
            .map_err(|error| postgres_history_error(&error))?;
        Ok(())
    }

    pub(in crate::history) async fn append_api_health(
        &self,
        rows: &[ApiHealthSampleRow],
    ) -> Result<(), HistoryError> {
        if rows.is_empty() {
            return Ok(());
        }
        let batch = ApiHealthBatch::from_rows(rows);
        self.pool.client().await?
            .execute(
                "INSERT INTO api_health \
                 (occurred_at_ms, exchange, endpoint, method, outcome, status_code, latency_ms, retry_after_ms, circuit_state, error_code, payload) \
                 SELECT occurred_at_ms, exchange, endpoint, method, outcome, status_code, latency_ms, retry_after_ms, circuit_state, error_code, payload \
                 FROM unnest($1::BIGINT[], $2::TEXT[], $3::TEXT[], $4::TEXT[], $5::TEXT[], $6::INTEGER[], $7::DOUBLE PRECISION[], $8::BIGINT[], $9::TEXT[], $10::TEXT[], $11::JSONB[]) \
                 AS t(occurred_at_ms, exchange, endpoint, method, outcome, status_code, latency_ms, retry_after_ms, circuit_state, error_code, payload)",
                &[
                    &batch.occurred_at_ms,
                    &batch.exchanges,
                    &batch.endpoints,
                    &batch.methods,
                    &batch.outcomes,
                    &batch.status_codes,
                    &batch.latencies_ms,
                    &batch.retry_after_ms,
                    &batch.circuit_states,
                    &batch.error_codes,
                    &batch.payloads,
                ],
            )
            .await
            .map_err(|error| postgres_history_error(&error))?;
        Ok(())
    }

    pub(in crate::history) async fn append_events(
        &self,
        rows: &[LedgerEventRow],
    ) -> Result<(), HistoryError> {
        if rows.is_empty() {
            return Ok(());
        }
        let batch = EventBatch::from_rows(rows);
        self.pool.client().await?
            .execute(
                "INSERT INTO events \
                 (occurred_at_ms, event_id, category, action, actor, resource, outcome, severity, request_id, run_id, ticket_id, client_order_id, exchange_order_id, payload) \
                 SELECT occurred_at_ms, event_id, category, action, actor, resource, outcome, severity, request_id, run_id, ticket_id, client_order_id, exchange_order_id, payload \
                 FROM unnest($1::BIGINT[], $2::TEXT[], $3::TEXT[], $4::TEXT[], $5::TEXT[], $6::TEXT[], $7::TEXT[], $8::TEXT[], $9::TEXT[], $10::TEXT[], $11::TEXT[], $12::TEXT[], $13::TEXT[], $14::JSONB[]) \
                 AS t(occurred_at_ms, event_id, category, action, actor, resource, outcome, severity, request_id, run_id, ticket_id, client_order_id, exchange_order_id, payload)",
                &[
                    &batch.occurred_at_ms,
                    &batch.event_ids,
                    &batch.categories,
                    &batch.actions,
                    &batch.actors,
                    &batch.resources,
                    &batch.outcomes,
                    &batch.severities,
                    &batch.request_ids,
                    &batch.run_ids,
                    &batch.ticket_ids,
                    &batch.client_order_ids,
                    &batch.exchange_order_ids,
                    &batch.payloads,
                ],
            )
            .await
            .map_err(|error| postgres_history_error(&error))?;
        Ok(())
    }
}
