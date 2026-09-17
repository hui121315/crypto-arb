//! Bybit V5 REST response envelopes.

use crate::error::{ExchangeError, ExchangeResult};
use serde::Deserialize;

const NAME: &str = "bybit";

#[derive(Debug, Deserialize)]
#[serde(bound(deserialize = "T: serde::de::DeserializeOwned"))]
pub(super) struct BybitResponse<T> {
    #[serde(rename = "retCode")]
    ret_code: i32,
    #[serde(default, rename = "retMsg")]
    ret_msg: String,
    result: BybitResult<T>,
    #[serde(default)]
    time: i64,
}

#[derive(Debug, Deserialize)]
#[serde(bound(deserialize = "T: serde::de::DeserializeOwned"))]
struct BybitResult<T> {
    #[serde(default = "Vec::new")]
    list: Vec<T>,
}

impl<T> BybitResponse<T> {
    pub(super) fn into_list(self, op: &str) -> ExchangeResult<Vec<T>> {
        self.ensure_ok(op)?;
        Ok(self.result.list)
    }

    pub(super) fn into_list_with_time(self, op: &str) -> ExchangeResult<(Vec<T>, i64)> {
        self.ensure_ok(op)?;
        Ok((self.result.list, self.time))
    }

    pub(super) fn into_server_time(self, op: &str) -> ExchangeResult<i64> {
        self.ensure_ok(op)?;
        Ok(self.time)
    }

    fn ensure_ok(&self, op: &str) -> ExchangeResult<()> {
        if self.ret_code == 0 {
            Ok(())
        } else {
            Err(ExchangeError::Api {
                exchange: NAME.into(),
                code: self.ret_code.to_string(),
                message: format!("{op}: {}", self.ret_msg),
            })
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(bound(deserialize = "T: serde::de::DeserializeOwned"))]
pub(super) struct BybitObjectResponse<T> {
    #[serde(rename = "retCode")]
    ret_code: i32,
    #[serde(default, rename = "retMsg")]
    ret_msg: String,
    result: T,
}

impl<T> BybitObjectResponse<T> {
    pub(super) fn into_result(self, op: &str) -> ExchangeResult<T> {
        ensure_ok(self.ret_code, &self.ret_msg, op)?;
        Ok(self.result)
    }
}

#[derive(Debug, Deserialize)]
#[serde(bound(deserialize = "T: serde::de::DeserializeOwned"))]
pub(super) struct BybitOptionalResultResponse<T> {
    #[serde(rename = "retCode")]
    ret_code: i32,
    #[serde(default, rename = "retMsg")]
    ret_msg: String,
    #[serde(default)]
    result: Option<T>,
}

impl<T> BybitOptionalResultResponse<T> {
    pub(super) fn into_result(self, op: &str) -> ExchangeResult<T> {
        ensure_ok(self.ret_code, &self.ret_msg, op)?;
        self.result
            .ok_or_else(|| ExchangeError::Parse(format!("bybit {op} null result")))
    }
}

fn ensure_ok(ret_code: i32, ret_msg: &str, op: &str) -> ExchangeResult<()> {
    if ret_code == 0 {
        Ok(())
    } else {
        Err(ExchangeError::Api {
            exchange: NAME.into(),
            code: ret_code.to_string(),
            message: format!("{op}: {ret_msg}"),
        })
    }
}

pub(super) fn parse_err(error: &reqwest::Error) -> ExchangeError {
    ExchangeError::Parse(format!("bybit json: {error}"))
}
