//! Explicit rebased stock inventory and equity-pair fees; no trading or borrowing.
//! https://docs.kraken.com/api-reference/account-data/get-extended-balance
//! https://docs.kraken.com/api-reference/account-data/get-trade-volume
use super::{kraken::Kraken, kraken_spot_rest::next_nonce};
use crate::{ExchangeError, ExchangeResult};
use reqwest::Method;
use rust_decimal::Decimal;
use serde_json::{json, Value};
use shared_types::stocks::StockPeerAccount;

fn invalid(message: &str) -> ExchangeError {
    ExchangeError::Parse(format!("kraken stock account: {message}"))
}
fn result(text: &str) -> ExchangeResult<Value> {
    let v: Value = serde_json::from_str(text).map_err(|_| invalid("invalid JSON"))?;
    if !v
        .get("error")
        .and_then(Value::as_array)
        .is_some_and(|e| e.is_empty())
    {
        return Err(invalid("remote read rejected"));
    }
    v.get("result")
        .filter(|v| v.is_object())
        .cloned()
        .ok_or_else(|| invalid("result missing"))
}
fn decimal(v: &Value) -> Option<Decimal> {
    match v {
        Value::String(s) => s.parse().ok(),
        Value::Number(n) => n.to_string().parse().ok(),
        _ => None,
    }
}
fn fee(v: &Value, key: &str) -> Option<String> {
    decimal(v.get("fees")?.get(key)?.get("fee")?)
        .filter(|n| *n >= Decimal::ZERO && *n < Decimal::from(100))
        .map(|v| v.normalize().to_string())
}
fn cash(v: &Value, key: &str) -> Option<String> {
    let row = v.get(key)?;
    let balance = decimal(row.get("balance")?)?.max(Decimal::ZERO);
    let hold = decimal(row.get("hold_trade")?).filter(|v| *v >= Decimal::ZERO)?;
    let used = match row.get("credit_used") {
        None => Decimal::ZERO,
        Some(v) => decimal(v).filter(|v| *v >= Decimal::ZERO)?,
    };
    // Credit limits are not owned inventory. Existing debt and held orders reduce it.
    balance
        .checked_sub(hold)?
        .checked_sub(used)
        .map(|n| n.max(Decimal::ZERO).normalize().to_string())
}

impl Kraken {
    pub(super) async fn stock_signed_read(
        &self,
        path: &str,
        params: Value,
    ) -> ExchangeResult<String> {
        let keys = self
            .config
            .credentials
            .as_ref()
            .and_then(|c| c.spot.as_ref())
            .ok_or_else(|| ExchangeError::Auth("Kraken Spot read credentials missing".into()))?;
        let url = format!("{}{path}", self.spot_base_url);
        let response = self
            .http
            .execute_with_retry_fresh(Method::POST, &url, || {
                let nonce = next_nonce();
                let mut body = params.clone();
                body["nonce"] = json!(nonce);
                let body = serde_json::to_string(&body).map_err(|_| invalid("request encoding"))?;
                let signature = crate::signing::kraken::spot_rest_sign(
                    &keys.api_secret,
                    path,
                    &nonce.to_string(),
                    &body,
                )
                .map_err(|_| ExchangeError::Auth("Kraken Spot signing failed".into()))?;
                Ok(self
                    .http
                    .request(Method::POST, &url)
                    .header("API-Key", &keys.api_key)
                    .header("API-Sign", signature)
                    .header("Content-Type", "application/json")
                    .body(body))
            })
            .await?;
        if !response.status().is_success() {
            return Err(invalid("account HTTP read failed"));
        }
        response
            .text()
            .await
            .map_err(|_| invalid("account response decoding"))
    }

    pub(super) async fn read_stock_cash_account(
        &self,
        native: &str,
    ) -> ExchangeResult<StockPeerAccount> {
        if self
            .config
            .credentials
            .as_ref()
            .and_then(|c| c.spot.as_ref())
            .is_none()
        {
            return Err(ExchangeError::Auth(
                "Kraken Spot read credentials missing".into(),
            ));
        }
        let (base, quote) = native
            .split_once('/')
            .filter(|(b, q)| !b.is_empty() && matches!(*q, "USD" | "USDC" | "USDT"))
            .ok_or_else(|| invalid("unsupported exact stock pair"))?;
        if native.len() > 100 {
            return Err(invalid("pair too long"));
        }
        let mut url = url::Url::parse(&format!("{}/0/public/AssetPairs", self.spot_base_url))
            .map_err(|_| invalid("base URL"))?;
        url.query_pairs_mut()
            .append_pair("pair", native)
            .append_pair("aclass_base", "tokenized_asset");
        let response = self
            .http
            .execute_with_retry_fresh(Method::GET, url.as_str(), || {
                Ok(self.http.request(Method::GET, url.as_str()))
            })
            .await?;
        if !response.status().is_success() {
            return Err(invalid("market metadata HTTP failure"));
        }
        let pairs = result(
            &response
                .text()
                .await
                .map_err(|_| invalid("market response"))?,
        )?;
        let (key, spec) = pairs
            .as_object()
            .unwrap()
            .iter()
            .find(|(_, r)| r.get("wsname").and_then(Value::as_str) == Some(native))
            .ok_or_else(|| invalid("exact official market missing"))?;
        if spec["aclass_base"] != "tokenized_asset" || spec["status"] != "online" {
            return Err(invalid("stock market unavailable"));
        }
        let base_key = spec["base"]
            .as_str()
            .ok_or_else(|| invalid("official base missing"))?;
        let quote_key = spec["quote"]
            .as_str()
            .ok_or_else(|| invalid("official quote missing"))?;
        if !base_key.eq_ignore_ascii_case(base)
            || super::kraken_symbols::canonical_asset(quote_key) != quote
        {
            return Err(invalid("official pair assets mismatch"));
        }
        let mut requests = vec![json!({"asset":native,"aclass":"equity_pair"})];
        let fx_symbol = format!("USDC/{quote}");
        let mut fx_key = None;
        if quote != "USDC" {
            requests.push(json!({"asset":fx_symbol,"aclass":"forex"}));
            let mut url = url::Url::parse(&format!("{}/0/public/AssetPairs", self.spot_base_url))
                .map_err(|_| invalid("base URL"))?;
            url.query_pairs_mut().append_pair("pair", &fx_symbol);
            let response = self
                .http
                .execute_with_retry_fresh(Method::GET, url.as_str(), || {
                    Ok(self.http.request(Method::GET, url.as_str()))
                })
                .await?;
            if !response.status().is_success() {
                return Err(invalid("FX metadata HTTP failure"));
            }
            let pairs = result(
                &response
                    .text()
                    .await
                    .map_err(|_| invalid("FX metadata response"))?,
            )?;
            fx_key = pairs
                .as_object()
                .unwrap()
                .iter()
                .find(|(_, r)| {
                    r["wsname"] == fx_symbol
                        && r["base"] == "USDC"
                        && r["quote"]
                            .as_str()
                            .is_some_and(|q| super::kraken_symbols::canonical_asset(q) == quote)
                        && r["status"] == "online"
                })
                .map(|(key, _)| key.clone());
            if fx_key.is_none() {
                return Err(invalid("exact official FX market missing"));
            }
        }
        let fees = self
            .stock_signed_read(
                "/0/private/TradeVolume",
                json!({"pair":requests,"rebase_multiplier":"rebased"}),
            )
            .await
            .and_then(|s| result(&s));
        let balances = self
            .stock_signed_read(
                "/0/private/BalanceEx",
                json!({"rebase_multiplier":"rebased"}),
            )
            .await
            .and_then(|s| result(&s));
        let mut a = StockPeerAccount {
            venue: "kraken".into(),
            native_symbol: native.into(),
            stock_asset: base.into(),
            quote_asset: quote.into(),
            stock_available: None,
            quote_available: None,
            usdc_available: None,
            stock_taker_pct: None,
            fx_taker_pct: None,
            observed_at_ms: common::time::now_ms(),
            sources: vec![
                "https://docs.kraken.com/api-reference/account-data/get-extended-balance".into(),
                "https://docs.kraken.com/api-reference/account-data/get-trade-volume".into(),
            ],
            problems: vec![],
        };
        match fees {
            Ok(f) => {
                a.stock_taker_pct = fee(&f, key);
                a.fx_taker_pct = if quote == "USDC" {
                    Some("0".into())
                } else {
                    fx_key.as_deref().and_then(|key| fee(&f, key))
                };
            }
            Err(_) => a
                .problems
                .push("Kraken 账户费率读取失败，请检查读取权限".into()),
        }
        match balances {
            Ok(b) => {
                a.stock_available = cash(&b, base_key);
                a.quote_available = cash(&b, quote_key);
                a.usdc_available = cash(&b, "USDC");
            }
            Err(_) => a
                .problems
                .push("Kraken 账户库存读取失败，请检查读取权限".into()),
        }
        if a.stock_taker_pct.is_none() {
            a.problems
                .push("未取得精确股票市场的账户 taker 费率".into());
        }
        if a.fx_taker_pct.is_none() {
            a.problems
                .push("未取得精确换汇市场的账户 taker 费率".into());
        }
        Ok(a)
    }
}

#[cfg(test)]
mod tests;
