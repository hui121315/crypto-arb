//! Bybit V5 deposit address and finality evidence.
//!
//! Official contracts:
//! - <https://bybit-exchange.github.io/docs/v5/asset/deposit/master-deposit-addr>
//! - <https://bybit-exchange.github.io/docs/v5/asset/deposit/deposit-record>
//! - <https://bybit-exchange.github.io/docs/v5/enum#depositstatus>

use super::bybit_private_rest::SignedHeaders;
use super::bybit_response::BybitObjectResponse;
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

const DEPOSIT_ADDRESS_PATH: &str = "/v5/asset/deposit/query-address";
const DEPOSIT_HISTORY_PATH: &str = "/v5/asset/deposit/query-record";
const HISTORY_CLOCK_SKEW_MS: i64 = 5 * 60_000;
const HISTORY_WINDOW_MS: i64 = 30 * 24 * 60 * 60 * 1_000;
const MAX_HISTORY_PAGES: usize = 5;

#[derive(Debug, Deserialize)]
struct DepositAddressPage {
    coin: String,
    #[serde(default)]
    chains: Vec<DepositAddressRow>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DepositAddressRow {
    chain_type: String,
    address_deposit: String,
    #[serde(default)]
    tag_deposit: String,
    chain: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DepositHistoryPage {
    #[serde(default)]
    rows: Vec<DepositHistoryRow>,
    #[serde(default)]
    next_page_cursor: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DepositHistoryRow {
    coin: String,
    chain: String,
    amount: String,
    #[serde(default)]
    deposit_fee: String,
    #[serde(rename = "txID")]
    tx_id: String,
    status: i64,
    to_address: String,
    #[serde(default)]
    tag: String,
    #[serde(default)]
    confirmations: String,
    #[serde(default)]
    deposit_type: String,
    #[serde(default)]
    travel_rule_status: Option<u8>,
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
    if request.direction != TransferDirection::DepositToVenue {
        return Err(ExchangeError::UnsupportedCapability(
            "bybit withdrawal destination allowlist",
        ));
    }
    required_text("currency", &request.currency)?;
    required_text("network", &request.network)?;
    let query = form_query(&[
        ("coin", request.currency.as_str()),
        ("chainType", request.network.as_str()),
    ]);
    let source_url = format!("{base_url}{DEPOSIT_ADDRESS_PATH}");
    let body = signed_get(http, &source_url, &query, &mut signed_headers).await?;
    parse_destination(&body, request, &source_url, common::time::now_ms())
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
    let start_time_ms = request
        .submitted_at_ms
        .saturating_sub(HISTORY_CLOCK_SKEW_MS)
        .max(0);
    let end_time_ms = common::time::now_ms()
        .max(request.submitted_at_ms)
        .min(start_time_ms.saturating_add(HISTORY_WINDOW_MS - 1));
    let start_time = start_time_ms.to_string();
    let end_time = end_time_ms.to_string();
    let source_url = format!("{base_url}{DEPOSIT_HISTORY_PATH}");
    let coin = request.currency.trim().to_ascii_uppercase();
    let mut cursor = String::new();
    let mut rows = Vec::new();
    for _ in 0..MAX_HISTORY_PAGES {
        let mut params = vec![
            ("txID", request.transaction_id.as_str()),
            ("coin", coin.as_str()),
            ("startTime", start_time.as_str()),
            ("endTime", end_time.as_str()),
            ("limit", "50"),
        ];
        if !cursor.is_empty() {
            params.push(("cursor", cursor.as_str()));
        }
        let query = form_query(&params);
        let body = signed_get(http, &source_url, &query, &mut signed_headers).await?;
        let page = parse_history_page(&body)?;
        if page.rows.len() > 50 {
            return Err(ExchangeError::Parse(
                "bybit deposit page exceeds requested limit".to_owned(),
            ));
        }
        rows.extend(page.rows);
        let next = page.next_page_cursor.trim();
        if next.is_empty() {
            return matching_status(&rows, request, &source_url, common::time::now_ms());
        }
        if next == cursor {
            break;
        }
        cursor = next.to_owned();
    }
    Err(ExchangeError::Parse(
        "bybit deposit history pagination is incomplete; cannot prove a unique deposit".to_owned(),
    ))
}

async fn signed_get<F>(
    http: &HttpClient,
    source_url: &str,
    query: &str,
    signed_headers: &mut F,
) -> ExchangeResult<String>
where
    F: FnMut(&str) -> ExchangeResult<SignedHeaders>,
{
    let url = format!("{source_url}?{query}");
    let response = http
        .execute_with_retry_fresh(Method::GET, source_url, || {
            let headers = signed_headers(query)?;
            let mut request = http.request(Method::GET, &url);
            for (key, value) in headers {
                request = request.header(key, value);
            }
            Ok(request)
        })
        .await?;
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

fn parse_destination(
    body: &str,
    request: &TransferDestinationRequest,
    source_url: &str,
    checked_at_ms: i64,
) -> ExchangeResult<TransferDestinationEvidence> {
    let response: BybitObjectResponse<DepositAddressPage> = serde_json::from_str(body)
        .map_err(|error| ExchangeError::Parse(format!("bybit deposit address: {error}")))?;
    let page = response.into_result("deposit address")?;
    if !page.coin.eq_ignore_ascii_case(request.currency.trim()) {
        return Err(ExchangeError::Parse(
            "bybit deposit address coin does not match requested currency".to_owned(),
        ));
    }
    let row = page
        .chains
        .into_iter()
        .find(|row| row.chain.eq_ignore_ascii_case(request.network.trim()))
        .ok_or_else(|| {
            ExchangeError::Parse(
                "bybit deposit address response has no exact requested chain".to_owned(),
            )
        })?;
    let address = non_empty(&row.address_deposit);
    let status = if address.is_some() {
        TransferDestinationStatus::Verified
    } else {
        TransferDestinationStatus::Missing
    };
    Ok(TransferDestinationEvidence {
        venue: "bybit".to_owned(),
        currency: page.coin.trim().to_ascii_uppercase(),
        network: row.chain.trim().to_owned(),
        direction: request.direction,
        address,
        tag: non_empty(&row.tag_deposit),
        status,
        allowlisted: None,
        checked_at_ms,
        source_url: source_url.to_owned(),
        problem: (status == TransferDestinationStatus::Missing)
            .then(|| format!("Bybit 官方接口未返回 {} 充值地址", row.chain_type.trim())),
    })
}

fn parse_history_page(body: &str) -> ExchangeResult<DepositHistoryPage> {
    let response: BybitObjectResponse<DepositHistoryPage> = serde_json::from_str(body)
        .map_err(|error| ExchangeError::Parse(format!("bybit deposit records: {error}")))?;
    response.into_result("deposit records")
}

#[cfg(test)]
fn parse_status(
    body: &str,
    request: &DepositStatusRequest,
    source_url: &str,
    checked_at_ms: i64,
) -> ExchangeResult<Option<DepositStatusEvidence>> {
    matching_status(
        &parse_history_page(body)?.rows,
        request,
        source_url,
        checked_at_ms,
    )
}

fn matching_status(
    rows: &[DepositHistoryRow],
    request: &DepositStatusRequest,
    source_url: &str,
    checked_at_ms: i64,
) -> ExchangeResult<Option<DepositStatusEvidence>> {
    let candidates = rows
        .iter()
        .filter(|row| transaction_ids_match(&row.tx_id, &request.transaction_id, &request.network));
    let mut matching = candidates
        .clone()
        .filter(|row| validate_history_identity(row, request).is_ok());
    let Some(row) = matching.next() else {
        if candidates.count() > 0 {
            return Err(ExchangeError::Parse(
                "bybit deposit history identity does not match the authorized chain transfer"
                    .to_owned(),
            ));
        }
        return Ok(None);
    };
    if matching.next().is_some() {
        return Err(ExchangeError::Parse(
            "bybit deposit history has ambiguous matching records".to_owned(),
        ));
    }
    let (status, problem) = map_status(row)?;
    Ok(Some(DepositStatusEvidence {
        venue: "bybit".to_owned(),
        currency: row.coin.trim().to_ascii_uppercase(),
        network: row.chain.trim().to_owned(),
        address: row.to_address.trim().to_owned(),
        tag: non_empty(&row.tag),
        amount: parse_decimal("amount", &row.amount)?,
        deposit_fee: non_empty(&row.deposit_fee)
            .map(|fee| parse_decimal("depositFee", &fee))
            .transpose()?,
        status,
        transaction_id: row.tx_id.trim().to_owned(),
        confirmations: row.confirmations.trim().parse().ok(),
        checked_at_ms,
        source_url: source_url.to_owned(),
        problem,
    }))
}

fn map_status(row: &DepositHistoryRow) -> ExchangeResult<(DepositStatus, Option<String>)> {
    match row.deposit_type.trim() {
        "10" => {
            return Ok((
                DepositStatus::Blocked,
                Some("Bybit 充值触及每日限额，需要在官网处理".to_owned()),
            ))
        }
        "20" => {
            return Ok((
                DepositStatus::Blocked,
                Some("Bybit 充值触发 AML 审核，需要在官网提交材料".to_owned()),
            ))
        }
        "50" => {
            return Ok((
                DepositStatus::Blocked,
                Some("Bybit 因账户或合规限制未入账，需要在官网取回资金".to_owned()),
            ))
        }
        "" | "0" => {}
        other => {
            return Err(ExchangeError::Parse(format!(
                "bybit depositType is unsupported: {other}"
            )))
        }
    }
    match row.travel_rule_status {
        Some(1) => {
            return Ok((
                DepositStatus::Blocked,
                Some("Bybit Travel Rule 需要补充交易对手信息".to_owned()),
            ))
        }
        Some(2) => {
            return Ok((
                DepositStatus::Pending,
                Some("Bybit Travel Rule 正在审核".to_owned()),
            ))
        }
        Some(3) => {
            return Ok((
                DepositStatus::Failed,
                Some("Bybit Travel Rule 审核拒绝或已取消".to_owned()),
            ))
        }
        None | Some(0) => {}
        Some(other) => {
            return Err(ExchangeError::Parse(format!(
                "bybit travelRuleStatus is unsupported: {other}"
            )))
        }
    }
    match row.status {
        0 => Ok((
            DepositStatus::Pending,
            Some("Bybit 充值状态未知，继续等待官方记录".to_owned()),
        )),
        1 => Ok((
            DepositStatus::Pending,
            Some("Bybit 充值等待区块确认".to_owned()),
        )),
        2 | 10011 => Ok((
            DepositStatus::Pending,
            Some("Bybit 充值正在入账处理".to_owned()),
        )),
        3 | 10012 | 70012 => Ok((DepositStatus::Completed, None)),
        4 | 70011 => Ok((
            DepositStatus::Failed,
            Some("Bybit 官方充值记录报告失败或已回滚".to_owned()),
        )),
        7 => Ok((
            DepositStatus::Blocked,
            Some("Bybit 检测到链回滚，已暂停自动流程等待最终结果".to_owned()),
        )),
        70013 => Ok((
            DepositStatus::Blocked,
            Some("Bybit 链回滚自动处理失败，需要人工审核".to_owned()),
        )),
        other => Err(ExchangeError::Parse(format!(
            "bybit deposit status is unsupported: {other}"
        ))),
    }
}

fn validate_status_request(request: &DepositStatusRequest) -> ExchangeResult<()> {
    if !request.venue.trim().eq_ignore_ascii_case("bybit") {
        return Err(ExchangeError::UnsupportedSymbol(format!(
            "bybit deposit venue={}",
            request.venue
        )));
    }
    required_text("currency", &request.currency)?;
    required_text("network", &request.network)?;
    required_text("address", &request.address)?;
    required_text("transactionId", &request.transaction_id)?;
    if request.amount <= Decimal::ZERO || request.submitted_at_ms <= 0 {
        return Err(ExchangeError::Parse(
            "bybit deposit amount and submittedAtMs must be positive".to_owned(),
        ));
    }
    Ok(())
}

fn validate_history_identity(
    row: &DepositHistoryRow,
    request: &DepositStatusRequest,
) -> ExchangeResult<()> {
    let matches = row.coin.eq_ignore_ascii_case(request.currency.trim())
        && canonical_network_id(&row.chain) == canonical_network_id(&request.network)
        && addresses_match(&row.to_address, &request.address, &row.chain)
        && tags_match(&row.tag, request.tag.as_deref());
    if matches {
        Ok(())
    } else {
        Err(ExchangeError::Parse(
            "bybit deposit history identity does not match the authorized chain transfer"
                .to_owned(),
        ))
    }
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
    let actual = actual.trim();
    match expected.map(str::trim).filter(|value| !value.is_empty()) {
        Some(expected) => actual == expected,
        None => actual.is_empty(),
    }
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
        .map_err(|error| ExchangeError::Parse(format!("bybit deposit {field}: {error}")))?;
    if amount < Decimal::ZERO {
        return Err(ExchangeError::Parse(format!(
            "bybit deposit {field} is negative"
        )));
    }
    Ok(amount)
}

fn required_text(field: &str, value: &str) -> ExchangeResult<()> {
    if value.trim().is_empty() {
        Err(ExchangeError::Parse(format!(
            "bybit deposit {field} is empty"
        )))
    } else {
        Ok(())
    }
}

fn non_empty(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::{
        matchers::{method, path},
        Mock, MockServer, ResponseTemplate,
    };

    #[tokio::test]
    async fn deposit_cursor_must_finish_before_unique_credit_is_proven() {
        for (duplicate, stalled) in [(false, false), (true, false), (false, true)] {
            let server = MockServer::start().await;
            let row = json!({"coin":"USDC","chain":"SOL","amount":"12.4","txID":"SolanaSignature","status":3,"toAddress":"BybitSolanaAddress","tag":"","depositFee":"0"});
            Mock::given(method("GET")).and(path(DEPOSIT_HISTORY_PATH))
                .respond_with(move |request: &wiremock::Request| {
                    let second = request.url.query_pairs().any(|(key, value)| key == "cursor" && value == "page-2");
                    assert!(request.url.query_pairs().any(|(key, value)| key == "txID" && value == "SolanaSignature"));
                    let rows = if !second || duplicate { vec![row.clone()] } else { vec![] };
                    ResponseTemplate::new(200).set_body_json(json!({"retCode":0,"result":{"rows":rows,"nextPageCursor":if !second || stalled { "page-2" } else { "" }}}))
                }).expect(2).mount(&server).await;
            let http = HttpClient::builder("bybit").max_retries(1).build().unwrap();
            let result = status(&http, &server.uri(), &status_request(), |_| {
                Ok([
                    ("X-BAPI-API-KEY".to_owned(), "fixture".to_owned()),
                    ("X-BAPI-SIGN".to_owned(), "fixture".to_owned()),
                    ("X-BAPI-TIMESTAMP".to_owned(), "1700000000000".to_owned()),
                    ("X-BAPI-RECV-WINDOW".to_owned(), "5000".to_owned()),
                ])
            })
            .await;
            if duplicate || stalled {
                let message = result.unwrap_err().to_string();
                assert!(message.contains(if duplicate { "ambiguous" } else { "incomplete" }));
            } else {
                assert_eq!(result.unwrap().unwrap().amount, Decimal::new(124, 1));
            }
        }
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
            venue: "bybit".to_owned(),
            currency: "USDC".to_owned(),
            network: "SOL".to_owned(),
            address: "BybitSolanaAddress".to_owned(),
            tag: None,
            transaction_id: "SolanaSignature".to_owned(),
            amount: Decimal::new(125, 1),
            submitted_at_ms: 1_700_000_000_000,
        }
    }

    #[test]
    fn parses_exact_official_deposit_address() {
        let row = parse_destination(
            r#"{"retCode":0,"retMsg":"success","result":{"coin":"USDC","chains":[{"chainType":"Solana","addressDeposit":"BybitSolanaAddress","tagDeposit":"","chain":"SOL","batchReleaseLimit":"-1","contractAddress":""}]}}"#,
            &destination_request(),
            "official",
            1,
        )
        .expect("deposit address");

        assert_eq!(row.status, TransferDestinationStatus::Verified);
        assert_eq!(row.network, "SOL");
    }

    #[test]
    fn travel_rule_collection_pauses_automation() {
        let row = parse_status(
            r#"{"retCode":0,"retMsg":"success","result":{"rows":[{"coin":"USDC","chain":"SOL","amount":"12.5","txID":"SolanaSignature","status":2,"toAddress":"BybitSolanaAddress","tag":"","confirmations":"4","depositType":"0","travelRuleStatus":1}]}}"#,
            &status_request(),
            "official",
            2,
        )
        .expect("deposit history")
        .expect("matching row");

        assert_eq!(row.status, DepositStatus::Blocked);
    }

    #[test]
    fn final_rollback_is_terminal_failure() {
        let row = parse_status(
            r#"{"retCode":0,"retMsg":"success","result":{"rows":[{"coin":"USDC","chain":"SOL","amount":"12.5","txID":"SolanaSignature","status":70011,"toAddress":"BybitSolanaAddress","tag":"","confirmations":"4","depositType":"0","travelRuleStatus":0}]}}"#,
            &status_request(),
            "official",
            2,
        )
        .expect("deposit history")
        .expect("matching row");

        assert_eq!(row.status, DepositStatus::Failed);
    }
}
