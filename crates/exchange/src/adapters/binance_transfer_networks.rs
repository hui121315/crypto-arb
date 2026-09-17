//! Binance wallet currency-network metadata.
//!
//! Docs: <https://developers.binance.com/en/docs/catalog/core-trading-wallet/api/rest-api/capital#all-coins-information>

use crate::error::{ExchangeError, ExchangeResult};
use crate::http::HttpClient;
use crate::{canonical_network_id, parse_optional_decimal, CurrencyTransferNetwork};
use reqwest::Method;
use serde::Deserialize;

const SOURCE_PATH: &str = "/sapi/v1/capital/config/getall";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CoinRow {
    coin: String,
    #[serde(default)]
    deposit_all_enable: bool,
    #[serde(default)]
    withdraw_all_enable: bool,
    #[serde(default)]
    network_list: Vec<NetworkRow>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NetworkRow {
    network: String,
    #[serde(default)]
    deposit_enable: bool,
    #[serde(default)]
    withdraw_enable: bool,
    #[serde(default)]
    busy: bool,
    #[serde(default)]
    withdraw_fee: String,
    #[serde(default)]
    withdraw_integer_multiple: String,
    #[serde(default)]
    withdraw_min: String,
    #[serde(default)]
    deposit_dust: String,
    #[serde(default)]
    withdraw_tag: bool,
    #[serde(default)]
    min_confirm: Option<u64>,
    #[serde(default, rename = "unLockConfirm")]
    unlock_confirm: Option<u64>,
    #[serde(default)]
    contract_address: String,
}

pub(super) async fn fetch<F>(
    http: &HttpClient,
    base_url: &str,
    mut signed_query: F,
) -> ExchangeResult<Vec<CurrencyTransferNetwork>>
where
    F: FnMut() -> ExchangeResult<(String, String)>,
{
    let url = format!("{base_url}{SOURCE_PATH}");
    let response = http
        .execute_with_retry_fresh(Method::GET, &url, || {
            let (query, api_key) = signed_query()?;
            Ok(http
                .request(Method::GET, format!("{url}?{query}"))
                .header("X-MBX-APIKEY", api_key))
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
    let coins: Vec<CoinRow> = serde_json::from_str(body)
        .map_err(|error| ExchangeError::Parse(format!("binance transfer networks: {error}")))?;
    Ok(coins
        .into_iter()
        .flat_map(|coin| {
            coin.network_list.into_iter().map(move |network| {
                let native_network = network.network.trim().to_owned();
                CurrencyTransferNetwork {
                    venue: "binance".to_owned(),
                    currency: coin.coin.trim().to_ascii_uppercase(),
                    canonical_network: canonical_network_id(&native_network),
                    network: native_network,
                    contract_address: non_empty(network.contract_address),
                    deposit_enabled: coin.deposit_all_enable && network.deposit_enable,
                    withdraw_enabled: coin.withdraw_all_enable
                        && network.withdraw_enable
                        && !network.busy,
                    withdrawal_fee: parse_optional_decimal(Some(&network.withdraw_fee)),
                    withdrawal_fee_rate: Some(rust_decimal::Decimal::ZERO),
                    withdrawal_step: parse_optional_decimal(Some(
                        &network.withdraw_integer_multiple,
                    )),
                    min_withdraw: parse_optional_decimal(Some(&network.withdraw_min)),
                    min_deposit: parse_optional_decimal(Some(&network.deposit_dust)),
                    requires_tag: network.withdraw_tag,
                    credit_confirmations: network.min_confirm,
                    unlock_confirmations: network.unlock_confirm,
                    network_status: network.busy.then(|| "busy".to_owned()),
                    checked_at_ms,
                    source_url: source_url.to_owned(),
                }
            })
        })
        .collect())
}

fn non_empty(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn busy_network_cannot_authorize_withdrawal() {
        let rows = parse(
            r#"[{"coin":"USDT","depositAllEnable":true,"withdrawAllEnable":true,"networkList":[{"network":"BSC","depositEnable":true,"withdrawEnable":true,"busy":true,"withdrawFee":"0.3","withdrawIntegerMultiple":"0.01","withdrawMin":"1","depositDust":"0.01","withdrawTag":false,"minConfirm":5,"unLockConfirm":10,"contractAddress":"0xabc"}]}]"#,
            "official",
            1,
        )
        .expect("binance transfer rows");

        assert!(rows[0].deposit_enabled);
        assert!(!rows[0].withdraw_enabled);
        assert!(rows[0].has_cost_evidence());
        assert_eq!(
            rows[0].withdrawal_step,
            parse_optional_decimal(Some("0.01"))
        );
        assert_eq!(rows[0].credit_confirmations, Some(5));
        assert_eq!(rows[0].unlock_confirmations, Some(10));
        assert_eq!(rows[0].network_status.as_deref(), Some("busy"));
    }
}
