use super::*;

pub(super) fn funding_from_row(row: &Row) -> FundingRow {
    let interval_hours: i32 = row.get("interval_hours");
    let volume_24h: Option<f64> = row.get("volume_24h");
    FundingRow {
        occurred_at_ms: row.get("occurred_at_ms"),
        exchange: row.get("exchange"),
        symbol: row.get("symbol"),
        rate: row.get("rate"),
        interval_hours: interval_hours.max(0) as u32,
        next_funding_ms: row.get("next_funding_ms"),
        volume_24h: volume_24h.unwrap_or_default(),
    }
}

pub(super) fn funding_diff_from_row(row: &Row) -> FundingDiffRow {
    let window_alignment_minutes: Option<i32> = row.get("window_alignment_minutes");
    let long_interval_hours: Option<i32> = row.get("long_interval_hours");
    let short_interval_hours: Option<i32> = row.get("short_interval_hours");
    let min_volume_24h: Option<f64> = row.get("min_volume_24h");
    FundingDiffRow {
        occurred_at_ms: row.get("occurred_at_ms"),
        symbol: row.get("symbol"),
        long_exchange: row.get("long_exchange"),
        short_exchange: row.get("short_exchange"),
        long_rate_8h: row.get("long_rate_8h"),
        short_rate_8h: row.get("short_rate_8h"),
        gross_diff_bps: row.get("gross_diff_bps"),
        long_next_funding_ms: row.get("long_next_funding_ms"),
        short_next_funding_ms: row.get("short_next_funding_ms"),
        window_alignment_minutes: window_alignment_minutes.unwrap_or_default(),
        long_interval_hours: long_interval_hours.unwrap_or_default().max(0) as u32,
        short_interval_hours: short_interval_hours.unwrap_or_default().max(0) as u32,
        min_volume_24h: min_volume_24h.unwrap_or_default(),
    }
}

pub(super) fn opportunity_from_row(row: &Row) -> Result<OpportunityRow, HistoryError> {
    let payload: serde_json::Value = row.get("payload");
    let payload: ArbitrageOpportunityDto =
        serde_json::from_value(payload).map_err(|e| HistoryError::Decode(e.to_string()))?;
    let spread_8h: Option<f64> = row.get("spread_8h");
    let net_yield: Option<f64> = row.get("net_yield");
    let volume_24h_min: Option<f64> = row.get("volume_24h_min");
    Ok(OpportunityRow {
        occurred_at_ms: row.get("occurred_at_ms"),
        id: row.get("id"),
        symbol: row.get("symbol"),
        long_exchange: row.get("long_exchange"),
        short_exchange: row.get("short_exchange"),
        spread_8h: spread_8h.unwrap_or_default(),
        net_yield: net_yield.unwrap_or_default(),
        volume_24h_min: volume_24h_min.unwrap_or_default(),
        payload,
    })
}

pub(super) fn api_health_from_row(row: &Row) -> ApiHealthSampleRow {
    ApiHealthSampleRow {
        occurred_at_ms: row.get("occurred_at_ms"),
        exchange: row.get("exchange"),
        endpoint: row.get("endpoint"),
        method: row.get("method"),
        outcome: row.get("outcome"),
        status_code: row.get("status_code"),
        latency_ms: row.get("latency_ms"),
        retry_after_ms: row.get("retry_after_ms"),
        circuit_state: row.get("circuit_state"),
        error_code: row.get("error_code"),
        payload: row.get("payload"),
    }
}

pub(super) fn event_from_row(row: &Row) -> LedgerEventRow {
    LedgerEventRow {
        occurred_at_ms: row.get("occurred_at_ms"),
        event_id: row.get("event_id"),
        category: row.get("category"),
        action: row.get("action"),
        actor: row.get("actor"),
        resource: row.get("resource"),
        outcome: row.get("outcome"),
        severity: row.get("severity"),
        request_id: row.get("request_id"),
        run_id: row.get("run_id"),
        ticket_id: row.get("ticket_id"),
        client_order_id: row.get("client_order_id"),
        exchange_order_id: row.get("exchange_order_id"),
        payload: row.get("payload"),
    }
}

pub(super) fn index_composition_from_row(
    row: &Row,
) -> Result<IndexCompositionHistoryRow, HistoryError> {
    let payload: serde_json::Value = row.get("payload");
    let payload: IndexCompositionSnapshot =
        serde_json::from_value(payload).map_err(|e| HistoryError::Decode(e.to_string()))?;
    let component_count: i32 = row.get("component_count");
    Ok(IndexCompositionHistoryRow {
        occurred_at_ms: row.get("occurred_at_ms"),
        venue: row.get("venue"),
        symbol: row.get("symbol"),
        index_id: row.get("index_id"),
        quality: payload.quality,
        component_count: component_count.max(0) as usize,
        source: row.get("source"),
        payload,
    })
}
