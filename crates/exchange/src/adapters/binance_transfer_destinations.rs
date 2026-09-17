//! Binance destination-address evidence for replenishment preflight.
//!
//! Official contracts:
//! - <https://developers.binance.com/en/docs/catalog/core-trading-wallet/api/rest-api/capital#deposit-address-supporting-network-user_data>
//! - <https://developers.binance.com/en/docs/catalog/core-trading-wallet/api/rest-api/capital#fetch-withdraw-address-list-user_data>

use crate::error::{ExchangeError, ExchangeResult};
use crate::http::HttpClient;
use crate::{
    TransferDestinationEvidence, TransferDestinationRequest, TransferDestinationStatus,
    TransferDirection,
};
use reqwest::Method;
use serde::Deserialize;

const DEPOSIT_ADDRESS_PATH: &str = "/sapi/v1/capital/deposit/address";
const WITHDRAW_ADDRESS_LIST_PATH: &str = "/sapi/v1/capital/withdraw/address/list";

#[derive(Debug, Deserialize)]
struct DepositAddressRow {
    address: String,
    #[serde(default)]
    tag: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WithdrawAddressRow {
    address: String,
    #[serde(default)]
    address_tag: String,
    coin: String,
    network: String,
    #[serde(default)]
    white_status: bool,
}

struct EvidenceParts {
    address: Option<String>,
    tag: Option<String>,
    status: TransferDestinationStatus,
    allowlisted: Option<bool>,
    problem: Option<String>,
}

pub(super) async fn fetch<F>(
    http: &HttpClient,
    base_url: &str,
    request: &TransferDestinationRequest,
    mut signed_query: F,
) -> ExchangeResult<TransferDestinationEvidence>
where
    F: FnMut(&[(&str, &str)]) -> ExchangeResult<(String, String)>,
{
    match request.direction {
        TransferDirection::DepositToVenue => {
            fetch_deposit(http, base_url, request, &mut signed_query).await
        }
        TransferDirection::WithdrawToChain => {
            fetch_withdraw(http, base_url, request, &mut signed_query).await
        }
    }
}

async fn fetch_deposit<F>(
    http: &HttpClient,
    base_url: &str,
    request: &TransferDestinationRequest,
    signed_query: &mut F,
) -> ExchangeResult<TransferDestinationEvidence>
where
    F: FnMut(&[(&str, &str)]) -> ExchangeResult<(String, String)>,
{
    let params = [
        ("coin", request.currency.as_str()),
        ("network", request.network.as_str()),
    ];
    let (query, api_key) = signed_query(&params)?;
    let url = format!("{base_url}{DEPOSIT_ADDRESS_PATH}");
    let body = private_get(http, &url, &query, &api_key).await?;
    parse_deposit(&body, request, &url, common::time::now_ms())
}

async fn fetch_withdraw<F>(
    http: &HttpClient,
    base_url: &str,
    request: &TransferDestinationRequest,
    signed_query: &mut F,
) -> ExchangeResult<TransferDestinationEvidence>
where
    F: FnMut(&[(&str, &str)]) -> ExchangeResult<(String, String)>,
{
    let (query, api_key) = signed_query(&[])?;
    let url = format!("{base_url}{WITHDRAW_ADDRESS_LIST_PATH}");
    let body = private_get(http, &url, &query, &api_key).await?;
    parse_withdraw(&body, request, &url, common::time::now_ms())
}

async fn private_get(
    http: &HttpClient,
    url: &str,
    query: &str,
    api_key: &str,
) -> ExchangeResult<String> {
    let response = http
        .execute_with_retry_fresh(Method::GET, url, || {
            Ok(http
                .request(Method::GET, format!("{url}?{query}"))
                .header("X-MBX-APIKEY", api_key))
        })
        .await?;
    response
        .text()
        .await
        .map_err(|error| ExchangeError::Network(error.to_string()))
}

fn parse_deposit(
    body: &str,
    request: &TransferDestinationRequest,
    source_url: &str,
    checked_at_ms: i64,
) -> ExchangeResult<TransferDestinationEvidence> {
    let row: DepositAddressRow = serde_json::from_str(body)
        .map_err(|error| ExchangeError::Parse(format!("binance deposit address: {error}")))?;
    let address = non_empty(&row.address);
    let status = if address.is_some() {
        TransferDestinationStatus::Verified
    } else {
        TransferDestinationStatus::Missing
    };
    Ok(evidence(
        request,
        checked_at_ms,
        source_url,
        EvidenceParts {
            address,
            tag: non_empty(&row.tag),
            status,
            allowlisted: None,
            problem: (status == TransferDestinationStatus::Missing)
                .then(|| "Binance 官方接口未返回充值地址".to_owned()),
        },
    ))
}

fn parse_withdraw(
    body: &str,
    request: &TransferDestinationRequest,
    source_url: &str,
    checked_at_ms: i64,
) -> ExchangeResult<TransferDestinationEvidence> {
    let rows: Vec<WithdrawAddressRow> = serde_json::from_str(body)
        .map_err(|error| ExchangeError::Parse(format!("binance withdraw addresses: {error}")))?;
    let expected_address = request.expected_address.as_deref().unwrap_or_default();
    let matched = rows.iter().find(|row| {
        row.coin.eq_ignore_ascii_case(&request.currency)
            && row.network.eq_ignore_ascii_case(&request.network)
            && address_matches(&row.address, expected_address, &request.network)
            && tags_match(&row.address_tag, request.expected_tag.as_deref())
    });
    let Some(row) = matched else {
        return Ok(evidence(
            request,
            checked_at_ms,
            source_url,
            EvidenceParts {
                address: non_empty(expected_address),
                tag: request.expected_tag.clone(),
                status: TransferDestinationStatus::Missing,
                allowlisted: Some(false),
                problem: Some("目标地址不在 Binance 当前提币地址列表中".to_owned()),
            },
        ));
    };
    let status = if row.white_status {
        TransferDestinationStatus::Verified
    } else {
        TransferDestinationStatus::Unverified
    };
    Ok(evidence(
        request,
        checked_at_ms,
        source_url,
        EvidenceParts {
            address: non_empty(&row.address),
            tag: non_empty(&row.address_tag),
            status,
            allowlisted: Some(row.white_status),
            problem: (!row.white_status)
                .then(|| "目标地址存在，但 Binance whiteStatus 未通过".to_owned()),
        },
    ))
}

fn evidence(
    request: &TransferDestinationRequest,
    checked_at_ms: i64,
    source_url: &str,
    parts: EvidenceParts,
) -> TransferDestinationEvidence {
    TransferDestinationEvidence {
        venue: "binance".to_owned(),
        currency: request.currency.trim().to_ascii_uppercase(),
        network: request.network.trim().to_owned(),
        direction: request.direction,
        address: parts.address,
        tag: parts.tag,
        status: parts.status,
        allowlisted: parts.allowlisted,
        checked_at_ms,
        source_url: source_url.to_owned(),
        problem: parts.problem,
    }
}

fn address_matches(actual: &str, expected: &str, network: &str) -> bool {
    if expected.trim().is_empty() {
        return false;
    }
    if crate::canonical_network_id(network) == "solana" {
        actual.trim() == expected.trim()
    } else {
        actual.trim().eq_ignore_ascii_case(expected.trim())
    }
}

fn tags_match(actual: &str, expected: Option<&str>) -> bool {
    let actual = actual.trim();
    match expected.map(str::trim).filter(|tag| !tag.is_empty()) {
        Some(expected) => actual == expected,
        None => actual.is_empty(),
    }
}

fn non_empty(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(direction: TransferDirection) -> TransferDestinationRequest {
        TransferDestinationRequest {
            currency: "USDC".to_owned(),
            network: "SOL".to_owned(),
            direction,
            expected_address: Some("WalletCaseSensitive".to_owned()),
            expected_tag: None,
            amount: None,
        }
    }

    #[test]
    fn withdraw_requires_exact_network_address_and_white_status() {
        let row = parse_withdraw(
            r#"[{"address":"WalletCaseSensitive","addressTag":"","coin":"USDC","network":"SOL","whiteStatus":true}]"#,
            &request(TransferDirection::WithdrawToChain),
            "official",
            1,
        )
        .expect("withdraw evidence");
        assert_eq!(row.status, TransferDestinationStatus::Verified);
        assert_eq!(row.allowlisted, Some(true));

        let wrong_case = parse_withdraw(
            r#"[{"address":"walletcasesensitive","addressTag":"","coin":"USDC","network":"SOL","whiteStatus":true}]"#,
            &request(TransferDirection::WithdrawToChain),
            "official",
            1,
        )
        .expect("withdraw evidence");
        assert_eq!(wrong_case.status, TransferDestinationStatus::Missing);
    }

    #[test]
    fn deposit_address_is_verified_only_when_non_empty() {
        let row = parse_deposit(
            r#"{"address":"deposit-wallet","coin":"USDC","tag":"","url":"https://example"}"#,
            &request(TransferDirection::DepositToVenue),
            "official",
            1,
        )
        .expect("deposit evidence");
        assert_eq!(row.status, TransferDestinationStatus::Verified);
        assert_eq!(row.address.as_deref(), Some("deposit-wallet"));
    }
}
