//! Binance USD-M account commission-rate evidence.

use crate::error::{ExchangeError, ExchangeResult};
use serde::Deserialize;

pub(super) const BINANCE_COMMISSION_RATE_PATH: &str = "/fapi/v1/commissionRate";
pub(super) const BINANCE_COMMISSION_RATE_DOC_URL: &str =
    "https://developers.binance.com/en/docs/catalog/core-trading-derivatives-trading-usd-s-m-futures/api/rest-api/account#user-commission-rate";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct BinanceCommissionRateResponse {
    symbol: String,
    maker_commission_rate: String,
    taker_commission_rate: String,
    rpi_commission_rate: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct BinanceCommissionRateEvidence {
    pub(super) symbol: String,
    pub(super) maker_commission_rate: f64,
    pub(super) taker_commission_rate: f64,
    pub(super) rpi_commission_rate: f64,
    pub(super) fetched_at_ms: i64,
    pub(super) source_url: &'static str,
}

pub(super) fn parse_commission_rate(
    row: &BinanceCommissionRateResponse,
    expected_symbol: &str,
    fetched_at_ms: i64,
) -> ExchangeResult<BinanceCommissionRateEvidence> {
    let expected_symbol = normalized_symbol(expected_symbol, "requested")?;
    let symbol = normalized_symbol(&row.symbol, "response")?;
    if symbol != expected_symbol {
        return Err(ExchangeError::Parse(format!(
            "binance commissionRate symbol mismatch: requested={expected_symbol} response={symbol}"
        )));
    }
    if fetched_at_ms <= 0 {
        return Err(ExchangeError::Parse(
            "binance commissionRate requires positive fetched_at_ms".into(),
        ));
    }
    Ok(BinanceCommissionRateEvidence {
        symbol,
        maker_commission_rate: signed_rate(&row.maker_commission_rate, "makerCommissionRate")?,
        taker_commission_rate: signed_rate(&row.taker_commission_rate, "takerCommissionRate")?,
        rpi_commission_rate: signed_rate(&row.rpi_commission_rate, "rpiCommissionRate")?,
        fetched_at_ms,
        source_url: BINANCE_COMMISSION_RATE_DOC_URL,
    })
}

fn normalized_symbol(value: &str, scope: &str) -> ExchangeResult<String> {
    let symbol = value.trim().to_ascii_uppercase();
    if symbol.is_empty() || !symbol.chars().all(|ch| ch.is_ascii_alphanumeric()) {
        return Err(ExchangeError::Parse(format!(
            "binance commissionRate {scope} symbol invalid: {value:?}"
        )));
    }
    Ok(symbol)
}

fn signed_rate(value: &str, field: &str) -> ExchangeResult<f64> {
    let rate = value.parse::<f64>().map_err(|_| {
        ExchangeError::Parse(format!("binance commissionRate {field} invalid: {value:?}"))
    })?;
    if !rate.is_finite() || rate.abs() > 1.0 {
        return Err(ExchangeError::Parse(format!(
            "binance commissionRate {field} outside decimal rate bounds: {value:?}"
        )));
    }
    Ok(rate)
}

#[cfg(test)]
#[path = "binance_fee_evidence_tests.rs"]
mod tests;
