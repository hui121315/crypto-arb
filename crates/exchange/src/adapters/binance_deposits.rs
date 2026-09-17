//! Binance deposit finality evidence for chain-to-CEX replenishment.
//!
//! Official contract:
//! - <https://developers.binance.com/en/docs/catalog/core-trading-wallet/api/rest-api/capital#deposit-history>

use crate::error::{ExchangeError, ExchangeResult};
use crate::http::HttpClient;
use crate::{DepositStatus, DepositStatusEvidence, DepositStatusRequest};
use reqwest::Method;
use rust_decimal::Decimal;
use serde::Deserialize;

const DEPOSIT_HISTORY_PATH: &str = "/sapi/v1/capital/deposit/hisrec";
const HISTORY_CLOCK_SKEW_MS: i64 = 5 * 60_000;
const HISTORY_WINDOW_MS: i64 = 90 * 24 * 60 * 60 * 1_000;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DepositHistoryRow {
    amount: String,
    coin: String,
    network: String,
    status: u8,
    address: String,
    #[serde(default)]
    address_tag: String,
    tx_id: String,
    insert_time: i64,
    #[serde(default)]
    confirm_times: String,
}

pub(super) async fn status<F>(
    http: &HttpClient,
    base_url: &str,
    request: &DepositStatusRequest,
    mut signed_query: F,
) -> ExchangeResult<Option<DepositStatusEvidence>>
where
    F: FnMut(&[(&str, &str)]) -> ExchangeResult<(String, String)>,
{
    validate_request(request)?;
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
        ("txId", request.transaction_id.as_str()),
        ("startTime", start_time.as_str()),
        ("endTime", end_time.as_str()),
        ("limit", "1000"),
        ("recvWindow", "5000"),
    ];
    let url = format!("{base_url}{DEPOSIT_HISTORY_PATH}");
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
    request: &DepositStatusRequest,
    source_url: &str,
    checked_at_ms: i64,
) -> ExchangeResult<Option<DepositStatusEvidence>> {
    let rows: Vec<DepositHistoryRow> = serde_json::from_str(body)
        .map_err(|error| ExchangeError::Parse(format!("binance deposit history: {error}")))?;
    if rows.len() >= 1_000 {
        return Err(ExchangeError::Parse(
            "binance deposit history result is incomplete; cannot prove a unique deposit"
                .to_owned(),
        ));
    }
    let candidates = rows
        .iter()
        .filter(|row| transaction_ids_match(&row.tx_id, &request.transaction_id, &request.network));
    let mut matching = candidates
        .clone()
        .filter(|row| validate_history_identity(row, request).is_ok());
    let Some(row) = matching.next() else {
        if candidates.count() > 0 {
            return Err(ExchangeError::Parse(
                "binance deposit history identity does not match the authorized chain transfer"
                    .to_owned(),
            ));
        }
        return Ok(None);
    };
    if matching.next().is_some() {
        return Err(ExchangeError::Parse(
            "binance deposit history has ambiguous matching records".to_owned(),
        ));
    }
    let status = map_status(row.status)?;
    // unlockConfirm is a required threshold, not an observed confirmation count.
    let confirmations = confirmations(&row.confirm_times);
    Ok(Some(DepositStatusEvidence {
        venue: "binance".to_owned(),
        currency: row.coin.trim().to_ascii_uppercase(),
        network: row.network.trim().to_owned(),
        address: row.address.trim().to_owned(),
        tag: non_empty(&row.address_tag),
        amount: parse_decimal("amount", &row.amount)?,
        deposit_fee: None,
        status,
        transaction_id: row.tx_id.trim().to_owned(),
        confirmations,
        checked_at_ms,
        source_url: source_url.to_owned(),
        problem: match row.status {
            2 => Some("Binance 已拒绝该笔充值，请在官网核对原因".to_owned()),
            6 => Some("Binance 已入账并可交易，但尚未解锁后续提币".to_owned()),
            7 => Some("Binance 标记为错误充值，需要在官网处理".to_owned()),
            8 => Some("Binance 充值等待用户确认，请在官网处理".to_owned()),
            _ => None,
        },
    }))
}

fn validate_request(request: &DepositStatusRequest) -> ExchangeResult<()> {
    validate_venue(&request.venue)?;
    required_text("currency", &request.currency)?;
    required_text("network", &request.network)?;
    required_text("address", &request.address)?;
    required_text("transactionId", &request.transaction_id)?;
    if request.amount <= Decimal::ZERO || request.submitted_at_ms <= 0 {
        return Err(ExchangeError::Parse(
            "binance deposit amount and submittedAtMs must be positive".to_owned(),
        ));
    }
    Ok(())
}

fn validate_history_identity(
    row: &DepositHistoryRow,
    request: &DepositStatusRequest,
) -> ExchangeResult<()> {
    let timestamp_matches = row.insert_time
        >= request
            .submitted_at_ms
            .saturating_sub(HISTORY_CLOCK_SKEW_MS);
    let matches = row.coin.eq_ignore_ascii_case(request.currency.trim())
        && row.network.eq_ignore_ascii_case(request.network.trim())
        && addresses_match(&row.address, &request.address, &row.network)
        && tags_match(&row.address_tag, request.tag.as_deref())
        && timestamp_matches;
    if matches {
        Ok(())
    } else {
        Err(ExchangeError::Parse(
            "binance deposit history identity does not match the authorized chain transfer"
                .to_owned(),
        ))
    }
}

fn transaction_ids_match(actual: &str, expected: &str, network: &str) -> bool {
    if crate::canonical_network_id(network) == "solana" {
        actual.trim() == expected.trim()
    } else {
        actual.trim().eq_ignore_ascii_case(expected.trim())
    }
}

fn addresses_match(actual: &str, expected: &str, network: &str) -> bool {
    if crate::canonical_network_id(network) == "solana" {
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

fn map_status(status: u8) -> ExchangeResult<DepositStatus> {
    match status {
        0 => Ok(DepositStatus::Pending),
        6 => Ok(DepositStatus::CreditedLocked),
        1 => Ok(DepositStatus::Completed),
        2 => Ok(DepositStatus::Failed),
        7 | 8 => Ok(DepositStatus::Blocked),
        other => Err(ExchangeError::Parse(format!(
            "binance deposit status is unsupported: {other}"
        ))),
    }
}

fn confirmations(value: &str) -> Option<u64> {
    value
        .trim()
        .split('/')
        .next()
        .and_then(|value| value.trim().parse().ok())
}

fn validate_venue(venue: &str) -> ExchangeResult<()> {
    if venue.trim().eq_ignore_ascii_case("binance") {
        Ok(())
    } else {
        Err(ExchangeError::UnsupportedSymbol(format!(
            "binance deposit venue={venue}"
        )))
    }
}

fn parse_decimal(field: &str, value: &str) -> ExchangeResult<Decimal> {
    let amount: Decimal = value
        .trim()
        .parse()
        .map_err(|error| ExchangeError::Parse(format!("binance deposit {field}: {error}")))?;
    if amount < Decimal::ZERO {
        return Err(ExchangeError::Parse(format!(
            "binance deposit {field} is negative"
        )));
    }
    Ok(amount)
}

fn required_text(field: &str, value: &str) -> ExchangeResult<String> {
    non_empty(value)
        .ok_or_else(|| ExchangeError::Parse(format!("binance deposit {field} is empty")))
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

    // Synthetic transfer identity, official response field types.
    const FIXTURE: &str = include_str!("../../fixtures/binance/deposit_history_usdc_solana.json");

    fn request() -> DepositStatusRequest {
        DepositStatusRequest {
            venue: "binance".to_owned(),
            currency: "USDC".to_owned(),
            network: "SOL".to_owned(),
            address: "SolanaDepositAddress".to_owned(),
            tag: None,
            transaction_id: "SolanaTxSignature".to_owned(),
            amount: Decimal::new(125, 1),
            submitted_at_ms: 1_700_000_000_000,
        }
    }

    #[test]
    fn exact_transaction_identity_maps_credited_locked() {
        let row = parse_status(FIXTURE, &request(), "official", 1_700_000_002_000)
            .expect("deposit history")
            .expect("matching deposit");

        assert_eq!(row.status, DepositStatus::CreditedLocked);
        assert_eq!(row.confirmations, Some(12));
        assert_eq!(row.amount, Decimal::new(125, 1));
    }

    #[test]
    fn actual_amount_reaches_reconciliation_instead_of_failing_identity() {
        let evidence = parse_status(
            r#"[{"amount":"12.4","coin":"USDC","network":"SOL","status":1,"address":"SolanaDepositAddress","addressTag":"","txId":"SolanaTxSignature","insertTime":1700000001000,"unlockConfirm":12,"confirmTimes":"12/12"}]"#,
            &request(),
            "official",
            1_700_000_002_000,
        )
        .unwrap().unwrap();
        assert_eq!(evidence.amount, Decimal::new(124, 1));
        assert_eq!(evidence.status, DepositStatus::Completed);
    }

    #[test]
    fn saturated_history_cannot_prove_a_unique_credit() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(FIXTURE).unwrap();
        let body = serde_json::to_string(&vec![rows[0].clone(); 1_000]).unwrap();
        assert!(
            parse_status(&body, &request(), "official", 1_700_000_002_000)
                .unwrap_err()
                .to_string()
                .contains("incomplete")
        );
    }

    #[test]
    fn official_rejection_and_manual_action_statuses_are_not_parse_errors() {
        let mut rows: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
        for (status, expected) in [
            (1, DepositStatus::Completed),
            (2, DepositStatus::Failed),
            (7, DepositStatus::Blocked),
            (8, DepositStatus::Blocked),
        ] {
            rows[0]["status"] = status.into();
            let evidence = parse_status(&rows.to_string(), &request(), "official", 1700000002000)
                .unwrap()
                .unwrap();
            assert_eq!(evidence.status, expected);
            assert_eq!(evidence.problem.is_some(), status != 1);
        }
    }

    #[test]
    fn required_unlock_threshold_is_never_reported_as_observed_confirmations() {
        let mut rows: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
        rows[0].as_object_mut().unwrap().remove("confirmTimes");
        let evidence = parse_status(&rows.to_string(), &request(), "official", 1700000002000)
            .unwrap()
            .unwrap();
        assert_eq!(evidence.confirmations, None);
        assert_eq!(evidence.status, DepositStatus::CreditedLocked);
    }

    #[test]
    fn matching_uses_full_identity_and_rejects_ambiguous_duplicates() {
        let mut rows: Vec<serde_json::Value> = serde_json::from_str(FIXTURE).unwrap();
        let matching = rows[0].clone();
        rows[0]["coin"] = "OTHER".into();
        rows.push(matching.clone());
        let evidence = parse_status(
            &serde_json::to_string(&rows).unwrap(),
            &request(),
            "official",
            1700000002000,
        )
        .unwrap()
        .unwrap();
        assert_eq!(evidence.currency, "USDC");
        rows.push(matching);
        assert!(parse_status(
            &serde_json::to_string(&rows).unwrap(),
            &request(),
            "official",
            1700000002000
        )
        .unwrap_err()
        .to_string()
        .contains("ambiguous"));
    }

    #[tokio::test]
    async fn history_queries_exact_tx_id_and_reads_numeric_unlock_confirm() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(DEPOSIT_HISTORY_PATH))
            .and(header("X-MBX-APIKEY", "test-key"))
            .and(query_param("txId", "SolanaTxSignature"))
            .and(query_param("coin", "USDC"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(FIXTURE, "application/json"))
            .expect(1)
            .mount(&server)
            .await;
        let http = HttpClient::builder("binance")
            .max_retries(1)
            .build()
            .unwrap();
        let evidence = status(&http, &server.uri(), &request(), |params| {
            let mut query = url::form_urlencoded::Serializer::new(String::new());
            query.extend_pairs(params.iter().copied());
            Ok((query.finish(), "test-key".to_owned()))
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(evidence.status, DepositStatus::CreditedLocked);
        assert_eq!(evidence.amount, request().amount);
    }
}
