//! Bitget official currency-chain metadata.
//!
//! Docs: <https://www.bitget.com/api-doc/spot/market/Get-Coin-List>

use super::bitget_response::BitgetResponse;
use crate::error::{ExchangeError, ExchangeResult};
use crate::http::HttpClient;
use crate::{canonical_network_id, parse_optional_decimal, CurrencyTransferNetwork};
use reqwest::Method;
use serde::Deserialize;
use serde_json::Value;

const SOURCE_PATH: &str = "/api/v2/spot/public/coins";

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
    need_tag: Value,
    #[serde(default)]
    withdrawable: Value,
    #[serde(default)]
    rechargeable: Value,
    #[serde(default)]
    withdraw_fee: String,
    #[serde(default)]
    extra_withdraw_fee: String,
    #[serde(default)]
    min_deposit_amount: String,
    #[serde(default)]
    min_withdraw_amount: String,
    #[serde(default)]
    deposit_confirm: String,
    #[serde(default)]
    withdraw_confirm: String,
    #[serde(default)]
    congestion: String,
    #[serde(default)]
    contract_address: Option<String>,
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
    let response: BitgetResponse<CoinRow> = serde_json::from_str(body)
        .map_err(|error| ExchangeError::Parse(format!("bitget transfer networks: {error}")))?;
    let coins = response.into_data("transfer networks")?;
    Ok(coins
        .into_iter()
        .flat_map(|coin| {
            coin.chains.into_iter().map(move |chain| {
                let network = chain.chain.trim().to_owned();
                CurrencyTransferNetwork {
                    venue: "bitget".to_owned(),
                    currency: coin.coin.trim().to_ascii_uppercase(),
                    canonical_network: canonical_network_id(&network),
                    network,
                    contract_address: non_empty(chain.contract_address),
                    deposit_enabled: boolish(&chain.rechargeable),
                    withdraw_enabled: boolish(&chain.withdrawable),
                    withdrawal_fee: parse_optional_decimal(Some(&chain.withdraw_fee)),
                    withdrawal_fee_rate: parse_optional_decimal(Some(&chain.extra_withdraw_fee)),
                    withdrawal_step: None,
                    min_withdraw: parse_optional_decimal(Some(&chain.min_withdraw_amount)),
                    min_deposit: parse_optional_decimal(Some(&chain.min_deposit_amount)),
                    requires_tag: boolish(&chain.need_tag),
                    credit_confirmations: parse_optional_u64(&chain.deposit_confirm),
                    unlock_confirmations: parse_optional_u64(&chain.withdraw_confirm),
                    network_status: non_empty_string(chain.congestion),
                    checked_at_ms,
                    source_url: source_url.to_owned(),
                }
            })
        })
        .collect())
}

fn boolish(value: &Value) -> bool {
    match value {
        Value::Bool(value) => *value,
        Value::String(value) => value.eq_ignore_ascii_case("true") || value == "1",
        Value::Number(value) => value.as_i64() == Some(1),
        _ => false,
    }
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.trim().is_empty())
}

fn non_empty_string(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
}

fn parse_optional_u64(value: &str) -> Option<u64> {
    value.trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_official_coin_chain_shape() {
        let rows = parse(
            r#"{"code":"00000","msg":"success","data":[{"coin":"USDT","chains":[{"chain":"ERC20","needTag":"false","withdrawable":"true","rechargeable":"true","withdrawFee":"3","extraWithdrawFee":"0","minDepositAmount":"1","minWithdrawAmount":"5","depositConfirm":"12","withdrawConfirm":"24","congestion":"normal","contractAddress":"0xabc"}]}]}"#,
            "official",
            1,
        )
        .expect("bitget transfer rows");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].canonical_network, "ethereum");
        assert!(rows[0].deposit_enabled && rows[0].withdraw_enabled);
        assert!(rows[0].has_cost_evidence());
        assert_eq!(rows[0].credit_confirmations, Some(12));
        assert_eq!(rows[0].unlock_confirmations, Some(24));
        assert_eq!(rows[0].network_status.as_deref(), Some("normal"));
    }

    #[test]
    fn native_chain_accepts_official_null_contract_address() {
        let rows = parse(
            r#"{"code":"00000","msg":"success","data":[{"coin":"BTC","chains":[{"chain":"BTC","needTag":"false","withdrawable":"true","rechargeable":"true","withdrawFee":"0.00003","extraWithdrawFee":"0","minDepositAmount":"0.00001","minWithdrawAmount":"0.0005","contractAddress":null}]}]}"#,
            "official",
            1,
        )
        .expect("bitget native transfer row");

        assert_eq!(rows.len(), 1);
        assert!(rows[0].contract_address.is_none());
        assert!(rows[0].has_cost_evidence());
    }
}
