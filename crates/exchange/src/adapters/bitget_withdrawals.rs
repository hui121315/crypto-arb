//! Bitget UTA withdrawal submission and finality evidence.
//!
//! Official contracts:
//! - <https://www.bitget.com/api-doc/uta/account/withdrawal>
//! - <https://www.bitget.com/api-doc/uta/account/withdrawal/Get-Withdrawal-Records>
//! - <https://www.bitget.com/api-doc/uta/account/Get-Account-Assets>

use super::bitget_response::{api_error_from_body, BitgetObjectResponse, BitgetResponse};
use super::bitget_uta_private_rest::SignedHeaders;
use crate::error::{ExchangeError, ExchangeResult};
use crate::http::HttpClient;
use crate::{
    WithdrawalSourceBalance, WithdrawalSourceBalanceRequest, WithdrawalStatus,
    WithdrawalStatusEvidence, WithdrawalStatusRequest, WithdrawalSubmission,
    WithdrawalSubmitRequest, WithdrawalWalletType,
};
use reqwest::Method;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

const WITHDRAW_PATH: &str = "/api/v3/account/withdrawal";
const WITHDRAW_HISTORY_PATH: &str = "/api/v3/account/withdrawal-records";
const ACCOUNT_ASSETS_PATH: &str = "/api/v3/account/assets";
const HISTORY_CLOCK_SKEW_MS: i64 = 5 * 60_000;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WithdrawBody<'a> {
    coin: &'a str,
    chain: &'a str,
    transfer_type: &'static str,
    address: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    tag: Option<&'a str>,
    size: String,
    client_oid: &'a str,
    account_type: &'static str,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WithdrawAck {
    order_id: String,
    client_oid: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WithdrawHistoryRow {
    order_id: String,
    client_oid: String,
    record_id: String,
    coin: String,
    #[serde(rename = "type")]
    operation_type: String,
    dest: String,
    size: String,
    status: String,
    to_address: String,
    chain: String,
    fee: String,
    confirm: String,
}

#[derive(Debug, Deserialize)]
struct AccountAssetsPayload {
    assets: Vec<AccountAssetRow>,
}

#[derive(Debug, Deserialize)]
struct AccountAssetRow {
    coin: String,
    available: String,
}

pub(super) async fn submit<F>(
    http: &HttpClient,
    base_url: &str,
    request: &WithdrawalSubmitRequest,
    mut signed_headers: F,
) -> ExchangeResult<WithdrawalSubmission>
where
    F: FnMut(&str, &str) -> ExchangeResult<SignedHeaders>,
{
    validate_submit(request)?;
    let tag = request
        .tag
        .as_deref()
        .map(str::trim)
        .filter(|tag| !tag.is_empty());
    let body = serde_json::to_string(&WithdrawBody {
        coin: request.currency.trim(),
        chain: request.network.trim(),
        transfer_type: "on_chain",
        address: request.address.trim(),
        tag,
        size: request.amount.normalize().to_string(),
        client_oid: request.client_withdrawal_id.trim(),
        account_type: "uta",
    })
    .map_err(|error| ExchangeError::Parse(format!("bitget withdrawal body: {error}")))?;
    let source_url = format!("{base_url}{WITHDRAW_PATH}");
    let response = http
        .execute_once_fresh(Method::POST, &source_url, || {
            let headers = signed_headers(WITHDRAW_PATH, &body)?;
            let mut request = http
                .request(Method::POST, &source_url)
                .header("Content-Type", "application/json")
                .body(body.clone());
            for (key, value) in headers {
                request = request.header(key, value);
            }
            Ok(request)
        })
        .await?;
    let body = checked_body(response, "withdrawal").await?;
    let response: BitgetObjectResponse<WithdrawAck> = serde_json::from_str(&body)
        .map_err(|error| ExchangeError::Parse(format!("bitget withdrawal ack: {error}")))?;
    let ack = response.into_result("withdrawal")?;
    if ack.client_oid != request.client_withdrawal_id {
        return Err(ExchangeError::Parse(
            "bitget withdrawal ack returned a different clientOid".to_owned(),
        ));
    }
    Ok(WithdrawalSubmission {
        venue: "bitget".to_owned(),
        provider_withdrawal_id: required_text("orderId", &ack.order_id)?.to_owned(),
        client_withdrawal_id: ack.client_oid,
        submitted_at_ms: common::time::now_ms(),
        source_url,
        problem: None,
    })
}

pub(super) async fn status<F>(
    http: &HttpClient,
    base_url: &str,
    request: &WithdrawalStatusRequest,
    mut signed_headers: F,
) -> ExchangeResult<Option<WithdrawalStatusEvidence>>
where
    F: FnMut(&str, &str) -> ExchangeResult<SignedHeaders>,
{
    validate_status(request)?;
    let start_time = request
        .submitted_at_ms
        .saturating_sub(HISTORY_CLOCK_SKEW_MS)
        .max(0)
        .to_string();
    let end_time = common::time::now_ms()
        .max(request.submitted_at_ms)
        .to_string();
    let query = form_query(&[
        ("coin", request.currency.as_str()),
        ("clientOid", request.client_withdrawal_id.as_str()),
        ("startTime", start_time.as_str()),
        ("endTime", end_time.as_str()),
        ("limit", "100"),
    ]);
    let path = format!("{WITHDRAW_HISTORY_PATH}?{query}");
    let source_url = format!("{base_url}{WITHDRAW_HISTORY_PATH}");
    let body = signed_get(
        http,
        base_url,
        &path,
        &source_url,
        "withdrawal records",
        &mut signed_headers,
    )
    .await?;
    let response: BitgetResponse<WithdrawHistoryRow> = serde_json::from_str(&body)
        .map_err(|error| ExchangeError::Parse(format!("bitget withdrawal records: {error}")))?;
    let Some(row) = response
        .into_data("withdrawal records")?
        .into_iter()
        .find(|row| row.client_oid == request.client_withdrawal_id)
    else {
        return Ok(None);
    };
    parse_status(row, request, &source_url, common::time::now_ms()).map(Some)
}

pub(super) async fn source_balance<F>(
    http: &HttpClient,
    base_url: &str,
    request: &WithdrawalSourceBalanceRequest,
    mut signed_headers: F,
) -> ExchangeResult<WithdrawalSourceBalance>
where
    F: FnMut(&str, &str) -> ExchangeResult<SignedHeaders>,
{
    validate_venue(&request.venue)?;
    required_text("currency", &request.currency)?;
    if request.wallet_type != WithdrawalWalletType::Spot {
        return Err(ExchangeError::UnsupportedCapability(
            "bitget UTA funding-wallet withdrawal balance",
        ));
    }
    let source_url = format!("{base_url}{ACCOUNT_ASSETS_PATH}");
    let body = signed_get(
        http,
        base_url,
        ACCOUNT_ASSETS_PATH,
        &source_url,
        "account assets",
        &mut signed_headers,
    )
    .await?;
    let response: BitgetObjectResponse<AccountAssetsPayload> = serde_json::from_str(&body)
        .map_err(|error| ExchangeError::Parse(format!("bitget account assets: {error}")))?;
    let row = response
        .into_result("account assets")?
        .assets
        .into_iter()
        .find(|row| row.coin.eq_ignore_ascii_case(request.currency.trim()))
        .ok_or_else(|| {
            ExchangeError::Parse(format!(
                "bitget account assets has no {} row",
                request.currency.trim().to_ascii_uppercase()
            ))
        })?;
    Ok(WithdrawalSourceBalance {
        venue: "bitget".to_owned(),
        currency: row.coin.trim().to_ascii_uppercase(),
        wallet_type: request.wallet_type,
        available: parse_decimal("available", &row.available)?,
        checked_at_ms: common::time::now_ms(),
        source_url,
    })
}

fn parse_status(
    row: WithdrawHistoryRow,
    request: &WithdrawalStatusRequest,
    source_url: &str,
    checked_at_ms: i64,
) -> ExchangeResult<WithdrawalStatusEvidence> {
    validate_history_identity(&row, request)?;
    let (status, problem) = match row.status.trim().to_ascii_lowercase().as_str() {
        "pending" => (
            WithdrawalStatus::Pending,
            Some("Bitget 提币仍在处理中".to_owned()),
        ),
        "success" => (WithdrawalStatus::Completed, None),
        "fail" => (
            WithdrawalStatus::Failed,
            Some("Bitget 官方提币记录报告失败".to_owned()),
        ),
        other => {
            return Err(ExchangeError::Parse(format!(
                "bitget withdrawal status is unsupported: {other}"
            )))
        }
    };
    let transaction_id = (row.dest.eq_ignore_ascii_case("on_chain"))
        .then(|| non_empty(&row.record_id))
        .flatten();
    Ok(WithdrawalStatusEvidence {
        venue: "bitget".to_owned(),
        provider_withdrawal_id: required_text("orderId", &row.order_id)?.to_owned(),
        client_withdrawal_id: row.client_oid,
        currency: row.coin.trim().to_ascii_uppercase(),
        network: row.chain.trim().to_owned(),
        address: row.to_address.trim().to_owned(),
        amount: parse_decimal("size", &row.size)?,
        transaction_fee: parse_decimal("fee", &row.fee)?.abs(),
        status,
        transaction_id,
        confirmations: row.confirm.trim().parse::<u64>().ok(),
        checked_at_ms,
        source_url: source_url.to_owned(),
        problem,
    })
}

fn validate_submit(request: &WithdrawalSubmitRequest) -> ExchangeResult<()> {
    validate_venue(&request.venue)?;
    required_text("currency", &request.currency)?;
    required_text("network", &request.network)?;
    required_text("address", &request.address)?;
    required_text("clientOid", &request.client_withdrawal_id)?;
    if request.wallet_type != WithdrawalWalletType::Spot {
        return Err(ExchangeError::UnsupportedCapability(
            "bitget UTA funding-wallet withdrawal",
        ));
    }
    if request.amount <= Decimal::ZERO {
        return Err(ExchangeError::Parse(
            "bitget withdrawal amount must be positive".to_owned(),
        ));
    }
    Ok(())
}

fn validate_status(request: &WithdrawalStatusRequest) -> ExchangeResult<()> {
    validate_venue(&request.venue)?;
    required_text("currency", &request.currency)?;
    required_text("network", &request.network)?;
    required_text("address", &request.address)?;
    required_text("clientOid", &request.client_withdrawal_id)?;
    if request.submitted_at_ms <= 0 {
        return Err(ExchangeError::Parse(
            "bitget withdrawal submittedAtMs must be positive".to_owned(),
        ));
    }
    Ok(())
}

fn validate_history_identity(
    row: &WithdrawHistoryRow,
    request: &WithdrawalStatusRequest,
) -> ExchangeResult<()> {
    let address_matches = if crate::canonical_network_id(&row.chain) == "solana" {
        row.to_address.trim() == request.address.trim()
    } else {
        row.to_address
            .trim()
            .eq_ignore_ascii_case(request.address.trim())
    };
    if row.operation_type.eq_ignore_ascii_case("withdraw")
        && row.dest.eq_ignore_ascii_case("on_chain")
        && row.coin.eq_ignore_ascii_case(request.currency.trim())
        && crate::canonical_network_id(&row.chain) == crate::canonical_network_id(&request.network)
        && address_matches
    {
        Ok(())
    } else {
        Err(ExchangeError::Parse(
            "bitget withdrawal history identity does not match the authorized request".to_owned(),
        ))
    }
}

async fn signed_get<F>(
    http: &HttpClient,
    base_url: &str,
    path: &str,
    source_url: &str,
    context: &str,
    signed_headers: &mut F,
) -> ExchangeResult<String>
where
    F: FnMut(&str, &str) -> ExchangeResult<SignedHeaders>,
{
    let url = format!("{base_url}{path}");
    let response = http
        .execute_with_retry_fresh(Method::GET, source_url, || {
            let headers = signed_headers(path, "")?;
            let mut request = http.request(Method::GET, &url);
            for (key, value) in headers {
                request = request.header(key, value);
            }
            Ok(request)
        })
        .await?;
    checked_body(response, context).await
}

fn form_query(params: &[(&str, &str)]) -> String {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    serializer.extend_pairs(params.iter().copied());
    serializer.finish()
}

fn validate_venue(venue: &str) -> ExchangeResult<()> {
    if venue.trim().eq_ignore_ascii_case("bitget") {
        Ok(())
    } else {
        Err(ExchangeError::UnsupportedSymbol(format!(
            "bitget withdrawal venue={venue}"
        )))
    }
}

fn required_text<'a>(field: &str, value: &'a str) -> ExchangeResult<&'a str> {
    let value = value.trim();
    if value.is_empty() {
        Err(ExchangeError::Parse(format!(
            "bitget withdrawal {field} is empty"
        )))
    } else {
        Ok(value)
    }
}

fn parse_decimal(field: &str, value: &str) -> ExchangeResult<Decimal> {
    value
        .trim()
        .parse()
        .map_err(|error| ExchangeError::Parse(format!("bitget withdrawal {field}: {error}")))
}

fn non_empty(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

async fn checked_body(response: reqwest::Response, context: &str) -> ExchangeResult<String> {
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| ExchangeError::Network(error.to_string()))?;
    if status.is_success() {
        Ok(body)
    } else if let Some(error) = api_error_from_body(&body, context) {
        Err(error)
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
    use wiremock::matchers::{body_json, header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn headers(_path: &str, _body: &str) -> ExchangeResult<SignedHeaders> {
        Ok([
            ("ACCESS-KEY".to_owned(), "key".to_owned()),
            ("ACCESS-SIGN".to_owned(), "sign".to_owned()),
            ("ACCESS-PASSPHRASE".to_owned(), "pass".to_owned()),
            ("ACCESS-TIMESTAMP".to_owned(), "1700000000000".to_owned()),
            ("locale".to_owned(), "en-US".to_owned()),
        ])
    }

    fn submit_request() -> WithdrawalSubmitRequest {
        WithdrawalSubmitRequest {
            venue: "bitget".to_owned(),
            currency: "USDC".to_owned(),
            network: "SOL".to_owned(),
            address: "SolanaAddress".to_owned(),
            tag: None,
            amount: Decimal::new(125, 1),
            client_withdrawal_id: "crossline-1".to_owned(),
            wallet_type: WithdrawalWalletType::Spot,
            max_fee: None,
        }
    }

    #[tokio::test]
    async fn submission_uses_uta_single_attempt_and_client_oid() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(WITHDRAW_PATH))
            .and(header("ACCESS-KEY", "key"))
            .and(body_json(serde_json::json!({
                "coin": "USDC",
                "chain": "SOL",
                "transferType": "on_chain",
                "address": "SolanaAddress",
                "size": "12.5",
                "clientOid": "crossline-1",
                "accountType": "uta"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                r#"{"code":"00000","msg":"success","data":{"orderId":"provider-1","clientOid":"crossline-1"}}"#,
                "application/json",
            ))
            .expect(1)
            .mount(&server)
            .await;
        let http = HttpClient::builder("bitget")
            .max_retries(3)
            .build()
            .expect("http client");

        let result = submit(&http, &server.uri(), &submit_request(), headers)
            .await
            .expect("withdrawal submission");

        assert_eq!(result.provider_withdrawal_id, "provider-1");
    }

    #[tokio::test]
    async fn history_maps_success_and_requires_exact_scope() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(WITHDRAW_HISTORY_PATH))
            .and(query_param("coin", "USDC"))
            .and(query_param("clientOid", "crossline-1"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                r#"{"code":"00000","msg":"success","data":[{"orderId":"provider-1","clientOid":"crossline-1","recordId":"tx-1","coin":"USDC","type":"withdraw","dest":"on_chain","size":"12.5","status":"success","toAddress":"SolanaAddress","chain":"SOL","fee":"0.1","confirm":"3"}]}"#,
                "application/json",
            ))
            .mount(&server)
            .await;
        let request = WithdrawalStatusRequest {
            venue: "bitget".to_owned(),
            currency: "USDC".to_owned(),
            network: "SOL".to_owned(),
            address: "SolanaAddress".to_owned(),
            client_withdrawal_id: "crossline-1".to_owned(),
            tag: None,
            provider_withdrawal_id: None,
            submitted_at_ms: 1_700_000_000_000,
        };
        let http = HttpClient::builder("bitget")
            .max_retries(1)
            .build()
            .expect("http client");

        let result = status(&http, &server.uri(), &request, headers)
            .await
            .expect("withdrawal history")
            .expect("matching row");

        assert_eq!(result.status, WithdrawalStatus::Completed);
        assert_eq!(result.transaction_id.as_deref(), Some("tx-1"));
        assert_eq!(result.confirmations, Some(3));
    }

    #[tokio::test]
    async fn source_balance_preserves_exact_uta_decimal() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(ACCOUNT_ASSETS_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                r#"{"code":"00000","msg":"success","data":{"assets":[{"coin":"USDC","available":"42.12500001"}]}}"#,
                "application/json",
            ))
            .mount(&server)
            .await;
        let request = WithdrawalSourceBalanceRequest {
            venue: "bitget".to_owned(),
            currency: "USDC".to_owned(),
            wallet_type: WithdrawalWalletType::Spot,
        };
        let http = HttpClient::builder("bitget")
            .max_retries(1)
            .build()
            .expect("http client");

        let result = source_balance(&http, &server.uri(), &request, headers)
            .await
            .expect("source balance");

        assert_eq!(result.available, Decimal::new(4_212_500_001, 8));
    }
}
