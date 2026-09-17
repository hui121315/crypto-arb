use crate::middleware::audit;
use crate::services::market_data;
use crate::services::venue_quality;
use crate::state::AppState;
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::routing::get;
use axum::{Json, Router};
use common::AppError;
use serde::Deserialize;
use serde_json::json;
use shared_types::{
    IndexCompositionListEnvelope, IndexCompositionSnapshot, InstrumentCoverageDiagnostic,
    MarketDataEnvelope, VenueQualityEnvelope,
};

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/venues/quality", get(snapshot))
        .route("/api/venues/instrument-coverage", get(instrument_coverage))
        .route("/api/trading/venues/quality", get(snapshot))
        .route(
            "/api/venues/index-compositions",
            get(index_compositions).post(upsert_index_compositions),
        )
        .route(
            "/api/venues/index-compositions/envelope",
            get(index_compositions_envelope),
        )
        .route(
            "/api/venues/index-compositions/fetch",
            get(fetch_index_composition),
        )
}

#[derive(Debug, Deserialize)]
struct InstrumentCoverageQuery {
    #[serde(default)]
    symbol: Option<String>,
}

async fn instrument_coverage(
    State(state): State<AppState>,
    Query(query): Query<InstrumentCoverageQuery>,
) -> Result<Json<InstrumentCoverageDiagnostic>, AppError> {
    let symbol = exchange::strip_common_suffixes(&required_query_text(
        query.symbol.as_deref(),
        "symbol is required",
    )?);
    Ok(Json(
        state
            .instrument_registry()
            .coverage_diagnostic(&symbol, common::time::now_ms()),
    ))
}

async fn snapshot(State(state): State<AppState>) -> Json<VenueQualityEnvelope> {
    Json(venue_quality::snapshot_envelope(&state))
}

async fn index_compositions(State(state): State<AppState>) -> Json<Vec<IndexCompositionSnapshot>> {
    Json(state.market_data().index_compositions_snapshot())
}

async fn index_compositions_envelope(
    State(state): State<AppState>,
) -> Json<IndexCompositionListEnvelope> {
    let now_ms = common::time::now_ms();
    Json(market_data::envelope::index_composition_list_envelope(
        state.market_data().index_compositions_snapshot(),
        now_ms,
    ))
}

async fn upsert_index_compositions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(rows): Json<Vec<IndexCompositionSnapshot>>,
) -> Result<Json<Vec<IndexCompositionSnapshot>>, AppError> {
    state.market_data().store_index_compositions(
        &rows,
        crate::services::market_data::MarketSource::RestBaseline,
    );
    state
        .history_store()
        .append_index_compositions(&rows)
        .await?;
    audit_index_composition_upsert(&headers, &rows);
    Ok(Json(rows))
}

#[derive(Debug, Deserialize)]
struct IndexCompositionQuery {
    #[serde(default)]
    venue: Option<String>,
    #[serde(default)]
    symbol: Option<String>,
}

async fn fetch_index_composition(
    State(state): State<AppState>,
    Query(query): Query<IndexCompositionQuery>,
) -> Result<Json<MarketDataEnvelope<Option<IndexCompositionSnapshot>>>, AppError> {
    let venue = required_query_text(query.venue.as_deref(), "venue and symbol are required")?;
    let symbol = required_query_text(query.symbol.as_deref(), "venue and symbol are required")?;
    let now_ms = common::time::now_ms();
    let read = state
        .market_data()
        .index_composition_or_fetch(state.aggregator(), &venue, &symbol, now_ms)
        .await;
    let envelope = market_data::envelope::index_composition_envelope(read, &venue, &symbol, now_ms);
    if let Some(snapshot) = envelope.data.as_ref() {
        state
            .history_store()
            .append_index_compositions(std::slice::from_ref(snapshot))
            .await?;
    }
    Ok(Json(envelope))
}

fn required_query_text(value: Option<&str>, message: &'static str) -> Result<String, AppError> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| AppError::BadRequest(message.into()))
}

fn audit_index_composition_upsert(headers: &HeaderMap, rows: &[IndexCompositionSnapshot]) {
    audit::record_http_event(
        headers,
        "index_composition.upsert",
        "index-compositions",
        "success",
        json!({
            "rows": rows.len(),
            "keys": index_composition_keys(rows),
        }),
    );
}

fn index_composition_keys(rows: &[IndexCompositionSnapshot]) -> Vec<String> {
    rows.iter()
        .take(20)
        .map(|row| format!("{}:{}", row.venue, row.symbol))
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::panic)]

    use super::*;
    use crate::services::market_data::{MarketQuality, MarketRead, MarketSource};

    #[test]
    fn index_composition_failure_returns_market_envelope_problem() {
        let envelope = market_data::envelope::index_composition_envelope(
            MarketRead {
                value: None,
                quality: MarketQuality::RateLimited,
                freshness_ms: None,
                source: MarketSource::LocalCache,
                retry_after_ms: Some(2_000),
                last_error: Some("rate limited".into()),
            },
            "okx",
            "BTC-USD",
            1_000,
        );

        assert!(envelope.data.is_none());
        assert_eq!(envelope.health.retry_after_ms, Some(2_000));
        assert_eq!(
            envelope
                .health
                .problem
                .as_ref()
                .map(|problem| problem.code.as_str()),
            Some("MARKET_DATA_RATE_LIMITED")
        );
    }

    #[test]
    fn instrument_coverage_query_normalizes_contract_symbol() {
        assert_eq!(exchange::strip_common_suffixes("BTC-USDT"), "BTC");
        assert_eq!(exchange::strip_common_suffixes("ETHUSDT"), "ETH");
    }

    #[test]
    fn required_query_text_uses_typed_bad_request_for_missing_values() {
        for message in ["symbol is required", "venue and symbol are required"] {
            let result = required_query_text(None, message);
            let error = match result {
                Err(error) => error,
                Ok(value) => panic!("missing query input unexpectedly resolved as {value}"),
            };

            assert_eq!(error.code(), "BAD_REQUEST");
            assert_eq!(error.status().as_u16(), 400);
        }
    }

    #[tokio::test]
    async fn instrument_coverage_handler_returns_structured_diagnostic() -> anyhow::Result<()> {
        let state = AppState::new(common::config::AppConfig::default()).await?;

        let Json(diagnostic) = instrument_coverage(
            State(state),
            Query(InstrumentCoverageQuery {
                symbol: Some(" BTC-USDT ".into()),
            }),
        )
        .await?;

        assert_eq!(diagnostic.canonical_symbol, "BTC");
        assert_eq!(
            diagnostic.venue_count,
            crate::services::instrument_registry::SUPPORTED_VENUES.len()
        );
        assert!(diagnostic
            .diagnostics_text
            .starts_with(&format!("BTC 规格就绪 0/{}", diagnostic.venue_count)));
        Ok(())
    }
}
