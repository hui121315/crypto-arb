use super::*;

pub(super) struct FundingBatch {
    pub(super) occurred_at_ms: i64,
    pub(super) exchanges: Vec<String>,
    pub(super) symbols: Vec<String>,
    pub(super) rates: Vec<f64>,
    pub(super) interval_hours: Vec<i32>,
    pub(super) next_funding_ms: Vec<i64>,
    pub(super) volumes_24h: Vec<f64>,
}

pub(super) struct FundingDiffBatch {
    pub(super) occurred_at_ms: i64,
    pub(super) symbols: Vec<String>,
    pub(super) long_exchanges: Vec<String>,
    pub(super) short_exchanges: Vec<String>,
    pub(super) long_rate_8h: Vec<f64>,
    pub(super) short_rate_8h: Vec<f64>,
    pub(super) gross_diff_bps: Vec<f64>,
    pub(super) long_next_funding_ms: Vec<i64>,
    pub(super) short_next_funding_ms: Vec<i64>,
    pub(super) window_alignment_minutes: Vec<i32>,
    pub(super) long_interval_hours: Vec<i32>,
    pub(super) short_interval_hours: Vec<i32>,
    pub(super) min_volume_24h: Vec<f64>,
}

impl FundingDiffBatch {
    pub(super) fn from_rows(rows: &[FundingDiffRow]) -> Self {
        let occurred_at_ms = rows[0].occurred_at_ms;
        let mut batch = Self {
            occurred_at_ms,
            symbols: Vec::with_capacity(rows.len()),
            long_exchanges: Vec::with_capacity(rows.len()),
            short_exchanges: Vec::with_capacity(rows.len()),
            long_rate_8h: Vec::with_capacity(rows.len()),
            short_rate_8h: Vec::with_capacity(rows.len()),
            gross_diff_bps: Vec::with_capacity(rows.len()),
            long_next_funding_ms: Vec::with_capacity(rows.len()),
            short_next_funding_ms: Vec::with_capacity(rows.len()),
            window_alignment_minutes: Vec::with_capacity(rows.len()),
            long_interval_hours: Vec::with_capacity(rows.len()),
            short_interval_hours: Vec::with_capacity(rows.len()),
            min_volume_24h: Vec::with_capacity(rows.len()),
        };
        for row in rows {
            batch.symbols.push(row.symbol.clone());
            batch.long_exchanges.push(row.long_exchange.clone());
            batch.short_exchanges.push(row.short_exchange.clone());
            batch.long_rate_8h.push(row.long_rate_8h);
            batch.short_rate_8h.push(row.short_rate_8h);
            batch.gross_diff_bps.push(row.gross_diff_bps);
            batch.long_next_funding_ms.push(row.long_next_funding_ms);
            batch.short_next_funding_ms.push(row.short_next_funding_ms);
            batch
                .window_alignment_minutes
                .push(row.window_alignment_minutes);
            batch
                .long_interval_hours
                .push(row.long_interval_hours as i32);
            batch
                .short_interval_hours
                .push(row.short_interval_hours as i32);
            batch.min_volume_24h.push(row.min_volume_24h);
        }
        batch
    }
}

impl FundingBatch {
    pub(super) fn from_rows(rows: &[FundingRateData]) -> Self {
        let occurred_at_ms = common::time::now_ms();
        let mut batch = Self {
            occurred_at_ms,
            exchanges: Vec::with_capacity(rows.len()),
            symbols: Vec::with_capacity(rows.len()),
            rates: Vec::with_capacity(rows.len()),
            interval_hours: Vec::with_capacity(rows.len()),
            next_funding_ms: Vec::with_capacity(rows.len()),
            volumes_24h: Vec::with_capacity(rows.len()),
        };
        for rate in rows {
            batch.exchanges.push(rate.exchange.clone());
            batch.symbols.push(rate.symbol.clone());
            batch.rates.push(rate.rate);
            batch.interval_hours.push(rate.funding_interval as i32);
            batch.next_funding_ms.push(rate.next_funding_time);
            batch.volumes_24h.push(rate.volume_24h);
        }
        batch
    }
}

pub(super) struct OpportunityBatch {
    pub(super) occurred_at_ms: i64,
    pub(super) ids: Vec<String>,
    pub(super) symbols: Vec<String>,
    pub(super) long_exchanges: Vec<String>,
    pub(super) short_exchanges: Vec<String>,
    pub(super) spreads_8h: Vec<f64>,
    pub(super) net_yields: Vec<f64>,
    pub(super) volumes_24h_min: Vec<f64>,
    pub(super) payloads: Vec<serde_json::Value>,
}

pub(super) struct IndexCompositionBatch {
    pub(super) occurred_at_ms: i64,
    pub(super) venues: Vec<String>,
    pub(super) symbols: Vec<String>,
    pub(super) index_ids: Vec<String>,
    pub(super) qualities: Vec<String>,
    pub(super) component_counts: Vec<i32>,
    pub(super) sources: Vec<String>,
    pub(super) payloads: Vec<serde_json::Value>,
}

impl OpportunityBatch {
    pub(super) fn from_rows(rows: &[ArbitrageOpportunityDto]) -> Result<Self, HistoryError> {
        let occurred_at_ms = common::time::now_ms();
        let mut batch = Self {
            occurred_at_ms,
            ids: Vec::with_capacity(rows.len()),
            symbols: Vec::with_capacity(rows.len()),
            long_exchanges: Vec::with_capacity(rows.len()),
            short_exchanges: Vec::with_capacity(rows.len()),
            spreads_8h: Vec::with_capacity(rows.len()),
            net_yields: Vec::with_capacity(rows.len()),
            volumes_24h_min: Vec::with_capacity(rows.len()),
            payloads: Vec::with_capacity(rows.len()),
        };
        for opp in rows {
            batch.ids.push(opp.id.clone());
            batch.symbols.push(opp.symbol.clone());
            batch.long_exchanges.push(opp.long_exchange.clone());
            batch.short_exchanges.push(opp.short_exchange.clone());
            batch.spreads_8h.push(opp.spread_8h);
            batch.net_yields.push(opp.net_single_yield);
            batch.volumes_24h_min.push(opp.volume_24h);
            batch
                .payloads
                .push(serde_json::to_value(opp).map_err(|e| HistoryError::Encode(e.to_string()))?);
        }
        Ok(batch)
    }
}

impl IndexCompositionBatch {
    pub(super) fn from_rows(rows: &[IndexCompositionSnapshot]) -> Result<Self, HistoryError> {
        let occurred_at_ms = common::time::now_ms();
        let mut batch = Self {
            occurred_at_ms,
            venues: Vec::with_capacity(rows.len()),
            symbols: Vec::with_capacity(rows.len()),
            index_ids: Vec::with_capacity(rows.len()),
            qualities: Vec::with_capacity(rows.len()),
            component_counts: Vec::with_capacity(rows.len()),
            sources: Vec::with_capacity(rows.len()),
            payloads: Vec::with_capacity(rows.len()),
        };
        for row in rows {
            batch.venues.push(row.venue.clone());
            batch.symbols.push(row.symbol.clone());
            batch.index_ids.push(row.index_id.clone());
            batch.qualities.push(index_quality_label(row));
            batch.component_counts.push(row.components.len() as i32);
            batch.sources.push(row.source.clone());
            batch
                .payloads
                .push(serde_json::to_value(row).map_err(|e| HistoryError::Encode(e.to_string()))?);
        }
        Ok(batch)
    }
}

pub(super) fn index_quality_label(row: &IndexCompositionSnapshot) -> String {
    serde_json::to_value(row.quality)
        .ok()
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| format!("{:?}", row.quality))
}

pub(super) struct ApiHealthBatch {
    pub(super) occurred_at_ms: Vec<i64>,
    pub(super) exchanges: Vec<String>,
    pub(super) endpoints: Vec<String>,
    pub(super) methods: Vec<Option<String>>,
    pub(super) outcomes: Vec<String>,
    pub(super) status_codes: Vec<Option<i32>>,
    pub(super) latencies_ms: Vec<Option<f64>>,
    pub(super) retry_after_ms: Vec<Option<i64>>,
    pub(super) circuit_states: Vec<Option<String>>,
    pub(super) error_codes: Vec<Option<String>>,
    pub(super) payloads: Vec<serde_json::Value>,
}

impl ApiHealthBatch {
    pub(super) fn from_rows(rows: &[ApiHealthSampleRow]) -> Self {
        let mut batch = Self {
            occurred_at_ms: Vec::with_capacity(rows.len()),
            exchanges: Vec::with_capacity(rows.len()),
            endpoints: Vec::with_capacity(rows.len()),
            methods: Vec::with_capacity(rows.len()),
            outcomes: Vec::with_capacity(rows.len()),
            status_codes: Vec::with_capacity(rows.len()),
            latencies_ms: Vec::with_capacity(rows.len()),
            retry_after_ms: Vec::with_capacity(rows.len()),
            circuit_states: Vec::with_capacity(rows.len()),
            error_codes: Vec::with_capacity(rows.len()),
            payloads: Vec::with_capacity(rows.len()),
        };
        for row in rows {
            batch.occurred_at_ms.push(row.occurred_at_ms);
            batch.exchanges.push(row.exchange.clone());
            batch.endpoints.push(row.endpoint.clone());
            batch.methods.push(row.method.clone());
            batch.outcomes.push(row.outcome.clone());
            batch.status_codes.push(row.status_code);
            batch.latencies_ms.push(row.latency_ms);
            batch.retry_after_ms.push(row.retry_after_ms);
            batch.circuit_states.push(row.circuit_state.clone());
            batch.error_codes.push(row.error_code.clone());
            batch.payloads.push(payload_value(&row.payload));
        }
        batch
    }
}

pub(super) struct EventBatch {
    pub(super) occurred_at_ms: Vec<i64>,
    pub(super) event_ids: Vec<String>,
    pub(super) categories: Vec<String>,
    pub(super) actions: Vec<String>,
    pub(super) actors: Vec<Option<String>>,
    pub(super) resources: Vec<Option<String>>,
    pub(super) outcomes: Vec<String>,
    pub(super) severities: Vec<Option<String>>,
    pub(super) request_ids: Vec<Option<String>>,
    pub(super) run_ids: Vec<Option<String>>,
    pub(super) ticket_ids: Vec<Option<String>>,
    pub(super) client_order_ids: Vec<Option<String>>,
    pub(super) exchange_order_ids: Vec<Option<String>>,
    pub(super) payloads: Vec<serde_json::Value>,
}

impl EventBatch {
    pub(super) fn from_rows(rows: &[LedgerEventRow]) -> Self {
        let mut batch = Self {
            occurred_at_ms: Vec::with_capacity(rows.len()),
            event_ids: Vec::with_capacity(rows.len()),
            categories: Vec::with_capacity(rows.len()),
            actions: Vec::with_capacity(rows.len()),
            actors: Vec::with_capacity(rows.len()),
            resources: Vec::with_capacity(rows.len()),
            outcomes: Vec::with_capacity(rows.len()),
            severities: Vec::with_capacity(rows.len()),
            request_ids: Vec::with_capacity(rows.len()),
            run_ids: Vec::with_capacity(rows.len()),
            ticket_ids: Vec::with_capacity(rows.len()),
            client_order_ids: Vec::with_capacity(rows.len()),
            exchange_order_ids: Vec::with_capacity(rows.len()),
            payloads: Vec::with_capacity(rows.len()),
        };
        for row in rows {
            batch.occurred_at_ms.push(row.occurred_at_ms);
            batch.event_ids.push(row.event_id.clone());
            batch.categories.push(row.category.clone());
            batch.actions.push(row.action.clone());
            batch.actors.push(row.actor.clone());
            batch.resources.push(row.resource.clone());
            batch.outcomes.push(row.outcome.clone());
            batch.severities.push(row.severity.clone());
            batch.request_ids.push(row.request_id.clone());
            batch.run_ids.push(row.run_id.clone());
            batch.ticket_ids.push(row.ticket_id.clone());
            batch.client_order_ids.push(row.client_order_id.clone());
            batch.exchange_order_ids.push(row.exchange_order_id.clone());
            batch.payloads.push(payload_value(&row.payload));
        }
        batch
    }
}

/// Normalise an optional JSON payload to a non-null value so the `JSONB[]`
/// bind never sends SQL `NULL` into a `NOT NULL DEFAULT '{}'` column.
pub(super) fn payload_value(payload: &serde_json::Value) -> serde_json::Value {
    if payload.is_null() {
        serde_json::json!({})
    } else {
        payload.clone()
    }
}
