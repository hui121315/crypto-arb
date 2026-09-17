//! OKX V5 funding-account currency-chain metadata.
//!
//! Docs: <https://www.okx.com/docs-v5/en/#rest-api-funding-get-currencies>

use super::okx_response::OkxResponse;
use crate::error::{ExchangeError, ExchangeResult};
use crate::http::HttpClient;
use crate::{canonical_network_id, parse_optional_decimal, CurrencyTransferNetwork};
use reqwest::Method;
use serde::Deserialize;

const SOURCE_PATH: &str = "/api/v5/asset/currencies";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CurrencyRow {
    ccy: String,
    chain: String,
    #[serde(default)]
    can_dep: bool,
    #[serde(default)]
    can_wd: bool,
    #[serde(default)]
    min_dep: String,
    #[serde(default)]
    min_wd: String,
    #[serde(default)]
    fee: String,
    #[serde(default)]
    min_fee: String,
    #[serde(default)]
    max_fee: String,
    #[serde(default)]
    burning_fee_rate: String,
    #[serde(default)]
    need_tag: bool,
    #[serde(default)]
    min_dep_arrival_confirm: String,
    #[serde(default)]
    min_wd_unlock_confirm: String,
    #[serde(default)]
    dep_est_open_time: String,
    #[serde(default)]
    wd_est_open_time: String,
    #[serde(default)]
    ct_addr: String,
}

pub(super) async fn fetch<F>(
    http: &HttpClient,
    base_url: &str,
    currencies: &[String],
    mut signed_headers: F,
) -> ExchangeResult<Vec<CurrencyTransferNetwork>>
where
    F: FnMut(&str) -> ExchangeResult<[(String, String); 4]>,
{
    let requested = currencies
        .iter()
        .map(|currency| currency.trim().to_ascii_uppercase())
        .filter(|currency| !currency.is_empty())
        .collect::<std::collections::BTreeSet<_>>();
    let path = if requested.is_empty() {
        SOURCE_PATH.to_owned()
    } else {
        let mut query = url::form_urlencoded::Serializer::new(String::new());
        query.append_pair("ccy", &requested.into_iter().collect::<Vec<_>>().join(","));
        format!("{SOURCE_PATH}?{}", query.finish())
    };
    let url = format!("{base_url}{path}");
    let response = http
        .execute_with_retry_fresh(Method::GET, &url, || {
            let headers = signed_headers(&path)?;
            let mut request = http.request(Method::GET, &url);
            for (key, value) in headers {
                request = request.header(key, value);
            }
            Ok(request)
        })
        .await?;
    let body = response
        .text()
        .await
        .map_err(|error| ExchangeError::Network(error.to_string()))?;
    parse(&body, &url, common::time::now_ms())
}

fn parse(
    body: &str,
    source_url: &str,
    checked_at_ms: i64,
) -> ExchangeResult<Vec<CurrencyTransferNetwork>> {
    let response: OkxResponse<CurrencyRow> = serde_json::from_str(body)
        .map_err(|error| ExchangeError::Parse(format!("okx transfer networks: {error}")))?;
    Ok(response
        .into_data("transfer networks")?
        .into_iter()
        .map(|row| {
            let network = row
                .chain
                .split_once('-')
                .map_or(row.chain.as_str(), |(_, network)| network)
                .trim()
                .to_owned();
            let network_status = okx_network_status(&row);
            CurrencyTransferNetwork {
                venue: "okx".to_owned(),
                currency: row.ccy.trim().to_ascii_uppercase(),
                canonical_network: canonical_network_id(&network),
                network,
                contract_address: non_empty(row.ct_addr),
                deposit_enabled: row.can_dep,
                withdraw_enabled: row.can_wd,
                withdrawal_fee: parse_optional_decimal(Some(&row.fee))
                    .or_else(|| parse_optional_decimal(Some(&row.max_fee)))
                    .or_else(|| parse_optional_decimal(Some(&row.min_fee))),
                withdrawal_fee_rate: parse_optional_decimal(Some(&row.burning_fee_rate))
                    .or(Some(rust_decimal::Decimal::ZERO)),
                withdrawal_step: None,
                min_withdraw: parse_optional_decimal(Some(&row.min_wd)),
                min_deposit: parse_optional_decimal(Some(&row.min_dep)),
                requires_tag: row.need_tag,
                credit_confirmations: parse_optional_u64(&row.min_dep_arrival_confirm),
                unlock_confirmations: parse_optional_u64(&row.min_wd_unlock_confirm),
                network_status,
                checked_at_ms,
                source_url: source_url.to_owned(),
            }
        })
        .collect())
}

fn non_empty(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
}

fn parse_optional_u64(value: &str) -> Option<u64> {
    value.trim().parse().ok()
}

fn okx_network_status(row: &CurrencyRow) -> Option<String> {
    let mut reasons = Vec::with_capacity(2);
    if !row.can_dep {
        reasons.push(if row.dep_est_open_time.trim().is_empty() {
            "deposit unavailable".to_owned()
        } else {
            format!("deposit opens at {}", row.dep_est_open_time)
        });
    }
    if !row.can_wd {
        reasons.push(if row.wd_est_open_time.trim().is_empty() {
            "withdrawal unavailable".to_owned()
        } else {
            format!("withdrawal opens at {}", row.wd_est_open_time)
        });
    }
    (!reasons.is_empty()).then(|| reasons.join("; "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_conservative_maximum_fee() {
        let rows = parse(
            r#"{"code":"0","msg":"","data":[{"ccy":"USDT","chain":"USDT-TRC20","canDep":true,"canWd":true,"minDep":"1","minWd":"2","fee":"0.8","minFee":"0.5","maxFee":"1","burningFeeRate":"","needTag":false,"minDepArrivalConfirm":"1","minWdUnlockConfirm":"2","ctAddr":"tail123"}]}"#,
            "official",
            1,
        )
        .expect("okx transfer rows");

        assert_eq!(rows[0].canonical_network, "tron");
        assert_eq!(rows[0].withdrawal_fee, parse_optional_decimal(Some("0.8")));
        assert_eq!(rows[0].contract_address.as_deref(), Some("tail123"));
        assert!(rows[0].has_cost_evidence());
        assert_eq!(rows[0].credit_confirmations, Some(1));
        assert_eq!(rows[0].unlock_confirmations, Some(2));
    }
}
