//! Gate official spot currency-chain status.
//!
//! Docs: <https://www.gate.com/docs/developers/apiv4/en/spot/#query-all-currency-information>

use crate::error::{ExchangeError, ExchangeResult};
use crate::http::HttpClient;
use crate::{canonical_network_id, CurrencyTransferNetwork};
use reqwest::Method;
use serde::Deserialize;

const SOURCE_PATH: &str = "/api/v4/spot/currencies";

#[derive(Debug, Deserialize)]
struct CurrencyRow {
    currency: String,
    #[serde(default)]
    delisted: bool,
    #[serde(default)]
    chains: Vec<ChainRow>,
}

#[derive(Debug, Deserialize)]
struct ChainRow {
    name: String,
    #[serde(default)]
    addr: String,
    #[serde(default)]
    withdraw_disabled: bool,
    #[serde(default)]
    withdraw_delayed: bool,
    #[serde(default)]
    deposit_disabled: bool,
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
    let currencies: Vec<CurrencyRow> = serde_json::from_str(body)
        .map_err(|error| ExchangeError::Parse(format!("gate transfer networks: {error}")))?;
    Ok(currencies
        .into_iter()
        .flat_map(|currency| {
            currency.chains.into_iter().map(move |chain| {
                let network = chain.name.trim().to_owned();
                let network_status = gate_network_status(currency.delisted, &chain);
                CurrencyTransferNetwork {
                    venue: "gate".to_owned(),
                    currency: currency.currency.trim().to_ascii_uppercase(),
                    canonical_network: canonical_network_id(&network),
                    network,
                    contract_address: non_empty(chain.addr),
                    deposit_enabled: !currency.delisted && !chain.deposit_disabled,
                    withdraw_enabled: !currency.delisted
                        && !chain.withdraw_disabled
                        && !chain.withdraw_delayed,
                    withdrawal_fee: None,
                    withdrawal_fee_rate: None,
                    withdrawal_step: None,
                    min_withdraw: None,
                    min_deposit: None,
                    requires_tag: false,
                    credit_confirmations: None,
                    unlock_confirmations: None,
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

fn gate_network_status(delisted: bool, row: &ChainRow) -> Option<String> {
    let mut reasons = Vec::with_capacity(3);
    if delisted {
        reasons.push("currency delisted");
    }
    if row.deposit_disabled {
        reasons.push("deposit disabled");
    }
    if row.withdraw_disabled {
        reasons.push("withdrawal disabled");
    } else if row.withdraw_delayed {
        reasons.push("withdrawal delayed");
    }
    (!reasons.is_empty()).then(|| reasons.join("; "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delayed_withdrawal_is_not_a_live_route() {
        let rows = parse(
            r#"[{"currency":"USDT","delisted":false,"chains":[{"name":"ETH","addr":"0xabc","withdraw_disabled":false,"withdraw_delayed":true,"deposit_disabled":false}]}]"#,
            "official",
            1,
        )
        .expect("gate transfer rows");

        assert!(rows[0].deposit_enabled);
        assert!(!rows[0].withdraw_enabled);
        assert_eq!(
            rows[0].network_status.as_deref(),
            Some("withdrawal delayed")
        );
    }
}
