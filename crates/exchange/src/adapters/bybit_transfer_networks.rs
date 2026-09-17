//! Bybit V5 currency-chain metadata.
//!
//! Docs: <https://bybit-exchange.github.io/docs/v5/asset/coin-info>

use super::bybit_response::BybitObjectResponse;
use crate::error::{ExchangeError, ExchangeResult};
use crate::http::HttpClient;
use crate::{canonical_network_id, parse_optional_decimal, CurrencyTransferNetwork};
use reqwest::Method;
use serde::Deserialize;

const SOURCE_PATH: &str = "/v5/asset/coin/query-info";

#[derive(Debug, Deserialize)]
struct CoinPage {
    #[serde(default)]
    rows: Vec<CoinRow>,
}

#[derive(Debug, Deserialize)]
struct CoinRow {
    coin: String,
    #[serde(default)]
    chains: Vec<ChainRow>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChainRow {
    chain: String,
    #[serde(default)]
    chain_type: String,
    #[serde(default)]
    withdraw_fee: String,
    #[serde(default)]
    withdraw_percentage_fee: String,
    #[serde(default)]
    deposit_min: String,
    #[serde(default)]
    withdraw_min: String,
    #[serde(default)]
    min_accuracy: String,
    #[serde(default)]
    chain_deposit: String,
    #[serde(default)]
    chain_withdraw: String,
    #[serde(default)]
    confirmation: String,
    #[serde(default)]
    safe_confirm_number: String,
    #[serde(default)]
    contract_address: String,
}

pub(super) async fn fetch<F>(
    http: &HttpClient,
    base_url: &str,
    mut signed_headers: F,
) -> ExchangeResult<Vec<CurrencyTransferNetwork>>
where
    F: FnMut() -> ExchangeResult<[(String, String); 4]>,
{
    let url = format!("{base_url}{SOURCE_PATH}");
    let response = http
        .execute_with_retry_fresh(Method::GET, &url, || {
            let headers = signed_headers()?;
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
    let response: BybitObjectResponse<CoinPage> = serde_json::from_str(body)
        .map_err(|error| ExchangeError::Parse(format!("bybit transfer networks: {error}")))?;
    let page = response.into_result("transfer networks")?;
    Ok(page
        .rows
        .into_iter()
        .flat_map(|coin| {
            coin.chains.into_iter().map(move |chain| {
                // Bybit requires the `chain` value (for example `ETH`) as
                // `chainType` on its deposit-address endpoint. `chainType` in
                // this response is only the human-readable label.
                let network = chain.chain.trim().to_owned();
                let canonical_source = if chain.chain_type.trim().is_empty() {
                    &network
                } else {
                    chain.chain_type.trim()
                };
                let network_status = bybit_network_status(&chain);
                CurrencyTransferNetwork {
                    venue: "bybit".to_owned(),
                    currency: coin.coin.trim().to_ascii_uppercase(),
                    canonical_network: canonical_network_id(canonical_source),
                    network,
                    contract_address: non_empty(chain.contract_address),
                    deposit_enabled: chain.chain_deposit == "1",
                    withdraw_enabled: chain.chain_withdraw == "1"
                        && !chain.withdraw_fee.trim().is_empty(),
                    withdrawal_fee: parse_optional_decimal(Some(&chain.withdraw_fee)),
                    withdrawal_fee_rate: parse_optional_decimal(Some(
                        &chain.withdraw_percentage_fee,
                    )),
                    withdrawal_step: chain
                        .min_accuracy
                        .trim()
                        .parse::<u32>()
                        .ok()
                        .and_then(|scale| rust_decimal::Decimal::try_new(1, scale).ok()),
                    min_withdraw: parse_optional_decimal(Some(&chain.withdraw_min)),
                    min_deposit: parse_optional_decimal(Some(&chain.deposit_min)),
                    requires_tag: false,
                    credit_confirmations: parse_optional_u64(&chain.confirmation),
                    unlock_confirmations: parse_optional_u64(&chain.safe_confirm_number),
                    network_status,
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

fn parse_optional_u64(value: &str) -> Option<u64> {
    value.trim().parse().ok()
}

fn bybit_network_status(row: &ChainRow) -> Option<String> {
    match (row.chain_deposit.as_str(), row.chain_withdraw.as_str()) {
        ("1", "1") => None,
        ("1", _) => Some("withdrawal suspended".to_owned()),
        (_, "1") => Some("deposit suspended".to_owned()),
        _ => Some("deposit and withdrawal suspended".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bybit_withdrawal_metadata_preserves_precision_and_unknown_percentage_fee() {
        for (accuracy, rate, step) in [
            ("6", "0.01", Some(rust_decimal::Decimal::new(1, 6))),
            ("0", "", Some(rust_decimal::Decimal::ONE)),
            ("29", "", None),
            ("bad", "", None),
        ] {
            let body = serde_json::json!({"retCode":0,"retMsg":"OK","result":{"rows":[{"coin":"USDC", "chains":[{
                "chain":"SOL","chainType":"Solana","withdrawFee":"0.1","withdrawPercentageFee":rate,
                "minAccuracy":accuracy,"withdrawMin":"1","chainDeposit":"1","chainWithdraw":"1"
            }]}]}}).to_string();
            let row = parse(&body, "official", 1).unwrap().remove(0);
            assert_eq!(row.withdrawal_step, step);
            assert_eq!(
                row.withdrawal_fee_rate,
                rate.parse::<rust_decimal::Decimal>().ok()
            );
        }
    }

    #[test]
    fn suspended_chain_fails_closed() {
        let rows = parse(
            r#"{"retCode":0,"retMsg":"OK","result":{"rows":[{"coin":"USDT","chains":[{"chain":"ETH","chainType":"ERC20","withdrawFee":"3","withdrawPercentageFee":"0","depositMin":"1","withdrawMin":"5","chainDeposit":"1","chainWithdraw":"0","confirmation":"12","safeConfirmNumber":"24","contractAddress":"0xabc"}]}]}}"#,
            "official",
            1,
        )
        .expect("bybit transfer rows");

        assert!(rows[0].deposit_enabled);
        assert!(!rows[0].withdraw_enabled);
        assert_eq!(rows[0].network, "ETH");
        assert_eq!(rows[0].canonical_network, "ethereum");
        assert_eq!(rows[0].credit_confirmations, Some(12));
        assert_eq!(rows[0].unlock_confirmations, Some(24));
        assert_eq!(
            rows[0].network_status.as_deref(),
            Some("withdrawal suspended")
        );
    }
}
