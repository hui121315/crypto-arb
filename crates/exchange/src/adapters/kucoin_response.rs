//! KuCoin REST response envelopes.

use crate::error::{ExchangeError, ExchangeResult};
use serde::Deserialize;

const NAME: &str = "kucoin";
const SUCCESS_CODE: &str = "200000";

#[derive(Debug, Deserialize)]
#[serde(bound(deserialize = "T: serde::de::DeserializeOwned"))]
pub(super) struct KucoinResponse<T> {
    code: String,
    #[serde(default)]
    msg: String,
    data: Option<T>,
}

impl<T> KucoinResponse<T> {
    pub(super) fn into_data(self, op: &str) -> ExchangeResult<T> {
        if self.code != SUCCESS_CODE {
            return Err(ExchangeError::Api {
                exchange: NAME.into(),
                code: self.code,
                message: format!("{op}: {}", self.msg),
            });
        }
        self.data
            .ok_or_else(|| ExchangeError::Parse(format!("kucoin {op} empty data")))
    }
}

pub(super) fn parse_err(error: &reqwest::Error) -> ExchangeError {
    ExchangeError::Parse(format!("kucoin json: {error}"))
}
