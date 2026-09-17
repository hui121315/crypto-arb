use super::query::latest_matching_rows;
use super::{
    in_time_range, match_opt, trim_to_max, HistoryError, OpportunityQuery, OpportunityRow,
};
use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;
use flate2::Compression;
use serde::de::DeserializeOwned;
use serde::Serialize;
use shared_types::ArbitrageOpportunityDto;
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone)]
pub(super) struct OpportunityMemory {
    rows: Arc<RwLock<VecDeque<CompressedOpportunityRow>>>,
    max_rows: usize,
}

impl OpportunityMemory {
    pub(super) fn new(max_rows: usize) -> Self {
        Self {
            rows: Arc::new(RwLock::new(VecDeque::new())),
            max_rows,
        }
    }

    pub(super) async fn append(
        &self,
        rows: &[ArbitrageOpportunityDto],
    ) -> Result<(), HistoryError> {
        if rows.is_empty() {
            return Ok(());
        }
        let rows = rows.to_vec();
        let occurred_at_ms = common::time::now_ms();
        let encoded = tokio::task::spawn_blocking(move || encode_rows(rows, occurred_at_ms))
            .await
            .map_err(|error| HistoryError::Encode(format!("compression task failed: {error}")))??;
        let mut stored = self.rows.write().await;
        stored.extend(encoded);
        trim_to_max(&mut stored, self.max_rows);
        Ok(())
    }

    pub(super) async fn query(
        &self,
        query: OpportunityQuery,
    ) -> Result<Vec<OpportunityRow>, HistoryError> {
        let selected = {
            let rows = self.rows.read().await;
            latest_matching_rows(
                &rows,
                query.limit,
                |row| row.matches(&query),
                |row| row.occurred_at_ms,
            )
        };
        tokio::task::spawn_blocking(move || {
            selected
                .into_iter()
                .map(CompressedOpportunityRow::decode)
                .collect()
        })
        .await
        .map_err(|error| HistoryError::Decode(format!("decompression task failed: {error}")))?
    }

    pub(super) async fn row_count(&self) -> usize {
        self.rows.read().await.len()
    }
}

#[derive(Debug, Clone)]
struct CompressedOpportunityRow {
    occurred_at_ms: i64,
    id: String,
    symbol: String,
    long_exchange: String,
    short_exchange: String,
    spread_8h: f64,
    net_yield: f64,
    volume_24h_min: f64,
    payload: Vec<u8>,
}

impl CompressedOpportunityRow {
    fn encode(opp: ArbitrageOpportunityDto, occurred_at_ms: i64) -> Result<Self, HistoryError> {
        let payload = deflate_json(&opp)?;
        Ok(Self {
            occurred_at_ms,
            id: opp.id,
            symbol: opp.symbol,
            long_exchange: opp.long_exchange,
            short_exchange: opp.short_exchange,
            spread_8h: opp.spread_8h,
            net_yield: opp.net_single_yield,
            volume_24h_min: opp.volume_24h,
            payload,
        })
    }

    fn matches(&self, query: &OpportunityQuery) -> bool {
        match_opt(&query.symbol, &self.symbol)
            && query
                .min_yield
                .map(|min| self.net_yield >= min)
                .unwrap_or(true)
            && in_time_range(self.occurred_at_ms, query.from_ms, query.to_ms)
    }

    fn decode(self) -> Result<OpportunityRow, HistoryError> {
        Ok(OpportunityRow {
            occurred_at_ms: self.occurred_at_ms,
            id: self.id,
            symbol: self.symbol,
            long_exchange: self.long_exchange,
            short_exchange: self.short_exchange,
            spread_8h: self.spread_8h,
            net_yield: self.net_yield,
            volume_24h_min: self.volume_24h_min,
            payload: inflate_json(&self.payload)?,
        })
    }
}

fn encode_rows(
    rows: Vec<ArbitrageOpportunityDto>,
    occurred_at_ms: i64,
) -> Result<Vec<CompressedOpportunityRow>, HistoryError> {
    rows.into_iter()
        .map(|row| CompressedOpportunityRow::encode(row, occurred_at_ms))
        .collect()
}

fn deflate_json<T: Serialize>(value: &T) -> Result<Vec<u8>, HistoryError> {
    let json =
        serde_json::to_vec(value).map_err(|error| HistoryError::Encode(error.to_string()))?;
    let mut encoder = DeflateEncoder::new(Vec::with_capacity(json.len() / 2), Compression::fast());
    encoder
        .write_all(&json)
        .map_err(|error| HistoryError::Encode(error.to_string()))?;
    encoder
        .finish()
        .map_err(|error| HistoryError::Encode(error.to_string()))
}

fn inflate_json<T: DeserializeOwned>(payload: &[u8]) -> Result<T, HistoryError> {
    let mut decoder = DeflateDecoder::new(payload);
    let mut json = Vec::new();
    decoder
        .read_to_end(&mut json)
        .map_err(|error| HistoryError::Decode(error.to_string()))?;
    serde_json::from_slice(&json).map_err(|error| HistoryError::Decode(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deflate_round_trip_reduces_repeated_json() -> Result<(), HistoryError> {
        let value = serde_json::json!({
            "symbol": "BTC",
            "evidence": "verified-market-evidence".repeat(256),
        });
        let plain =
            serde_json::to_vec(&value).map_err(|error| HistoryError::Encode(error.to_string()))?;
        let compressed = deflate_json(&value)?;
        let decoded: serde_json::Value = inflate_json(&compressed)?;

        assert_eq!(decoded, value);
        assert!(compressed.len() < plain.len() / 2);
        Ok(())
    }
}
