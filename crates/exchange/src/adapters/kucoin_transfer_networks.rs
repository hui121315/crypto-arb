//! KuCoin public currency-chain metadata.
//!
//! Docs: <https://www.kucoin.com/docs-new/rest/spot-trading/market-data/get-all-currencies>

use crate::error::{ExchangeError, ExchangeResult};
use crate::http::HttpClient;
use crate::{canonical_network_id, parse_optional_decimal, CurrencyTransferNetwork};
use reqwest::Method;
use serde::Deserialize;

const SOURCE_PATH: &str = "/api/v3/currencies";

#[derive(Debug, Deserialize)]
struct KucoinResponse {
    code: String,
    #[serde(default)]
    data: Vec<CurrencyRow>,
}

#[derive(Debug, Deserialize)]
struct CurrencyRow {
    currency: String,
    #[serde(default)]
    chains: Option<Vec<ChainRow>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChainRow {
    chain_name: String,
    #[serde(default)]
    chain_id: String,
    #[serde(default)]
    withdrawal_min_size: String,
    #[serde(default)]
    withdraw_min_size: String,
    #[serde(default)]
    deposit_min_size: Option<String>,
    #[serde(default)]
    withdraw_fee_rate: String,
    #[serde(default)]
    withdrawal_min_fee: String,
    #[serde(default)]
    withdraw_min_fee: String,
    #[serde(default)]
    is_withdraw_enabled: bool,
    #[serde(default)]
    is_deposit_enabled: bool,
    #[serde(default)]
    need_tag: bool,
    #[serde(default)]
    confirms: Option<u64>,
    #[serde(default)]
    pre_confirms: Option<u64>,
    #[serde(default)]
    contract_address: String,
}

pub(super) async fn fetch(
    http: &HttpClient,
    base_url: &str,
) -> ExchangeResult<Vec<CurrencyTransferNetwork>> {
    let url = format!("{base_url}{SOURCE_PATH}");
    let response = http
        .execute_with_retry(|| http.request(Method::GET, &url))
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
    let response: KucoinResponse = serde_json::from_str(body)
        .map_err(|error| ExchangeError::Parse(format!("kucoin transfer networks: {error}")))?;
    if response.code != "200000" {
        return Err(ExchangeError::Api {
            exchange: "kucoin".to_owned(),
            code: response.code,
            message: "currency-chain metadata request failed".to_owned(),
        });
    }
    Ok(response
        .data
        .into_iter()
        .flat_map(|currency| {
            currency
                .chains
                .unwrap_or_default()
                .into_iter()
                .map(move |chain| {
                    let network = if chain.chain_id.trim().is_empty() {
                        chain.chain_name.trim().to_owned()
                    } else {
                        chain.chain_id.trim().to_owned()
                    };
                    let fixed_fee =
                        first_decimal(&chain.withdraw_min_fee, &chain.withdrawal_min_fee);
                    let min_withdraw =
                        first_decimal(&chain.withdraw_min_size, &chain.withdrawal_min_size);
                    let network_status = kucoin_network_status(&chain);
                    CurrencyTransferNetwork {
                        venue: "kucoin".to_owned(),
                        currency: currency.currency.trim().to_ascii_uppercase(),
                        canonical_network: canonical_network_id(&network),
                        network,
                        contract_address: non_empty(chain.contract_address),
                        deposit_enabled: chain.is_deposit_enabled,
                        withdraw_enabled: chain.is_withdraw_enabled,
                        withdrawal_fee: fixed_fee,
                        withdrawal_fee_rate: parse_optional_decimal(Some(&chain.withdraw_fee_rate)),
                        withdrawal_step: None,
                        min_withdraw,
                        min_deposit: parse_optional_decimal(chain.deposit_min_size.as_deref()),
                        requires_tag: chain.need_tag,
                        credit_confirmations: chain.pre_confirms,
                        unlock_confirmations: chain.confirms,
                        network_status,
                        checked_at_ms,
                        source_url: source_url.to_owned(),
                    }
                })
        })
        .collect())
}

fn first_decimal(primary: &str, fallback: &str) -> Option<rust_decimal::Decimal> {
    parse_optional_decimal(Some(primary)).or_else(|| parse_optional_decimal(Some(fallback)))
}

fn non_empty(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
}

fn kucoin_network_status(row: &ChainRow) -> Option<String> {
    match (row.is_deposit_enabled, row.is_withdraw_enabled) {
        (true, true) => None,
        (true, false) => Some("withdrawal disabled".to_owned()),
        (false, true) => Some("deposit disabled".to_owned()),
        (false, false) => Some("deposit and withdrawal disabled".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_chain_id_for_cross_venue_identity() {
        let rows = parse(
            r#"{"code":"200000","data":[{"currency":"BTC","chains":[{"chainName":"Bitcoin","chainId":"btc","withdrawMinSize":"0.001","depositMinSize":"0.0002","withdrawFeeRate":"0","withdrawMinFee":"0.0005","isWithdrawEnabled":true,"isDepositEnabled":true,"confirms":3,"preConfirms":1,"needTag":false,"contractAddress":""}]}]}"#,
            "official",
            1,
        )
        .expect("kucoin transfer rows");

        assert_eq!(rows[0].canonical_network, "bitcoin");
        assert!(rows[0].has_cost_evidence());
        assert_eq!(rows[0].credit_confirmations, Some(1));
        assert_eq!(rows[0].unlock_confirmations, Some(3));
    }

    #[test]
    fn fiat_currency_accepts_official_null_chains() {
        let rows = parse(
            r#"{"code":"200000","data":[{"currency":"USD","chains":null}]}"#,
            "official",
            1,
        )
        .expect("kucoin fiat currency row");

        assert!(rows.is_empty());
    }
}
