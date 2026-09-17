//! Binance withdrawal submission and finality evidence.
//!
//! Official contracts:
//! - <https://developers.binance.com/en/docs/catalog/core-trading-wallet/api/rest-api/capital#withdraw-user_data>
//! - <https://developers.binance.com/en/docs/catalog/core-trading-wallet/api/rest-api/capital#withdraw-history-supporting-network-user_data>

use crate::error::{ExchangeError, ExchangeResult};
use crate::http::HttpClient;
use crate::{
    WithdrawalSourceBalance, WithdrawalSourceBalanceRequest, WithdrawalStatus,
    WithdrawalStatusEvidence, WithdrawalStatusRequest, WithdrawalSubmission,
    WithdrawalSubmitRequest, WithdrawalWalletType,
};
use reqwest::Method;
use rust_decimal::Decimal;
use serde::Deserialize;

const WITHDRAW_APPLY_PATH: &str = "/sapi/v1/capital/withdraw/apply";
const WITHDRAW_HISTORY_PATH: &str = "/sapi/v1/capital/withdraw/history";
const CAPITAL_CONFIG_PATH: &str = "/sapi/v1/capital/config/getall";
const HISTORY_WINDOW_MS: i64 = 7 * 24 * 60 * 60 * 1_000;
const HISTORY_CLOCK_SKEW_MS: i64 = 60_000;

#[derive(Debug, Deserialize)]
struct WithdrawAck {
    id: String,
}

#[derive(Debug, Deserialize)]
struct CapitalCoinRow {
    coin: String,
    free: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WithdrawHistoryRow {
    address: String,
    amount: String,
    coin: String,
    id: String,
    withdraw_order_id: String,
    #[serde(default)]
    network: String,
    status: u8,
    transaction_fee: String,
    #[serde(default)]
    confirm_no: Option<u64>,
    #[serde(default)]
    info: String,
    #[serde(default)]
    tx_id: String,
}

pub(super) async fn submit<F>(
    http: &HttpClient,
    base_url: &str,
    request: &WithdrawalSubmitRequest,
    mut signed_query: F,
) -> ExchangeResult<WithdrawalSubmission>
where
    F: FnMut(&[(&str, &str)]) -> ExchangeResult<(String, String)>,
{
    validate_submit(request)?;
    let amount = request.amount.normalize().to_string();
    let mut params = vec![
        ("coin", request.currency.as_str()),
        ("withdrawOrderId", request.client_withdrawal_id.as_str()),
        ("network", request.network.as_str()),
        ("address", request.address.as_str()),
        ("amount", amount.as_str()),
        ("walletType", wallet_type_code(request.wallet_type)),
        ("recvWindow", "5000"),
    ];
    if let Some(tag) = request
        .tag
        .as_deref()
        .map(str::trim)
        .filter(|tag| !tag.is_empty())
    {
        params.push(("addressTag", tag));
    }
    let url = format!("{base_url}{WITHDRAW_APPLY_PATH}");
    let response = http
        .execute_once_fresh(Method::POST, &url, || {
            let (query, api_key) = signed_query(&params)?;
            Ok(http
                .request(Method::POST, format!("{url}?{query}"))
                .header("X-MBX-APIKEY", api_key))
        })
        .await?;
    let body = checked_body(response).await?;
    let ack: WithdrawAck = serde_json::from_str(&body)
        .map_err(|error| ExchangeError::Parse(format!("binance withdrawal ack: {error}")))?;
    let provider_withdrawal_id = required_text("id", &ack.id)?;
    Ok(WithdrawalSubmission {
        venue: "binance".to_owned(),
        provider_withdrawal_id,
        client_withdrawal_id: request.client_withdrawal_id.clone(),
        submitted_at_ms: common::time::now_ms(),
        source_url: url,
        problem: None,
    })
}

pub(super) async fn source_balance<F>(
    http: &HttpClient,
    base_url: &str,
    request: &WithdrawalSourceBalanceRequest,
    mut signed_query: F,
) -> ExchangeResult<WithdrawalSourceBalance>
where
    F: FnMut(&[(&str, &str)]) -> ExchangeResult<(String, String)>,
{
    validate_venue(&request.venue)?;
    required_text("currency", &request.currency)?;
    if request.wallet_type != WithdrawalWalletType::Spot {
        return Err(ExchangeError::UnsupportedCapability(
            "binance funding wallet withdrawal balance",
        ));
    }
    let url = format!("{base_url}{CAPITAL_CONFIG_PATH}");
    let response = http
        .execute_with_retry_fresh(Method::GET, &url, || {
            let (query, api_key) = signed_query(&[("recvWindow", "5000")])?;
            Ok(http
                .request(Method::GET, format!("{url}?{query}"))
                .header("X-MBX-APIKEY", api_key))
        })
        .await?;
    let body = checked_body(response).await?;
    let rows: Vec<CapitalCoinRow> = serde_json::from_str(&body)
        .map_err(|error| ExchangeError::Parse(format!("binance capital balance: {error}")))?;
    let row = rows
        .into_iter()
        .find(|row| row.coin.eq_ignore_ascii_case(request.currency.trim()))
        .ok_or_else(|| {
            ExchangeError::Parse(format!(
                "binance capital balance has no {} row",
                request.currency.trim().to_ascii_uppercase()
            ))
        })?;
    Ok(WithdrawalSourceBalance {
        venue: "binance".to_owned(),
        currency: row.coin.trim().to_ascii_uppercase(),
        wallet_type: request.wallet_type,
        available: parse_decimal("free", &row.free)?,
        checked_at_ms: common::time::now_ms(),
        source_url: url,
    })
}

pub(super) async fn status<F>(
    http: &HttpClient,
    base_url: &str,
    request: &WithdrawalStatusRequest,
    mut signed_query: F,
) -> ExchangeResult<Option<WithdrawalStatusEvidence>>
where
    F: FnMut(&[(&str, &str)]) -> ExchangeResult<(String, String)>,
{
    validate_status(request)?;
    let start_time = request
        .submitted_at_ms
        .saturating_sub(HISTORY_CLOCK_SKEW_MS)
        .max(0);
    let end_time = common::time::now_ms()
        .max(request.submitted_at_ms)
        .min(start_time.saturating_add(HISTORY_WINDOW_MS - 1));
    let start_time = start_time.to_string();
    let end_time = end_time.to_string();
    let params = [
        ("coin", request.currency.as_str()),
        ("withdrawOrderId", request.client_withdrawal_id.as_str()),
        ("startTime", start_time.as_str()),
        ("endTime", end_time.as_str()),
        ("recvWindow", "5000"),
    ];
    let url = format!("{base_url}{WITHDRAW_HISTORY_PATH}");
    let response = http
        .execute_with_retry_fresh(Method::GET, &url, || {
            let (query, api_key) = signed_query(&params)?;
            Ok(http
                .request(Method::GET, format!("{url}?{query}"))
                .header("X-MBX-APIKEY", api_key))
        })
        .await?;
    let body = checked_body(response).await?;
    parse_status(&body, request, &url, common::time::now_ms())
}

fn parse_status(
    body: &str,
    request: &WithdrawalStatusRequest,
    source_url: &str,
    checked_at_ms: i64,
) -> ExchangeResult<Option<WithdrawalStatusEvidence>> {
    let rows: Vec<WithdrawHistoryRow> = serde_json::from_str(body)
        .map_err(|error| ExchangeError::Parse(format!("binance withdrawal history: {error}")))?;
    let Some(row) = rows
        .into_iter()
        .find(|row| row.withdraw_order_id == request.client_withdrawal_id)
    else {
        return Ok(None);
    };
    validate_history_identity(&row, request)?;
    let status = map_status(row.status)?;
    let problem = match status {
        WithdrawalStatus::Cancelled => Some("Binance withdrawal was cancelled".to_owned()),
        WithdrawalStatus::Rejected => Some(
            non_empty(&row.info).unwrap_or_else(|| "Binance withdrawal was rejected".to_owned()),
        ),
        WithdrawalStatus::Failed => {
            Some(non_empty(&row.info).unwrap_or_else(|| "Binance withdrawal failed".to_owned()))
        }
        WithdrawalStatus::Pending | WithdrawalStatus::Completed => non_empty(&row.info),
    };
    Ok(Some(WithdrawalStatusEvidence {
        venue: "binance".to_owned(),
        provider_withdrawal_id: required_text("id", &row.id)?,
        client_withdrawal_id: row.withdraw_order_id,
        currency: row.coin.trim().to_ascii_uppercase(),
        network: row.network.trim().to_owned(),
        address: row.address.trim().to_owned(),
        amount: parse_decimal("amount", &row.amount)?,
        transaction_fee: parse_decimal("transactionFee", &row.transaction_fee)?,
        status,
        transaction_id: non_empty(&row.tx_id),
        confirmations: row.confirm_no,
        checked_at_ms,
        source_url: source_url.to_owned(),
        problem,
    }))
}

fn validate_submit(request: &WithdrawalSubmitRequest) -> ExchangeResult<()> {
    validate_venue(&request.venue)?;
    required_text("currency", &request.currency)?;
    required_text("network", &request.network)?;
    required_text("address", &request.address)?;
    required_text("clientWithdrawalId", &request.client_withdrawal_id)?;
    if request.amount <= Decimal::ZERO {
        return Err(ExchangeError::Parse(
            "binance withdrawal amount must be positive".to_owned(),
        ));
    }
    Ok(())
}

fn wallet_type_code(wallet_type: WithdrawalWalletType) -> &'static str {
    match wallet_type {
        WithdrawalWalletType::Spot => "0",
        WithdrawalWalletType::Funding => "1",
    }
}

fn validate_status(request: &WithdrawalStatusRequest) -> ExchangeResult<()> {
    validate_venue(&request.venue)?;
    required_text("currency", &request.currency)?;
    required_text("network", &request.network)?;
    required_text("address", &request.address)?;
    required_text("clientWithdrawalId", &request.client_withdrawal_id)?;
    if request.submitted_at_ms <= 0 {
        return Err(ExchangeError::Parse(
            "binance withdrawal submittedAtMs must be positive".to_owned(),
        ));
    }
    Ok(())
}

fn validate_history_identity(
    row: &WithdrawHistoryRow,
    request: &WithdrawalStatusRequest,
) -> ExchangeResult<()> {
    let currency_matches = row.coin.eq_ignore_ascii_case(request.currency.trim());
    let network_matches = row.network.eq_ignore_ascii_case(request.network.trim());
    let address_matches = if crate::canonical_network_id(&row.network) == "solana" {
        row.address.trim() == request.address.trim()
    } else {
        row.address
            .trim()
            .eq_ignore_ascii_case(request.address.trim())
    };
    if currency_matches && network_matches && address_matches {
        Ok(())
    } else {
        Err(ExchangeError::Parse(
            "binance withdrawal history identity does not match the authorized request".to_owned(),
        ))
    }
}

fn validate_venue(venue: &str) -> ExchangeResult<()> {
    if venue.trim().eq_ignore_ascii_case("binance") {
        Ok(())
    } else {
        Err(ExchangeError::UnsupportedSymbol(format!(
            "binance withdrawal venue={venue}"
        )))
    }
}

fn map_status(status: u8) -> ExchangeResult<WithdrawalStatus> {
    match status {
        0 | 2 | 4 => Ok(WithdrawalStatus::Pending),
        1 => Ok(WithdrawalStatus::Cancelled),
        3 => Ok(WithdrawalStatus::Rejected),
        5 => Ok(WithdrawalStatus::Failed),
        6 => Ok(WithdrawalStatus::Completed),
        other => Err(ExchangeError::Parse(format!(
            "binance withdrawal status is unsupported: {other}"
        ))),
    }
}

fn parse_decimal(field: &str, value: &str) -> ExchangeResult<Decimal> {
    value
        .trim()
        .parse()
        .map_err(|error| ExchangeError::Parse(format!("binance withdrawal {field}: {error}")))
}

fn required_text(field: &str, value: &str) -> ExchangeResult<String> {
    non_empty(value)
        .ok_or_else(|| ExchangeError::Parse(format!("binance withdrawal {field} is empty")))
}

fn non_empty(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

async fn checked_body(response: reqwest::Response) -> ExchangeResult<String> {
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| ExchangeError::Network(error.to_string()))?;
    if status.is_success() {
        Ok(body)
    } else {
        Err(ExchangeError::Http {
            status: status.as_u16(),
            body,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn signer(params: &[(&str, &str)]) -> ExchangeResult<(String, String)> {
        let mut query = url::form_urlencoded::Serializer::new(String::new());
        query.extend_pairs(params.iter().copied());
        query.append_pair("timestamp", "1700000000000");
        query.append_pair("signature", "signed");
        Ok((query.finish(), "key".to_owned()))
    }

    fn submit_request() -> WithdrawalSubmitRequest {
        WithdrawalSubmitRequest {
            venue: "binance".to_owned(),
            currency: "USDC".to_owned(),
            network: "SOL".to_owned(),
            address: "SolanaAddress".to_owned(),
            tag: None,
            amount: Decimal::new(125, 1),
            client_withdrawal_id: "run-1".to_owned(),
            wallet_type: WithdrawalWalletType::Spot,
            max_fee: None,
        }
    }

    #[tokio::test]
    async fn submission_uses_official_single_attempt_contract() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(WITHDRAW_APPLY_PATH))
            .and(header("X-MBX-APIKEY", "key"))
            .and(query_param("coin", "USDC"))
            .and(query_param("withdrawOrderId", "run-1"))
            .and(query_param("network", "SOL"))
            .and(query_param("address", "SolanaAddress"))
            .and(query_param("amount", "12.5"))
            .and(query_param("walletType", "0"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(r#"{"id":"provider-1"}"#, "application/json"),
            )
            .expect(1)
            .mount(&server)
            .await;
        let http = HttpClient::builder("binance")
            .max_retries(3)
            .build()
            .expect("http client");
        let result = submit(&http, &server.uri(), &submit_request(), signer)
            .await
            .expect("withdrawal submission");
        assert_eq!(result.provider_withdrawal_id, "provider-1");
    }

    #[tokio::test]
    async fn history_requires_exact_identity_and_maps_completed() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(WITHDRAW_HISTORY_PATH))
            .and(header("X-MBX-APIKEY", "key"))
            .and(query_param("coin", "USDC"))
            .and(query_param("withdrawOrderId", "run-1"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                r#"[{"address":"SolanaAddress","amount":"12.5","coin":"USDC","id":"provider-1","withdrawOrderId":"run-1","network":"SOL","status":6,"transactionFee":"0.1","confirmNo":3,"info":"","txId":"tx-1"}]"#,
                "application/json",
            ))
            .mount(&server)
            .await;
        let request = WithdrawalStatusRequest {
            venue: "binance".to_owned(),
            currency: "USDC".to_owned(),
            network: "SOL".to_owned(),
            address: "SolanaAddress".to_owned(),
            client_withdrawal_id: "run-1".to_owned(),
            tag: None,
            provider_withdrawal_id: None,
            submitted_at_ms: 1_700_000_000_000,
        };
        let http = HttpClient::builder("binance")
            .max_retries(1)
            .build()
            .expect("http client");
        let result = status(&http, &server.uri(), &request, signer)
            .await
            .expect("withdrawal history")
            .expect("matching withdrawal");
        assert_eq!(result.status, WithdrawalStatus::Completed);
        assert_eq!(result.transaction_id.as_deref(), Some("tx-1"));
    }

    #[tokio::test]
    async fn source_balance_uses_spot_wallet_capital_contract() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(CAPITAL_CONFIG_PATH))
            .and(header("X-MBX-APIKEY", "key"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(r#"[{"coin":"USDC","free":"42.125"}]"#, "application/json"),
            )
            .mount(&server)
            .await;
        let request = WithdrawalSourceBalanceRequest {
            venue: "binance".to_owned(),
            currency: "USDC".to_owned(),
            wallet_type: WithdrawalWalletType::Spot,
        };
        let http = HttpClient::builder("binance")
            .max_retries(1)
            .build()
            .expect("http client");

        let balance = source_balance(&http, &server.uri(), &request, signer)
            .await
            .expect("source balance");

        assert_eq!(balance.available, Decimal::new(42_125, 3));
        assert_eq!(balance.wallet_type, WithdrawalWalletType::Spot);
    }
}
