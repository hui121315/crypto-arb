//! Gate futures account fee-rate evidence and bounded cache.

use crate::error::{ExchangeError, ExchangeResult};
use dashmap::DashMap;
use serde::Deserialize;
use std::collections::HashMap;

pub(super) const GATE_FUTURES_FEE_DOC_URL: &str =
    "https://www.gate.com/docs/developers/apiv4/en/futures/#query-futures-market-trading-fee-rates";
const FEE_EVIDENCE_TTL_MS: i64 = 5 * 60 * 1_000;

pub(super) type GateFuturesFeeResponse = HashMap<String, GateFuturesFeeRow>;

#[derive(Debug, Deserialize)]
pub(super) struct GateFuturesFeeRow {
    maker_fee: serde_json::Value,
    taker_fee: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct GateFuturesFeeEvidence {
    pub(super) settle: String,
    pub(super) contract: String,
    pub(super) maker_fee_rate: f64,
    pub(super) taker_fee_rate: f64,
    pub(super) fetched_at_ms: i64,
    pub(super) valid_until_ms: i64,
    pub(super) source_url: &'static str,
}

#[derive(Debug, Default)]
pub(super) struct GateFuturesFeeCache {
    rows: DashMap<(String, String), GateFuturesFeeEvidence>,
}

impl GateFuturesFeeCache {
    pub(super) fn replace(&self, rows: Vec<GateFuturesFeeEvidence>) {
        let settle = rows.first().map(|row| row.settle.clone());
        if let Some(settle) = settle.as_deref() {
            self.rows
                .retain(|(cached_settle, _), _| cached_settle != settle);
        }
        for row in rows {
            self.rows
                .insert((row.settle.clone(), row.contract.clone()), row);
        }
    }

    pub(super) fn fresh(
        &self,
        settle: &str,
        contract: &str,
        now_ms: i64,
    ) -> Option<GateFuturesFeeEvidence> {
        let key = (
            settle.trim().to_ascii_uppercase(),
            contract.trim().to_ascii_uppercase(),
        );
        self.rows
            .get(&key)
            .filter(|row| row.valid_until_ms > now_ms)
            .map(|row| row.clone())
    }
}

pub(super) fn parse_futures_fee_evidence(
    rows: GateFuturesFeeResponse,
    settle: &str,
    fetched_at_ms: i64,
) -> ExchangeResult<Vec<GateFuturesFeeEvidence>> {
    let settle = verified_settle(settle)?;
    if fetched_at_ms <= 0 {
        return Err(ExchangeError::Parse(
            "gate fee evidence requires positive fetched_at_ms".into(),
        ));
    }
    rows.into_iter()
        .map(|(contract, row)| parse_fee_row(&contract, &row, &settle, fetched_at_ms))
        .collect()
}

fn parse_fee_row(
    contract: &str,
    row: &GateFuturesFeeRow,
    settle: &str,
    fetched_at_ms: i64,
) -> ExchangeResult<GateFuturesFeeEvidence> {
    let contract = contract.trim().to_ascii_uppercase();
    if contract.is_empty() {
        return Err(ExchangeError::Parse(
            "gate fee evidence contract is empty".into(),
        ));
    }
    Ok(GateFuturesFeeEvidence {
        settle: settle.to_owned(),
        contract,
        maker_fee_rate: signed_rate(&row.maker_fee, "fee.maker_fee")?,
        taker_fee_rate: signed_rate(&row.taker_fee, "fee.taker_fee")?,
        fetched_at_ms,
        valid_until_ms: fetched_at_ms.saturating_add(FEE_EVIDENCE_TTL_MS),
        source_url: GATE_FUTURES_FEE_DOC_URL,
    })
}

fn verified_settle(settle: &str) -> ExchangeResult<String> {
    match settle.trim().to_ascii_lowercase().as_str() {
        "usdt" => Ok("USDT".into()),
        "btc" => Ok("BTC".into()),
        other => Err(ExchangeError::Parse(format!(
            "gate fee unsupported settle evidence: {other}"
        ))),
    }
}

fn signed_rate(value: &serde_json::Value, field: &str) -> ExchangeResult<f64> {
    let rate = match value {
        serde_json::Value::Number(number) => number.as_f64(),
        serde_json::Value::String(text) => text.parse::<f64>().ok(),
        _ => None,
    }
    .filter(|number| number.is_finite())
    .ok_or_else(|| ExchangeError::Parse(format!("gate {field} invalid rate: {value}")))?;
    if rate.abs() > 1.0 {
        return Err(ExchangeError::Parse(format!(
            "gate {field} outside decimal rate bounds: {rate}"
        )));
    }
    Ok(rate)
}

#[cfg(test)]
#[path = "gate_fee_evidence_tests.rs"]
mod tests;
