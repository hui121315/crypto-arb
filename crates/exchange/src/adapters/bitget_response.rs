//! Bitget REST response envelopes (V2 and V3 / UTA share this shape).

use crate::error::{ExchangeError, ExchangeResult};
use serde::Deserialize;

const NAME: &str = "bitget";
const SUCCESS_CODE: &str = "00000";

#[derive(Debug, Deserialize)]
#[serde(bound(deserialize = "T: serde::de::DeserializeOwned"))]
pub(super) struct BitgetResponse<T> {
    code: String,
    #[serde(default)]
    msg: String,
    #[serde(default = "Vec::new")]
    data: Vec<T>,
}

impl<T> BitgetResponse<T> {
    pub(super) fn into_data(self, op: &str) -> ExchangeResult<Vec<T>> {
        ensure_ok(&self.code, &self.msg, op)?;
        Ok(self.data)
    }
}

#[derive(Debug, Deserialize)]
#[serde(bound(deserialize = "T: serde::de::DeserializeOwned"))]
pub(super) struct BitgetObjectResponse<T> {
    code: String,
    #[serde(default)]
    msg: String,
    data: Option<T>,
}

#[derive(Debug, Deserialize)]
struct BitgetErrorResponse {
    code: String,
    #[serde(default)]
    msg: String,
}

impl<T> BitgetObjectResponse<T> {
    pub(super) fn into_result(self, op: &str) -> ExchangeResult<T> {
        self.into_option(op)?
            .ok_or_else(|| ExchangeError::Parse(format!("bitget {op}: empty data")))
    }

    pub(super) fn into_option(self, op: &str) -> ExchangeResult<Option<T>> {
        ensure_ok(&self.code, &self.msg, op)?;
        Ok(self.data)
    }
}

fn ensure_ok(code: &str, msg: &str, op: &str) -> ExchangeResult<()> {
    if code == SUCCESS_CODE {
        Ok(())
    } else {
        Err(ExchangeError::Api {
            exchange: NAME.into(),
            code: code.to_owned(),
            message: format!("{op}: {msg}"),
        })
    }
}

pub(super) fn api_error_from_body(body: &str, op: &str) -> Option<ExchangeError> {
    let response: BitgetErrorResponse = serde_json::from_str(body).ok()?;
    ensure_ok(&response.code, &response.msg, op).err()
}

pub(super) fn parse_err(error: &reqwest::Error) -> ExchangeError {
    ExchangeError::Parse(format!("bitget json: {error}"))
}
