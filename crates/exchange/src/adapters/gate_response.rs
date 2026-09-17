//! Gate REST response parse adapters.

use crate::error::ExchangeError;

pub(super) fn parse_err(error: &reqwest::Error) -> ExchangeError {
    ExchangeError::Parse(format!("gate json: {error}"))
}
