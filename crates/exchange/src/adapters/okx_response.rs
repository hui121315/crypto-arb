//! OKX V5 REST response envelopes.

use crate::error::{ExchangeError, ExchangeResult};
use serde::de::DeserializeOwned;
use serde::Deserialize;

const NAME: &str = "okx";
const SUCCESS_CODE: &str = "0";

#[derive(Debug, Deserialize)]
#[serde(bound(deserialize = "T: serde::de::DeserializeOwned"))]
pub(super) struct OkxResponse<T> {
    code: String,
    #[serde(default)]
    msg: String,
    #[serde(default = "Vec::new")]
    data: Vec<T>,
}

impl<T> OkxResponse<T> {
    pub(super) fn into_data(self, op: &str) -> ExchangeResult<Vec<T>> {
        ensure_ok(&self.code, &self.msg, op)?;
        Ok(self.data)
    }
}

#[derive(Debug, Deserialize)]
#[serde(bound(deserialize = "T: serde::de::DeserializeOwned"))]
pub(super) struct OkxObjectResponse<T> {
    code: String,
    #[serde(default)]
    msg: String,
    data: Option<T>,
}

impl<T> OkxObjectResponse<T> {
    pub(super) fn into_result(self, op: &str) -> ExchangeResult<T> {
        ensure_ok(&self.code, &self.msg, op)?;
        self.data
            .ok_or_else(|| ExchangeError::Parse(format!("okx {op} empty")))
    }
}

pub(super) fn data_from_text<T: DeserializeOwned>(
    text: &str,
    context: &str,
) -> ExchangeResult<Vec<T>> {
    let response: OkxResponse<T> = serde_json::from_str(text)
        .map_err(|error| ExchangeError::Parse(format!("{context}: {error}: {text}")))?;
    response.into_data(context)
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

pub(super) fn parse_err(error: &reqwest::Error) -> ExchangeError {
    ExchangeError::Parse(format!("okx json: {error}"))
}
