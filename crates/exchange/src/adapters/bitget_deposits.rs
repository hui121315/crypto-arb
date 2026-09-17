//! Bitget UTA deposit address and finality evidence.
//!
//! Official contracts:
//! - <https://www.bitget.com/api-doc/uta/account/deposit/Get-Deposit-Address>
//! - <https://www.bitget.com/api-doc/uta/account/deposit/Get-Deposit-Records>
//! - <https://www.bitget.com/api-doc/uta/account/withdrawal/Get-Withdraw-Address>

use super::bitget_response::{api_error_from_body, BitgetObjectResponse, BitgetResponse};
use super::bitget_uta_private_rest::SignedHeaders;
use crate::error::{ExchangeError, ExchangeResult};
use crate::http::HttpClient;
use crate::{
    canonical_network_id, DepositStatus, DepositStatusEvidence, DepositStatusRequest,
    TransferDestinationEvidence, TransferDestinationRequest, TransferDestinationStatus,
    TransferDirection,
};
use reqwest::Method;
use rust_decimal::Decimal;
use serde::Deserialize;

const DEPOSIT_ADDRESS_PATH: &str = "/api/v3/account/deposit-address";
const DEPOSIT_HISTORY_PATH: &str = "/api/v3/account/deposit-records";
const WITHDRAW_ADDRESS_PATH: &str = "/api/v3/account/withdraw-address";
const HISTORY_CLOCK_SKEW_MS: i64 = 5 * 60_000;
const HISTORY_PAGE_LIMIT: usize = 100;
const MAX_HISTORY_PAGES: usize = 5;
const WITHDRAW_ADDRESS_PAGE_LIMIT: usize = 10;
const MAX_WITHDRAW_ADDRESS_PAGES: usize = 5;
const WITHDRAW_ADDRESS_RATE_LIMIT_MS: u64 = 1_050;

#[derive(Debug, Deserialize)]
struct DepositAddressRow {
    address: String,
    chain: String,
    coin: String,
    #[serde(default)]
    tag: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WithdrawAddressPage {
    #[serde(default)]
    address_list: Vec<WithdrawAddressRow>,
    #[serde(default)]
    cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WithdrawAddressRow {
    coin: String,
    chain: String,
    address: String,
    #[serde(default)]
    memo: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DepositHistoryRow {
    order_id: String,
    record_id: String,
    coin: String,
    #[serde(rename = "type")]
    operation_type: String,
    dest: String,
    size: String,
    status: String,
    to_address: String,
    chain: String,
    created_time: String,
}

pub(super) async fn destination<F>(
    http: &HttpClient,
    base_url: &str,
    request: &TransferDestinationRequest,
    mut signed_headers: F,
) -> ExchangeResult<TransferDestinationEvidence>
where
    F: FnMut(&str) -> ExchangeResult<SignedHeaders>,
{
    match request.direction {
        TransferDirection::DepositToVenue => {
            deposit_destination(http, base_url, request, &mut signed_headers).await
        }
        TransferDirection::WithdrawToChain => {
            withdraw_destination(http, base_url, request, &mut signed_headers).await
        }
    }
}

async fn deposit_destination<F>(
    http: &HttpClient,
    base_url: &str,
    request: &TransferDestinationRequest,
    signed_headers: &mut F,
) -> ExchangeResult<TransferDestinationEvidence>
where
    F: FnMut(&str) -> ExchangeResult<SignedHeaders>,
{
    required_text("currency", &request.currency)?;
    required_text("network", &request.network)?;
    let query = form_query(&[
        ("coin", request.currency.as_str()),
        ("chain", request.network.as_str()),
    ]);
    let path = format!("{DEPOSIT_ADDRESS_PATH}?{query}");
    let source_url = format!("{base_url}{DEPOSIT_ADDRESS_PATH}");
    let body = signed_get(http, base_url, &path, &source_url, signed_headers).await?;
    parse_destination(&body, request, &source_url, common::time::now_ms())
}

async fn withdraw_destination<F>(
    http: &HttpClient,
    base_url: &str,
    request: &TransferDestinationRequest,
    signed_headers: &mut F,
) -> ExchangeResult<TransferDestinationEvidence>
where
    F: FnMut(&str) -> ExchangeResult<SignedHeaders>,
{
    required_text("currency", &request.currency)?;
    required_text("network", &request.network)?;
    let expected_address = request
        .expected_address
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            ExchangeError::Parse("bitget expected withdrawal address is empty".to_owned())
        })?;
    let address_type = if is_evm_address(expected_address) {
        "EVM"
    } else {
        "regular"
    };
    let source_url = format!("{base_url}{WITHDRAW_ADDRESS_PATH}");
    let mut cursor: Option<String> = None;
    for page_index in 0..MAX_WITHDRAW_ADDRESS_PAGES {
        if page_index > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(
                WITHDRAW_ADDRESS_RATE_LIMIT_MS,
            ))
            .await;
        }
        let mut params = vec![
            ("coin", request.currency.as_str()),
            ("type", address_type),
            ("limit", "10"),
        ];
        if let Some(value) = cursor.as_deref() {
            params.push(("cursor", value));
        }
        let path = format!("{WITHDRAW_ADDRESS_PATH}?{}", form_query(&params));
        let body = signed_get(http, base_url, &path, &source_url, signed_headers).await?;
        let response: BitgetObjectResponse<WithdrawAddressPage> = serde_json::from_str(&body)
            .map_err(|error| {
                ExchangeError::Parse(format!("bitget withdraw address book: {error}"))
            })?;
        let page = response.into_result("withdraw address book")?;
        if let Some(row) = page.address_list.iter().find(|row| {
            row.coin.eq_ignore_ascii_case(request.currency.trim())
                && canonical_network_id(&row.chain) == canonical_network_id(&request.network)
                && addresses_match(&row.address, expected_address, &row.chain)
                && tags_match(&row.memo, request.expected_tag.as_deref())
        }) {
            return Ok(TransferDestinationEvidence {
                venue: "bitget".to_owned(),
                currency: row.coin.trim().to_ascii_uppercase(),
                network: row.chain.trim().to_owned(),
                direction: request.direction,
                address: Some(row.address.trim().to_owned()),
                tag: non_empty(&row.memo),
                status: TransferDestinationStatus::Verified,
                allowlisted: Some(true),
                checked_at_ms: common::time::now_ms(),
                source_url,
                problem: None,
            });
        }
        let next_cursor = page.cursor.as_deref().and_then(non_empty);
        if page.address_list.len() < WITHDRAW_ADDRESS_PAGE_LIMIT
            || next_cursor.as_deref() == cursor.as_deref()
        {
            break;
        }
        cursor = next_cursor;
    }
    Ok(TransferDestinationEvidence {
        venue: "bitget".to_owned(),
        currency: request.currency.trim().to_ascii_uppercase(),
        network: request.network.trim().to_owned(),
        direction: request.direction,
        address: Some(expected_address.to_owned()),
        tag: request.expected_tag.clone(),
        status: TransferDestinationStatus::Missing,
        allowlisted: Some(false),
        checked_at_ms: common::time::now_ms(),
        source_url,
        problem: Some("Bitget 官方提币地址簿未找到完全一致的币种、网络、地址与 memo".to_owned()),
    })
}

pub(super) async fn status<F>(
    http: &HttpClient,
    base_url: &str,
    request: &DepositStatusRequest,
    mut signed_headers: F,
) -> ExchangeResult<Option<DepositStatusEvidence>>
where
    F: FnMut(&str) -> ExchangeResult<SignedHeaders>,
{
    validate_status_request(request)?;
    let start_time = request
        .submitted_at_ms
        .saturating_sub(HISTORY_CLOCK_SKEW_MS)
        .max(0)
        .to_string();
    let end_time = common::time::now_ms()
        .max(request.submitted_at_ms)
        .to_string();
    let source_url = format!("{base_url}{DEPOSIT_HISTORY_PATH}");
    let mut cursor: Option<String> = None;
    let mut history = Vec::new();

    for _ in 0..MAX_HISTORY_PAGES {
        let mut params = vec![
            ("coin", request.currency.as_str()),
            ("startTime", start_time.as_str()),
            ("endTime", end_time.as_str()),
            ("limit", "100"),
        ];
        if let Some(value) = cursor.as_deref() {
            params.push(("cursor", value));
        }
        let query = form_query(&params);
        let path = format!("{DEPOSIT_HISTORY_PATH}?{query}");
        let body = signed_get(http, base_url, &path, &source_url, &mut signed_headers).await?;
        let rows = parse_history_page(&body)?;
        let page_len = rows.len();
        if page_len > HISTORY_PAGE_LIMIT {
            return Err(ExchangeError::Parse(
                "bitget deposit page exceeds requested limit".to_owned(),
            ));
        }
        let next_cursor = history_cursor(&rows);
        history.extend(rows);
        if page_len < HISTORY_PAGE_LIMIT {
            return matching_status(&history, request, &source_url, common::time::now_ms());
        }
        if next_cursor.is_none() || next_cursor.as_deref() == cursor.as_deref() {
            break;
        }
        cursor = next_cursor;
    }
    Err(ExchangeError::Parse(
        "bitget deposit history pagination is incomplete; cannot prove a unique deposit".to_owned(),
    ))
}

async fn signed_get<F>(
    http: &HttpClient,
    base_url: &str,
    path: &str,
    source_url: &str,
    signed_headers: &mut F,
) -> ExchangeResult<String>
where
    F: FnMut(&str) -> ExchangeResult<SignedHeaders>,
{
    let url = format!("{base_url}{path}");
    let response = http
        .execute_with_retry_fresh(Method::GET, source_url, || {
            let headers = signed_headers(path)?;
            let mut request = http.request(Method::GET, &url);
            for (key, value) in headers {
                request = request.header(key, value);
            }
            Ok(request)
        })
        .await?;
    checked_body(response, path).await
}

fn parse_destination(
    body: &str,
    request: &TransferDestinationRequest,
    source_url: &str,
    checked_at_ms: i64,
) -> ExchangeResult<TransferDestinationEvidence> {
    let response: BitgetObjectResponse<DepositAddressRow> = serde_json::from_str(body)
        .map_err(|error| ExchangeError::Parse(format!("bitget deposit address: {error}")))?;
    let row = response.into_result("deposit address")?;
    if !row.coin.eq_ignore_ascii_case(request.currency.trim())
        || !row.chain.eq_ignore_ascii_case(request.network.trim())
    {
        return Err(ExchangeError::Parse(
            "bitget deposit address identity does not match requested coin and chain".to_owned(),
        ));
    }
    let address = non_empty(&row.address);
    let status = if address.is_some() {
        TransferDestinationStatus::Verified
    } else {
        TransferDestinationStatus::Missing
    };
    Ok(TransferDestinationEvidence {
        venue: "bitget".to_owned(),
        currency: row.coin.trim().to_ascii_uppercase(),
        network: row.chain.trim().to_owned(),
        direction: request.direction,
        address,
        tag: row.tag.as_deref().and_then(non_empty),
        status,
        allowlisted: None,
        checked_at_ms,
        source_url: source_url.to_owned(),
        problem: (status == TransferDestinationStatus::Missing)
            .then(|| "Bitget 官方接口未返回充值地址".to_owned()),
    })
}

fn parse_history_page(body: &str) -> ExchangeResult<Vec<DepositHistoryRow>> {
    let response: BitgetResponse<DepositHistoryRow> = serde_json::from_str(body)
        .map_err(|error| ExchangeError::Parse(format!("bitget deposit records: {error}")))?;
    response.into_data("deposit records")
}

fn matching_status(
    rows: &[DepositHistoryRow],
    request: &DepositStatusRequest,
    source_url: &str,
    checked_at_ms: i64,
) -> ExchangeResult<Option<DepositStatusEvidence>> {
    let candidates = rows.iter().filter(|row| {
        transaction_ids_match(&row.record_id, &request.transaction_id, &request.network)
    });
    let mut matching = candidates
        .clone()
        .filter(|row| validate_history_identity(row, request).is_ok());
    let Some(row) = matching.next() else {
        if candidates.count() > 0 {
            return Err(ExchangeError::Parse(
                "bitget deposit history identity does not match the authorized chain transfer"
                    .to_owned(),
            ));
        }
        return Ok(None);
    };
    if matching.next().is_some() {
        return Err(ExchangeError::Parse(
            "bitget deposit history has ambiguous matching records".to_owned(),
        ));
    }
    parse_status(row, request, source_url, checked_at_ms).map(Some)
}

fn parse_status(
    row: &DepositHistoryRow,
    request: &DepositStatusRequest,
    source_url: &str,
    checked_at_ms: i64,
) -> ExchangeResult<DepositStatusEvidence> {
    validate_history_identity(row, request)?;
    let (status, problem) = match row.status.trim().to_ascii_lowercase().as_str() {
        "pending" => (
            DepositStatus::Pending,
            Some("Bitget 充值记录仍在链上确认中".to_owned()),
        ),
        "success" => (DepositStatus::Completed, None),
        "fail" => (
            DepositStatus::Failed,
            Some("Bitget 官方充值记录报告失败".to_owned()),
        ),
        other => {
            return Err(ExchangeError::Parse(format!(
                "bitget deposit status is unsupported: {other}"
            )))
        }
    };
    Ok(DepositStatusEvidence {
        venue: "bitget".to_owned(),
        currency: row.coin.trim().to_ascii_uppercase(),
        network: row.chain.trim().to_owned(),
        address: row.to_address.trim().to_owned(),
        tag: None,
        amount: parse_decimal("size", &row.size)?,
        deposit_fee: None,
        status,
        transaction_id: row.record_id.trim().to_owned(),
        confirmations: None,
        checked_at_ms,
        source_url: source_url.to_owned(),
        problem,
    })
}

fn validate_status_request(request: &DepositStatusRequest) -> ExchangeResult<()> {
    if !request.venue.trim().eq_ignore_ascii_case("bitget") {
        return Err(ExchangeError::UnsupportedSymbol(format!(
            "bitget deposit venue={}",
            request.venue
        )));
    }
    required_text("currency", &request.currency)?;
    required_text("network", &request.network)?;
    required_text("address", &request.address)?;
    required_text("transactionId", &request.transaction_id)?;
    if request
        .tag
        .as_deref()
        .is_some_and(|tag| !tag.trim().is_empty())
    {
        return Err(ExchangeError::UnsupportedCapability(
            "bitget deposit history tag verification",
        ));
    }
    if request.amount <= Decimal::ZERO || request.submitted_at_ms <= 0 {
        return Err(ExchangeError::Parse(
            "bitget deposit amount and submittedAtMs must be positive".to_owned(),
        ));
    }
    Ok(())
}

fn validate_history_identity(
    row: &DepositHistoryRow,
    request: &DepositStatusRequest,
) -> ExchangeResult<()> {
    let created_at_ms =
        row.created_time.trim().parse::<i64>().map_err(|error| {
            ExchangeError::Parse(format!("bitget deposit createdTime: {error}"))
        })?;
    let matches = row.operation_type.eq_ignore_ascii_case("deposit")
        && row.dest.eq_ignore_ascii_case("on_chain")
        && row.coin.eq_ignore_ascii_case(request.currency.trim())
        && canonical_network_id(&row.chain) == canonical_network_id(&request.network)
        && addresses_match(&row.to_address, &request.address, &row.chain)
        && created_at_ms
            >= request
                .submitted_at_ms
                .saturating_sub(HISTORY_CLOCK_SKEW_MS);
    if matches {
        Ok(())
    } else {
        Err(ExchangeError::Parse(
            "bitget deposit history identity does not match the authorized chain transfer"
                .to_owned(),
        ))
    }
}

fn history_cursor(rows: &[DepositHistoryRow]) -> Option<String> {
    rows.iter()
        .filter_map(|row| row.order_id.trim().parse::<u128>().ok())
        .min()
        .map(|value| value.to_string())
}

fn transaction_ids_match(actual: &str, expected: &str, network: &str) -> bool {
    if canonical_network_id(network) == "solana" {
        actual.trim() == expected.trim()
    } else {
        actual.trim().eq_ignore_ascii_case(expected.trim())
    }
}

fn addresses_match(actual: &str, expected: &str, network: &str) -> bool {
    if canonical_network_id(network) == "solana" {
        actual.trim() == expected.trim()
    } else {
        actual.trim().eq_ignore_ascii_case(expected.trim())
    }
}

fn tags_match(actual: &str, expected: Option<&str>) -> bool {
    match expected.map(str::trim).filter(|value| !value.is_empty()) {
        Some(expected) => actual.trim() == expected,
        None => actual.trim().is_empty(),
    }
}

fn is_evm_address(value: &str) -> bool {
    value.len() == 42
        && value.starts_with("0x")
        && value[2..].bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn form_query(params: &[(&str, &str)]) -> String {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    serializer.extend_pairs(params.iter().copied());
    serializer.finish()
}

fn parse_decimal(field: &str, value: &str) -> ExchangeResult<Decimal> {
    let amount: Decimal = value
        .trim()
        .parse()
        .map_err(|error| ExchangeError::Parse(format!("bitget deposit {field}: {error}")))?;
    if amount < Decimal::ZERO {
        return Err(ExchangeError::Parse(format!(
            "bitget deposit {field} is negative"
        )));
    }
    Ok(amount)
}

fn required_text(field: &str, value: &str) -> ExchangeResult<()> {
    if value.trim().is_empty() {
        Err(ExchangeError::Parse(format!(
            "bitget deposit {field} is empty"
        )))
    } else {
        Ok(())
    }
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
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn deposit_cursor_cannot_hide_duplicate_or_unfinished_history() {
        for (duplicate, stalled) in [(false, false), (true, false), (false, true)] {
            let server = MockServer::start().await;
            let matching = serde_json::json!({"orderId":"100","recordId":"SolanaSignature","coin":"USDC","type":"deposit","dest":"on_chain","size":"12.4","status":"success","toAddress":"BitgetSolanaAddress","chain":"SOL","createdTime":"1700000001000"});
            let mut first = vec![matching.clone()];
            for id in 1..100 {
                let mut other = matching.clone();
                other["orderId"] = id.to_string().into();
                other["recordId"] = format!("OtherSignature{id}").into();
                first.push(other);
            }
            Mock::given(method("GET"))
                .and(path(DEPOSIT_HISTORY_PATH))
                .respond_with(move |request: &wiremock::Request| {
                    let second = request
                        .url
                        .query_pairs()
                        .any(|(key, value)| key == "cursor" && value == "1");
                    let rows = if !second || stalled {
                        first.clone()
                    } else if duplicate {
                        vec![matching.clone()]
                    } else {
                        vec![]
                    };
                    ResponseTemplate::new(200).set_body_json(
                        serde_json::json!({"code":"00000","msg":"success","data":rows}),
                    )
                })
                .expect(2)
                .mount(&server)
                .await;
            let http = HttpClient::builder("bitget")
                .max_retries(1)
                .build()
                .unwrap();
            let result = status(&http, &server.uri(), &status_request(), headers).await;
            if duplicate || stalled {
                assert!(result.unwrap_err().to_string().contains(if duplicate {
                    "ambiguous"
                } else {
                    "incomplete"
                }));
            } else {
                assert_eq!(result.unwrap().unwrap().amount, Decimal::new(124, 1));
            }
        }
    }
    fn headers(_path: &str) -> ExchangeResult<SignedHeaders> {
        Ok([
            ("ACCESS-KEY".to_owned(), "key".to_owned()),
            ("ACCESS-SIGN".to_owned(), "sign".to_owned()),
            ("ACCESS-PASSPHRASE".to_owned(), "pass".to_owned()),
            ("ACCESS-TIMESTAMP".to_owned(), "1700000000000".to_owned()),
            ("locale".to_owned(), "en-US".to_owned()),
        ])
    }

    fn destination_request() -> TransferDestinationRequest {
        TransferDestinationRequest {
            currency: "USDC".to_owned(),
            network: "SOL".to_owned(),
            direction: TransferDirection::DepositToVenue,
            expected_address: None,
            expected_tag: None,
            amount: None,
        }
    }

    fn status_request() -> DepositStatusRequest {
        DepositStatusRequest {
            venue: "bitget".to_owned(),
            currency: "USDC".to_owned(),
            network: "SOL".to_owned(),
            address: "BitgetSolanaAddress".to_owned(),
            tag: None,
            transaction_id: "SolanaSignature".to_owned(),
            amount: Decimal::new(125, 1),
            submitted_at_ms: 1_700_000_000_000,
        }
    }

    fn withdrawal_destination_request() -> TransferDestinationRequest {
        TransferDestinationRequest {
            currency: "USDC".to_owned(),
            network: "SOL".to_owned(),
            direction: TransferDirection::WithdrawToChain,
            expected_address: Some("SolanaAddress".to_owned()),
            expected_tag: None,
            amount: None,
        }
    }

    #[test]
    fn parses_exact_official_deposit_address() {
        let row = parse_destination(
            r#"{"code":"00000","msg":"success","data":{"address":"BitgetSolanaAddress","chain":"SOL","coin":"USDC","tag":null,"url":""}}"#,
            &destination_request(),
            "official",
            1,
        )
        .expect("deposit address");

        assert_eq!(row.status, TransferDestinationStatus::Verified);
        assert_eq!(row.address.as_deref(), Some("BitgetSolanaAddress"));
    }

    #[test]
    fn exact_transaction_identity_maps_terminal_failure() {
        let rows = parse_history_page(
            r#"{"code":"00000","msg":"success","data":[{"orderId":"9","recordId":"SolanaSignature","coin":"usdc","type":"deposit","dest":"on_chain","size":"12.5","status":"fail","fromAddress":"source","toAddress":"BitgetSolanaAddress","chain":"SOL","createdTime":"1700000001000","updatedTime":"1700000002000"}]}"#,
        )
        .expect("deposit page");
        let row =
            parse_status(&rows[0], &status_request(), "official", 2).expect("exact deposit row");

        assert_eq!(row.status, DepositStatus::Failed);
    }

    #[tokio::test]
    async fn withdrawal_destination_requires_an_exact_official_address_book_row() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(WITHDRAW_ADDRESS_PATH))
            .and(query_param("coin", "USDC"))
            .and(query_param("type", "regular"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                r#"{"code":"00000","msg":"success","data":{"addressList":[{"coin":"USDC","chain":"SOL","address":"SolanaAddress","memo":"","type":"regular","createdTime":"1700000000000"}],"cursor":""}}"#,
                "application/json",
            ))
            .mount(&server)
            .await;
        let http = HttpClient::builder("bitget")
            .max_retries(1)
            .build()
            .expect("http client");

        let result = destination(
            &http,
            &server.uri(),
            &withdrawal_destination_request(),
            headers,
        )
        .await
        .expect("withdrawal destination");

        assert_eq!(result.status, TransferDestinationStatus::Verified);
        assert_eq!(result.allowlisted, Some(true));
    }
}
