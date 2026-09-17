//! Bybit V5 on-chain withdrawals. No write retries or fuzzy history matching.
//!
//! Official contracts:
//! - <https://bybit-exchange.github.io/docs/v5/asset/withdraw>
//! - <https://bybit-exchange.github.io/docs/v5/asset/withdraw/withdraw-record>
//! - <https://bybit-exchange.github.io/docs/v5/asset/withdraw/withdraw-address>
//! - <https://bybit-exchange.github.io/docs/v5/asset/balance/delay-amount>
//! - <https://bybit-exchange.github.io/docs/v5/enum#withdrawstatus>

use super::bybit_private_rest::SignedHeaders;
use super::bybit_response::BybitObjectResponse;
use crate::error::{ExchangeError, ExchangeResult};
use crate::http::HttpClient;
use crate::{
    TransferDestinationEvidence, TransferDestinationRequest, TransferDestinationStatus,
    TransferDirection, WithdrawalSourceBalance, WithdrawalSourceBalanceRequest, WithdrawalStatus,
    WithdrawalStatusEvidence, WithdrawalStatusRequest, WithdrawalSubmission,
    WithdrawalSubmitRequest, WithdrawalWalletType,
};
use reqwest::Method;
use rust_decimal::Decimal;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::collections::HashMap;

const WITHDRAW_PATH: &str = "/v5/asset/withdraw/create";
const HISTORY_PATH: &str = "/v5/asset/withdraw/query-record";
const ADDRESS_PATH: &str = "/v5/asset/withdraw/query-address";
const BALANCE_PATH: &str = "/v5/asset/withdraw/withdrawable-amount";
const HISTORY_CLOCK_SKEW_MS: i64 = 5 * 60_000;
const HISTORY_WINDOW_MS: i64 = 30 * 24 * 60 * 60 * 1_000;
const MAX_PAGES: usize = 5;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WithdrawBody<'a> {
    coin: String,
    chain: &'a str,
    address: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    tag: Option<&'a str>,
    amount: String,
    timestamp: i64,
    force_chain: u8,
    account_type: &'static str,
    fee_type: u8,
    request_id: String,
}

#[derive(Deserialize)]
struct WithdrawAck {
    id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Page<T> {
    rows: Vec<T>,
    next_page_cursor: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AddressRow {
    coin: String,
    chain: String,
    address: String,
    tag: String,
    status: u8,
    address_type: u8,
    verified: u8,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BalancePage {
    withdrawable_amount: HashMap<String, BalanceRow>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BalanceRow {
    coin: String,
    withdrawable_amount: String,
    available_balance: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct HistoryRow {
    withdraw_id: String,
    #[serde(rename = "txID")]
    tx_id: String,
    coin: String,
    chain: String,
    amount: String,
    withdraw_fee: String,
    status: String,
    to_address: String,
    tag: String,
    create_time: String,
    withdraw_type: u8,
    #[serde(default)]
    tax: String,
}

pub(super) async fn submit<F>(
    http: &HttpClient,
    base: &str,
    request: &WithdrawalSubmitRequest,
    timestamp: i64,
    mut sign: F,
) -> ExchangeResult<WithdrawalSubmission>
where
    F: FnMut(&str) -> ExchangeResult<SignedHeaders>,
{
    validate_venue(&request.venue)?;
    required("coin", &request.currency)?;
    required("chain", &request.network)?;
    required("address", &request.address)?;
    required("client withdrawal ID", &request.client_withdrawal_id)?;
    if request.amount <= Decimal::ZERO || timestamp <= 0 {
        return Err(invalid("withdrawal amount and timestamp must be positive"));
    }
    let body = serde_json::to_string(&WithdrawBody {
        coin: request.currency.trim().to_ascii_uppercase(),
        chain: request.network.trim(),
        address: request.address.trim(),
        tag: non_empty(request.tag.as_deref()),
        amount: request.amount.normalize().to_string(),
        timestamp,
        force_chain: 1,
        account_type: wallet_name(request.wallet_type),
        fee_type: 0,
        request_id: wire_request_id(&request.client_withdrawal_id),
    })
    .map_err(|error| invalid(format!("withdrawal body: {error}")))?;
    let source_url = format!("{base}{WITHDRAW_PATH}");
    let response = http
        .execute_once_fresh(Method::POST, &source_url, || {
            let mut builder = http
                .request(Method::POST, &source_url)
                .header("Content-Type", "application/json")
                .body(body.clone());
            for (key, value) in sign(&body)? {
                builder = builder.header(key, value);
            }
            Ok(builder)
        })
        .await?;
    let ack: WithdrawAck = decode(&checked_body(response).await?)?;
    Ok(WithdrawalSubmission {
        venue: "bybit".into(),
        provider_withdrawal_id: required("withdrawal acknowledgement ID", &ack.id)?.into(),
        client_withdrawal_id: request.client_withdrawal_id.clone(),
        submitted_at_ms: common::time::now_ms(),
        source_url,
        problem: None,
    })
}

pub(super) async fn source_balance<F>(
    http: &HttpClient,
    base: &str,
    request: &WithdrawalSourceBalanceRequest,
    mut sign: F,
) -> ExchangeResult<WithdrawalSourceBalance>
where
    F: FnMut(&str) -> ExchangeResult<SignedHeaders>,
{
    validate_venue(&request.venue)?;
    let coin = required("coin", &request.currency)?.to_ascii_uppercase();
    let source_url = format!("{base}{BALANCE_PATH}");
    let page: BalancePage =
        decode(&get(http, &source_url, &query(&[("coin", &coin)]), &mut sign).await?)?;
    let wallet = wallet_name(request.wallet_type);
    let row = page.withdrawable_amount.get(wallet).ok_or_else(|| {
        invalid(format!(
            "withdrawable balance has no {wallet} wallet; other wallets are not substituted"
        ))
    })?;
    if row.coin != coin {
        return Err(invalid("withdrawable balance coin mismatch"));
    }
    Ok(WithdrawalSourceBalance {
        venue: "bybit".into(),
        currency: coin,
        wallet_type: request.wallet_type,
        available: decimal("withdrawableAmount", &row.withdrawable_amount)?
            .min(decimal("availableBalance", &row.available_balance)?),
        checked_at_ms: common::time::now_ms(),
        source_url,
    })
}

pub(super) async fn destination<F>(
    http: &HttpClient,
    base: &str,
    request: &TransferDestinationRequest,
    mut sign: F,
) -> ExchangeResult<TransferDestinationEvidence>
where
    F: FnMut(&str) -> ExchangeResult<SignedHeaders>,
{
    if request.direction != TransferDirection::WithdrawToChain {
        return Err(invalid("withdrawal address direction mismatch"));
    }
    let coin = required("coin", &request.currency)?.to_ascii_uppercase();
    let chain = required("chain", &request.network)?;
    let address = non_empty(request.expected_address.as_deref())
        .ok_or_else(|| invalid("expected withdrawal address missing"))?;
    let source_url = format!("{base}{ADDRESS_PATH}");
    // Chain-only query also includes Bybit universal (baseCoin) addresses.
    let rows: Vec<AddressRow> = pages(
        http,
        &source_url,
        &[("chain", chain), ("addressType", "0"), ("limit", "50")],
        &mut sign,
    )
    .await?;
    let matches = rows
        .iter()
        .filter(|row| {
            (row.coin == coin || row.coin == "baseCoin")
                && row.chain == chain
                && row.address == address
                && matches!(row.address_type, 0 | 2)
                && non_empty(Some(&row.tag)) == non_empty(request.expected_tag.as_deref())
        })
        .collect::<Vec<_>>();
    let verified = !matches.is_empty()
        && matches
            .iter()
            .all(|row| row.status == 0 && row.verified == 1);
    Ok(TransferDestinationEvidence {
        venue: "bybit".into(),
        currency: coin,
        network: chain.into(),
        direction: request.direction,
        address: Some(address.into()),
        tag: non_empty(request.expected_tag.as_deref()).map(str::to_owned),
        status: if verified {
            TransferDestinationStatus::Verified
        } else if matches.is_empty() {
            TransferDestinationStatus::Missing
        } else {
            TransferDestinationStatus::Unverified
        },
        allowlisted: Some(!matches.is_empty()),
        checked_at_ms: common::time::now_ms(),
        source_url,
        problem: (!verified).then(|| {
            if matches.is_empty() {
                "Bybit 地址簿未找到币种、网络、地址及 memo 完全匹配的链上地址".into()
            } else {
                "Bybit 提币地址尚未验证或仍在新地址 24 小时限制期，请在交易所处理".into()
            }
        }),
    })
}

pub(super) async fn status<F>(
    http: &HttpClient,
    base: &str,
    request: &WithdrawalStatusRequest,
    mut sign: F,
) -> ExchangeResult<Option<WithdrawalStatusEvidence>>
where
    F: FnMut(&str) -> ExchangeResult<SignedHeaders>,
{
    validate_venue(&request.venue)?;
    required("coin", &request.currency)?;
    required("chain", &request.network)?;
    required("address", &request.address)?;
    required("client withdrawal ID", &request.client_withdrawal_id)?;
    let provider_id = non_empty(request.provider_withdrawal_id.as_deref()).ok_or_else(|| invalid(format!(
        "提币应答编号缺失；Bybit 历史接口不能按 requestId 查询，不按金额猜测且不重发；请核对 requestId={}",
        wire_request_id(&request.client_withdrawal_id)
    )))?;
    if request.submitted_at_ms <= 0 {
        return Err(invalid("original submission time missing"));
    }
    let start = request
        .submitted_at_ms
        .saturating_sub(HISTORY_CLOCK_SKEW_MS)
        .max(0);
    let end = common::time::now_ms()
        .max(request.submitted_at_ms)
        .min(start.saturating_add(HISTORY_WINDOW_MS - 1));
    let start_text = start.to_string();
    let end_text = end.to_string();
    let coin = request.currency.trim().to_ascii_uppercase();
    let source_url = format!("{base}{HISTORY_PATH}");
    let rows: Vec<HistoryRow> = pages(
        http,
        &source_url,
        &[
            ("withdrawID", provider_id),
            ("coin", &coin),
            ("withdrawType", "0"),
            ("startTime", &start_text),
            ("endTime", &end_text),
            ("limit", "50"),
        ],
        &mut sign,
    )
    .await?;
    let mut matching = rows
        .into_iter()
        .filter(|row| row.withdraw_id == provider_id);
    let Some(row) = matching.next() else {
        return Ok(None);
    };
    if matching.next().is_some() {
        return Err(invalid("duplicate withdrawal ID in history"));
    }
    parse_status(row, request, start, end, source_url).map(Some)
}

fn parse_status(
    row: HistoryRow,
    request: &WithdrawalStatusRequest,
    start: i64,
    end: i64,
    source_url: String,
) -> ExchangeResult<WithdrawalStatusEvidence> {
    let created = row
        .create_time
        .parse::<i64>()
        .map_err(|_| invalid("invalid createTime"))?;
    if row.coin != request.currency.trim().to_ascii_uppercase()
        || row.chain != request.network.trim()
        || row.to_address != request.address.trim()
        || row.withdraw_type != 0
        || non_empty(Some(&row.tag)) != non_empty(request.tag.as_deref())
        || created < start
        || created > end
    {
        return Err(invalid(
            "withdrawal history identity, chain, memo or creation time mismatch",
        ));
    }
    // The API does not specify the denomination of tax; do not hide it in a coin fee.
    if non_empty(Some(&row.tax)).is_some() && decimal("tax", &row.tax)? != Decimal::ZERO {
        return Err(invalid(
            "提币回执包含额外税款，费用币种尚未核验，暂停自动结算",
        ));
    }
    let (status, problem) = match row.status.as_str() {
        "success" => (WithdrawalStatus::Completed, None),
        "SecurityCheck" | "Pending" | "BlockchainConfirmed" => (
            WithdrawalStatus::Pending,
            Some(format!("Bybit 提币处理中：{}", row.status)),
        ),
        "CancelByUser" => (WithdrawalStatus::Cancelled, Some("Bybit 提币已取消".into())),
        "Reject" => (WithdrawalStatus::Rejected, Some("Bybit 提币被拒绝".into())),
        "Fail" => (WithdrawalStatus::Failed, Some("Bybit 提币失败".into())),
        other => return Err(invalid(format!("Bybit 提币状态需人工核验：{other}"))),
    };
    let transaction_id = non_empty(Some(&row.tx_id)).map(str::to_owned);
    if status == WithdrawalStatus::Completed && transaction_id.is_none() {
        return Err(invalid(
            "successful on-chain withdrawal has no transaction hash",
        ));
    }
    Ok(WithdrawalStatusEvidence {
        venue: "bybit".into(),
        provider_withdrawal_id: row.withdraw_id,
        client_withdrawal_id: request.client_withdrawal_id.clone(),
        currency: row.coin,
        network: row.chain,
        address: row.to_address,
        amount: decimal("amount", &row.amount)?,
        transaction_fee: decimal("withdrawFee", &row.withdraw_fee)?,
        status,
        transaction_id,
        confirmations: None,
        checked_at_ms: common::time::now_ms(),
        source_url,
        problem,
    })
}

fn wallet_name(wallet: WithdrawalWalletType) -> &'static str {
    match wallet {
        WithdrawalWalletType::Spot => "UTA",
        WithdrawalWalletType::Funding => "FUND",
    }
}

fn wire_request_id(client_id: &str) -> String {
    common::signing::hmac_sha256_hex(b"crossline.bybit.withdrawal.v1", client_id.as_bytes())[..32]
        .into()
}

async fn pages<T: DeserializeOwned, F>(
    http: &HttpClient,
    url: &str,
    params: &[(&str, &str)],
    sign: &mut F,
) -> ExchangeResult<Vec<T>>
where
    F: FnMut(&str) -> ExchangeResult<SignedHeaders>,
{
    let mut rows = Vec::new();
    let mut cursor = String::new();
    for _ in 0..MAX_PAGES {
        let mut params = params.to_vec();
        if !cursor.is_empty() {
            params.push(("cursor", &cursor));
        }
        let page: Page<T> = decode(&get(http, url, &query(&params), sign).await?)?;
        if page.rows.len() > 50 {
            return Err(invalid("page exceeds requested limit"));
        }
        rows.extend(page.rows);
        if page.next_page_cursor.is_empty() {
            return Ok(rows);
        }
        if page.next_page_cursor == cursor {
            break;
        }
        cursor = page.next_page_cursor;
    }
    Err(invalid("分页未完整读取，无法证明提币记录或地址唯一"))
}

async fn get<F>(http: &HttpClient, url: &str, query: &str, sign: &mut F) -> ExchangeResult<String>
where
    F: FnMut(&str) -> ExchangeResult<SignedHeaders>,
{
    let response = http
        .execute_with_retry_fresh(Method::GET, url, || {
            let mut request = http.request(Method::GET, format!("{url}?{query}"));
            for (key, value) in sign(query)? {
                request = request.header(key, value);
            }
            Ok(request)
        })
        .await?;
    checked_body(response).await
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

fn decode<T: DeserializeOwned>(body: &str) -> ExchangeResult<T> {
    let response: BybitObjectResponse<T> = serde_json::from_str(body)
        .map_err(|error| invalid(format!("withdrawal response: {error}")))?;
    response.into_result("withdrawal")
}

fn query(params: &[(&str, &str)]) -> String {
    url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(params.iter().copied())
        .finish()
}
fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|v| !v.is_empty())
}
fn required<'a>(name: &str, value: &'a str) -> ExchangeResult<&'a str> {
    non_empty(Some(value)).ok_or_else(|| invalid(format!("{name} missing")))
}
fn decimal(name: &str, value: &str) -> ExchangeResult<Decimal> {
    value
        .trim()
        .parse::<Decimal>()
        .ok()
        .filter(|value| *value >= Decimal::ZERO)
        .ok_or_else(|| invalid(format!("invalid {name}")))
}
fn validate_venue(venue: &str) -> ExchangeResult<()> {
    if venue == "bybit" {
        Ok(())
    } else {
        Err(invalid("withdrawal venue mismatch"))
    }
}
fn invalid(message: impl Into<String>) -> ExchangeError {
    ExchangeError::Parse(format!("bybit: {}", message.into()))
}

#[cfg(test)]
mod tests;
