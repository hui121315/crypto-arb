//! Read-only Kraken deposit address and credit evidence.
//! <https://docs.kraken.com/api-reference/funding-beta/list-funding-claimed-addresses-v2>
//! <https://docs.kraken.com/api-reference/funding-beta/list-funding-deposits>
//! Legacy DepositStatus schema: <https://docs.kraken.com/openapi/spot-rest.yaml>
use super::kraken_config::KrakenSpotCredentials;
use super::kraken_funding_rest::{invalid, rows};
use super::kraken_spot_rest::signed_post;
use super::kraken_symbols::{canonical_asset, funding_asset};
use super::kraken_transfer_networks::{methods, Amount, FundingMethod};
use crate::error::{ExchangeError, ExchangeResult};
use crate::http::HttpClient;
use crate::{
    DepositStatus, DepositStatusEvidence, DepositStatusRequest, TransferDestinationEvidence,
    TransferDestinationRequest, TransferDestinationStatus, TransferDirection,
};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeSet, HashSet};

const ADDRESS_PATH: &str = "/funding/v2/deposit/addresses";
const DEPOSITS_PATH: &str = "/funding/v1/deposits";
const LEGACY_PATH: &str = "/0/private/DepositStatus";
const CLOCK_SKEW_MS: i64 = 300_000;

#[derive(Deserialize)]
struct ClaimedAddress {
    method_id: String,
    expire_time: Option<String>,
    address_details: AddressDetails,
}
#[derive(Deserialize)]
struct AddressDetails {
    crypto: Option<CryptoAddress>,
}
#[derive(Deserialize)]
struct CryptoAddress {
    address: String,
    tag: Option<String>,
    memo: Option<String>,
}

pub(super) async fn destination(
    http: &HttpClient,
    base: &str,
    credentials: &KrakenSpotCredentials,
    request: &TransferDestinationRequest,
) -> ExchangeResult<TransferDestinationEvidence> {
    if request.direction != TransferDirection::DepositToVenue {
        return Err(ExchangeError::UnsupportedCapability(
            "Kraken withdrawal destination",
        ));
    }
    let method = exact_method(http, base, credentials, &request.currency, &request.network).await?;
    let mut evidence = TransferDestinationEvidence {
        venue: "kraken".into(),
        currency: canonical_asset(&request.currency),
        network: request.network.clone(),
        direction: request.direction,
        address: None,
        tag: None,
        status: TransferDestinationStatus::Unverified,
        allowlisted: None,
        checked_at_ms: common::time::now_ms(),
        source_url: format!("{base}{ADDRESS_PATH}"),
        problem: None,
    };
    if let Some(problem) = preflight_problem(&method, request.amount) {
        evidence.problem = Some(problem);
        return Ok(evidence);
    }
    let addresses: Vec<ClaimedAddress> = rows(
        http,
        base,
        ADDRESS_PATH,
        vec![
            ("scope[method_id]".into(), request.network.clone()),
            ("limit".into(), "100".into()),
        ],
        "addresses",
        credentials,
    )
    .await?;
    let mut candidates = BTreeSet::new();
    for row in addresses {
        if row.method_id != request.network {
            return Err(invalid("claimed address method mismatch"));
        }
        if let Some(expiry) = row.expire_time.as_deref() {
            if timestamp(expiry)? <= evidence.checked_at_ms.saturating_add(60_000) {
                continue;
            }
        }
        let Some(crypto) = row.address_details.crypto else {
            continue;
        };
        if crypto.address.trim().is_empty() {
            return Err(invalid("empty claimed address"));
        }
        let tag = nonempty(crypto.tag.as_deref());
        let memo = nonempty(crypto.memo.as_deref());
        if tag.is_some() && memo.is_some() && tag != memo {
            return Err(invalid("claimed address tag and memo conflict"));
        }
        let tag = tag.or(memo).map(str::to_owned);
        if request
            .expected_address
            .as_deref()
            .is_some_and(|expected| !same_identifier(expected, &crypto.address))
            || request
                .expected_tag
                .as_deref()
                .is_some_and(|expected| Some(expected) != tag.as_deref())
        {
            continue;
        }
        candidates.insert((crypto.address, tag));
    }
    if let Some((address, tag)) = candidates.into_iter().next() {
        evidence.address = Some(address);
        evidence.tag = tag;
        if evidence.tag.is_some() {
            evidence.problem =
                Some("Kraken 该地址需要 Tag/Memo，当前缺少独立到账证明，暂不自动转入".into());
        } else {
            evidence.status = TransferDestinationStatus::Verified;
        }
    } else {
        evidence.status = TransferDestinationStatus::Missing;
        evidence.problem = Some("Kraken 没有匹配且有效的已领取充值地址，请先在 Kraken 为该资产和网络领取地址；产品不会自动创建可能收费的地址".into());
    }
    Ok(evidence)
}

async fn exact_method(
    http: &HttpClient,
    base: &str,
    credentials: &KrakenSpotCredentials,
    currency: &str,
    id: &str,
) -> ExchangeResult<FundingMethod> {
    if currency.trim().is_empty() || id.trim().is_empty() {
        return Err(invalid("deposit currency or method missing"));
    }
    methods(http, base, credentials, currency, "deposit")
        .await?
        .into_iter()
        .find(|method| {
            method.method_id == id
                && method
                    .network
                    .as_ref()
                    .is_some_and(|network| !network.network_id.is_empty())
        })
        .ok_or_else(|| invalid("requested crypto deposit method unavailable"))
}

fn preflight_problem(method: &FundingMethod, amount: Option<Decimal>) -> Option<String> {
    if !method.zero_deposit_fee() {
        return Some("Kraken 该充值方式有费用或费用未知，净到账尚未核实，暂不自动转入".into());
    }
    let Some(amount) = amount.filter(|value| *value > Decimal::ZERO) else {
        return Some("缺少精确转入金额，无法核对 Kraken 充值限额".into());
    };
    let Some(minimum) = method
        .minimum_amount
        .as_deref()
        .and_then(|v| exact_decimal(v).ok())
    else {
        return Some("Kraken 充值最低金额未知，暂不自动转入".into());
    };
    if amount < minimum {
        return Some(format!(
            "转入金额 {amount} 低于 Kraken 最低充值金额 {minimum}"
        ));
    }
    if let Some(maximum) = method.maximum_amount.as_deref() {
        match exact_decimal(maximum) {
            Ok(maximum) if amount <= maximum => (),
            Ok(maximum) => {
                return Some(format!("转入金额 {amount} 超过 Kraken 充值上限 {maximum}"))
            }
            Err(_) => return Some("Kraken 充值上限无法核实".into()),
        }
    }
    None
}

#[derive(Deserialize)]
struct LegacyDeposit {
    aclass: String,
    asset: String,
    refid: String,
    txid: String,
    info: String,
    amount: String,
    fee: Option<String>,
    time: i64,
    status: String,
    #[serde(rename = "status-prop")]
    status_prop: Option<String>,
    #[serde(default)]
    originators: Vec<String>,
}
#[derive(Deserialize)]
struct FundingDeposit {
    deposit_id: String,
    method_id: String,
    network_id: String,
    status: String,
    amount: Option<Amount>,
    fee: Option<Amount>,
    create_time: String,
}

pub(super) async fn status(
    http: &HttpClient,
    base: &str,
    credentials: &KrakenSpotCredentials,
    request: &DepositStatusRequest,
) -> ExchangeResult<Option<DepositStatusEvidence>> {
    let now = common::time::now_ms();
    if request.venue != "kraken"
        || request.address.trim().is_empty()
        || request.transaction_id.trim().is_empty()
        || request.amount <= Decimal::ZERO
        || request.submitted_at_ms <= 0
        || request.submitted_at_ms > now + CLOCK_SKEW_MS
    {
        return Err(invalid("invalid deposit status request"));
    }
    let method = exact_method(http, base, credentials, &request.currency, &request.network).await?;
    let start = request.submitted_at_ms.saturating_sub(CLOCK_SKEW_MS).max(0);
    let end = now.max(request.submitted_at_ms);
    let history = legacy_history(http, base, credentials, &request.currency, start, end).await?;
    // Funding v1 has no tx hash. Never join deposits by amount or timestamp alone.
    let mut matching = history.iter().filter(|row| {
        same_identifier(&row.txid, &request.transaction_id)
            || row
                .originators
                .iter()
                .any(|tx| same_identifier(tx, &request.transaction_id))
    });
    let Some(legacy) = matching.next() else {
        return Ok(None);
    };
    if matching.next().is_some() {
        return Err(invalid(
            "multiple records match original deposit transaction",
        ));
    }
    if legacy.aclass != "currency"
        || canonical_asset(&legacy.asset) != canonical_asset(&request.currency)
        || !same_identifier(&legacy.info, &request.address)
        || legacy.refid.trim().is_empty()
        || legacy.time.saturating_mul(1000) < start
        || legacy.time.saturating_mul(1000) > end
    {
        return Err(invalid(
            "deposit transaction asset, address or time mismatch",
        ));
    }
    let mut evidence = DepositStatusEvidence {
        venue: "kraken".into(),
        currency: canonical_asset(&request.currency),
        network: request.network.clone(),
        address: request.address.clone(),
        tag: request.tag.clone(),
        amount: exact_decimal(&legacy.amount)?,
        deposit_fee: None,
        status: DepositStatus::Pending,
        transaction_id: request.transaction_id.clone(),
        confirmations: None,
        checked_at_ms: now,
        source_url: format!("{base}{LEGACY_PATH} + {base}{DEPOSITS_PATH}"),
        problem: None,
    };
    let blocked = if legacy.status_prop.as_deref().is_some_and(|v| !v.is_empty()) {
        Some("Kraken 充值处于审核、退回或未知附加状态，需要人工核对")
    } else if request.tag.as_deref().is_some_and(|tag| !tag.is_empty()) {
        Some("Kraken 充值历史没有独立 Tag/Memo 字段，暂不能证明本笔到账")
    } else if !legacy.originators.is_empty()
        && (legacy.originators.len() != 1
            || !same_identifier(&legacy.originators[0], &request.transaction_id))
    {
        Some("Kraken 合并归集了多笔交易，不能把总到账量记到本笔转账")
    } else {
        None
    };
    if let Some(problem) = blocked {
        evidence.status = DepositStatus::Blocked;
        evidence.problem = Some(problem.into());
        return Ok(Some(evidence));
    }
    let funding: Vec<FundingDeposit> = rows(
        http,
        base,
        DEPOSITS_PATH,
        vec![
            ("asset[class]".into(), "currency".into()),
            ("asset[name]".into(), funding_asset(&request.currency)),
            ("scope[method_id]".into(), request.network.clone()),
            ("limit".into(), "100".into()),
            ("start_time".into(), rfc3339(start)?),
            ("end_time".into(), rfc3339(end)?),
        ],
        "deposits",
        credentials,
    )
    .await?;
    let mut matching = funding.iter().filter(|row| row.deposit_id == legacy.refid);
    let Some(funding) = matching.next() else {
        evidence.problem =
            Some("Kraken 新旧充值记录尚未按原记录编号对上，继续等待，不按金额猜测".into());
        return Ok(Some(evidence));
    };
    if matching.next().is_some() {
        return Err(invalid("duplicate Funding deposit id"));
    }
    if funding.method_id != request.network
        || method
            .network
            .as_ref()
            .is_none_or(|n| n.network_id != funding.network_id)
        || timestamp(&funding.create_time)? < start
        || timestamp(&funding.create_time)? > end
    {
        return Err(invalid("Funding deposit method, network or time mismatch"));
    }
    if let Some(amount) = funding.amount.as_ref() {
        if amount.exact(&request.currency)? != evidence.amount {
            return Err(invalid("Funding and legacy deposit amounts disagree"));
        }
    }
    evidence.deposit_fee = funding
        .fee
        .as_ref()
        .map(|fee| fee.exact(&request.currency))
        .transpose()?;
    if let (Some(fee), Some(legacy_fee)) = (evidence.deposit_fee, legacy.fee.as_deref()) {
        if fee != exact_decimal(legacy_fee)? {
            return Err(invalid("Funding and legacy deposit fees disagree"));
        }
    }
    evidence.status = match (legacy.status.as_str(), funding.status.as_str()) {
        ("Success", "success")
            if funding.amount.is_some()
                && evidence.deposit_fee == Some(Decimal::ZERO)
                && legacy
                    .fee
                    .as_deref()
                    .and_then(|fee| exact_decimal(fee).ok())
                    == Some(Decimal::ZERO) =>
        {
            DepositStatus::Completed
        }
        ("Success", "success") => {
            evidence.problem = Some("Kraken 已报告完成，但充值费或净到账量还未核实".into());
            DepositStatus::Blocked
        }
        ("Failure", "failure") => DepositStatus::Failed,
        (
            "Initial" | "Pending" | "EarlyConfirmed" | "Settled" | "Success" | "Failure",
            "initial" | "pending" | "settled" | "success" | "failure",
        ) => {
            evidence.problem = Some("Kraken 充值尚未在新旧记录中同时完成，暂不计入可用资金".into());
            DepositStatus::Pending
        }
        _ => return Err(invalid("unrecognized deposit status")),
    };
    Ok(Some(evidence))
}

async fn legacy_history(
    http: &HttpClient,
    base: &str,
    credentials: &KrakenSpotCredentials,
    currency: &str,
    start: i64,
    end: i64,
) -> ExchangeResult<Vec<LegacyDeposit>> {
    let mut cursor = "true".to_owned();
    let mut seen = HashSet::new();
    let mut deposits = Vec::new();
    for _ in 0..5 {
        let body = signed_post(
            http,
            base,
            LEGACY_PATH,
            vec![
                ("asset".into(), funding_asset(currency)),
                ("aclass".into(), "currency".into()),
                ("start".into(), (start / 1000).to_string()),
                ("end".into(), (end / 1000 + 1).to_string()),
                ("cursor".into(), cursor.clone()),
                ("limit".into(), "100".into()),
            ],
            credentials,
        )
        .await?;
        let value: Value = serde_json::from_str(&body).map_err(|e| invalid(e.to_string()))?;
        let result = value
            .get("result")
            .ok_or_else(|| invalid("DepositStatus result missing"))?;
        let page = result
            .get("deposit")
            .and_then(Value::as_array)
            .ok_or_else(|| invalid("DepositStatus paginated rows missing"))?;
        if page.len() > 100 {
            return Err(invalid("DepositStatus page exceeds limit"));
        }
        for row in page {
            deposits.push(
                serde_json::from_value(row.clone())
                    .map_err(|e| invalid(format!("DepositStatus: {e}")))?,
            );
        }
        let next = match result.get("next_cursor") {
            None => return Ok(deposits),
            Some(Value::String(value)) if value.is_empty() => return Ok(deposits),
            Some(Value::String(value)) => value.clone(),
            _ => return Err(invalid("DepositStatus invalid cursor")),
        };
        if !seen.insert(next.clone()) {
            break;
        }
        cursor = next;
    }
    Err(invalid("DepositStatus pagination incomplete"))
}

fn nonempty(value: Option<&str>) -> Option<&str> {
    value.filter(|v| !v.trim().is_empty())
}
fn exact_decimal(value: &str) -> ExchangeResult<Decimal> {
    Decimal::from_str_exact(value)
        .ok()
        .filter(|value| *value >= Decimal::ZERO)
        .ok_or_else(|| invalid("invalid nonnegative amount"))
}
fn timestamp(value: &str) -> ExchangeResult<i64> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.timestamp_millis())
        .map_err(|_| invalid("invalid Funding timestamp"))
}
fn rfc3339(value: i64) -> ExchangeResult<String> {
    DateTime::<Utc>::from_timestamp_millis(value)
        .map(|value| value.to_rfc3339())
        .ok_or_else(|| invalid("invalid history time"))
}
fn same_identifier(left: &str, right: &str) -> bool {
    // Only hex identifiers may ignore case; Solana signatures and addresses may not.
    if left.starts_with("0x")
        && right.starts_with("0x")
        && left[2..].bytes().all(|v| v.is_ascii_hexdigit())
        && right[2..].bytes().all(|v| v.is_ascii_hexdigit())
    {
        left.eq_ignore_ascii_case(right)
    } else {
        left == right
    }
}

#[cfg(test)]
mod tests;
