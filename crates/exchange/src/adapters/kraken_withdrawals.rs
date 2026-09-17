//! Kraken Funding withdrawals: pinned fees, exact saved addresses, one write.
//! <https://docs.kraken.com/exchange/guides/rest/funding>
//! Schemas: <https://docs.kraken.com/openapi/spot-rest.yaml>
use super::kraken_config::KrakenSpotCredentials;
use super::kraken_funding_rest::{decode, get, invalid, post_once, rows};
use super::kraken_symbols::{canonical_asset, funding_asset};
use super::kraken_transfer_networks::{methods, Amount, FundingMethod};
use crate::error::{ExchangeError, ExchangeResult};
use crate::http::HttpClient;
use crate::{
    TransferDestinationEvidence, TransferDestinationRequest, TransferDestinationStatus,
    TransferDirection, WithdrawalSourceBalance, WithdrawalSourceBalanceRequest, WithdrawalStatus,
    WithdrawalStatusEvidence, WithdrawalStatusRequest, WithdrawalSubmission,
    WithdrawalSubmitRequest, WithdrawalWalletType,
};
use chrono::{DateTime, Utc};
use reqwest::Method;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashSet;

const ADDRESSES: &str = "/funding/v1/addresses";
const WITHDRAWALS: &str = "/funding/v1/withdrawals";
const CLOCK_SKEW_MS: i64 = 300_000;

#[derive(Deserialize)]
struct SavedAddress {
    address_id: String,
    scope: Value,
    verified: bool,
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
impl SavedAddress {
    fn matches(&self, address: &str, tag: Option<&str>) -> ExchangeResult<bool> {
        let Some(crypto) = self.address_details.crypto.as_ref() else {
            return Ok(false);
        };
        let own_tag = nonempty(crypto.tag.as_deref());
        let memo = nonempty(crypto.memo.as_deref());
        if own_tag.is_some() && memo.is_some() && own_tag != memo {
            return Err(invalid("withdrawal tag/memo conflict"));
        }
        Ok(same_address(&crypto.address, address) && own_tag.or(memo) == nonempty(tag))
    }
}

async fn addresses(
    http: &HttpClient,
    base: &str,
    credentials: &KrakenSpotCredentials,
    method_id: &str,
) -> ExchangeResult<Vec<SavedAddress>> {
    let rows: Vec<SavedAddress> = rows(
        http,
        base,
        ADDRESSES,
        vec![
            ("scope[method_id]".into(), method_id.into()),
            ("limit".into(), "100".into()),
        ],
        "addresses",
        credentials,
    )
    .await?;
    let mut ids = HashSet::new();
    for row in &rows {
        let scope = row
            .scope
            .as_object()
            .ok_or_else(|| invalid("withdrawal address scope missing"))?;
        if row.address_id.trim().is_empty()
            || !ids.insert(&row.address_id)
            || scope.len() != 1
            || !scope.iter().any(|(key, value)| {
                matches!(
                    key.as_str(),
                    "method_id" | "network_id" | "network_group_id"
                ) && value.as_str().is_some_and(|v| !v.is_empty())
            })
            || scope
                .get("method_id")
                .is_some_and(|value| value.as_str() != Some(method_id))
        {
            return Err(invalid("withdrawal address scope or identity mismatch"));
        }
    }
    Ok(rows)
}

async fn method(
    http: &HttpClient,
    base: &str,
    credentials: &KrakenSpotCredentials,
    currency: &str,
    id: &str,
) -> ExchangeResult<FundingMethod> {
    validate_currency(currency)?;
    if id.len() != 36 || !id.bytes().all(|v| v.is_ascii_hexdigit() || v == b'-') {
        return Err(invalid("withdrawal method ID invalid"));
    }
    methods(http, base, credentials, currency, "withdraw")
        .await?
        .into_iter()
        .find(|method| {
            method.method_id == id
                && method
                    .network
                    .as_ref()
                    .is_some_and(|network| !network.network_id.is_empty())
        })
        .ok_or_else(|| invalid("requested crypto withdrawal method unavailable"))
}

pub(super) async fn destination(
    http: &HttpClient,
    base: &str,
    credentials: &KrakenSpotCredentials,
    request: &TransferDestinationRequest,
) -> ExchangeResult<TransferDestinationEvidence> {
    if request.direction != TransferDirection::WithdrawToChain {
        return Err(invalid("withdrawal direction mismatch"));
    }
    let expected = nonempty(request.expected_address.as_deref())
        .ok_or_else(|| invalid("expected withdrawal address missing"))?;
    method(http, base, credentials, &request.currency, &request.network).await?;
    let mut matching = Vec::new();
    for row in addresses(http, base, credentials, &request.network).await? {
        if row.matches(expected, request.expected_tag.as_deref())? {
            matching.push(row);
        }
    }
    let verified = matching.iter().any(|row| row.verified);
    Ok(TransferDestinationEvidence {
        venue:"kraken".into(),currency:canonical_asset(&request.currency),network:request.network.clone(),direction:request.direction,
        address:Some(expected.into()),tag:request.expected_tag.clone(),
        status:if verified {TransferDestinationStatus::Verified} else if matching.is_empty() {TransferDestinationStatus::Missing} else {TransferDestinationStatus::Unverified},
        allowlisted:Some(verified),checked_at_ms:common::time::now_ms(),source_url:format!("{base}{ADDRESSES}"),
        problem:(!verified).then(||"Kraken 没有已验证且与目标钱包、Tag/Memo 完全一致的提币地址，请先在交易所配置；产品不会自动添加地址".into()),
    })
}

#[derive(Deserialize)]
struct Limits {
    available_balance: Amount,
    withdrawal_limits: Vec<MethodLimit>,
}
#[derive(Deserialize)]
struct MethodLimit {
    method_id: String,
    maximum_amount: Amount,
    limits: Vec<WindowLimit>,
}
#[derive(Deserialize)]
struct WindowLimit {
    limit: Value,
}

async fn limits(
    http: &HttpClient,
    base: &str,
    credentials: &KrakenSpotCredentials,
    currency: &str,
) -> ExchangeResult<Limits> {
    validate_currency(currency)?;
    get(
        http,
        base,
        &format!(
            "/funding/v1/limits/withdrawal/currency/{}",
            funding_asset(currency)
        ),
        &[],
        credentials,
    )
    .await
}
pub(super) async fn source_balance(
    http: &HttpClient,
    base: &str,
    credentials: &KrakenSpotCredentials,
    request: &WithdrawalSourceBalanceRequest,
) -> ExchangeResult<WithdrawalSourceBalance> {
    validate_wallet(&request.venue, request.wallet_type)?;
    let row = limits(http, base, credentials, &request.currency).await?;
    Ok(WithdrawalSourceBalance {
        venue: "kraken".into(),
        currency: canonical_asset(&request.currency),
        wallet_type: request.wallet_type,
        available: row.available_balance.exact(&request.currency)?,
        checked_at_ms: common::time::now_ms(),
        source_url: format!(
            "{base}/funding/v1/limits/withdrawal/currency/{}",
            funding_asset(&request.currency)
        ),
    })
}

pub(super) async fn asset_step(
    http: &HttpClient,
    base: &str,
    currency: &str,
) -> ExchangeResult<Decimal> {
    validate_currency(currency)?;
    let url = format!("{base}/0/public/Assets?asset={}", funding_asset(currency));
    let response = http
        .execute_with_retry_fresh(Method::GET, &url, || Ok(http.request(Method::GET, &url)))
        .await?;
    let value: Value = decode(response).await?;
    if value
        .get("error")
        .and_then(Value::as_array)
        .is_none_or(|errors| !errors.is_empty())
    {
        return Err(invalid("Kraken asset precision unavailable"));
    }
    let result = value
        .get("result")
        .and_then(Value::as_object)
        .ok_or_else(|| invalid("asset precision result missing"))?;
    let mut matched = result
        .iter()
        .filter(|(asset, _)| canonical_asset(asset) == canonical_asset(currency));
    let Some((_, row)) = matched.next() else {
        return Err(invalid("requested asset precision missing"));
    };
    if matched.next().is_some()
        || row.get("aclass").and_then(Value::as_str) != Some("currency")
        || !matches!(
            row.get("status").and_then(Value::as_str),
            Some("enabled" | "withdrawal_only")
        )
    {
        return Err(invalid("asset identity or funding status not ready"));
    }
    let decimals = row
        .get("decimals")
        .and_then(Value::as_u64)
        .filter(|v| *v <= 28)
        .ok_or_else(|| invalid("asset accounting precision missing"))?;
    Ok(Decimal::new(1, decimals as u32))
}

#[derive(Deserialize)]
struct Quote {
    fee: Amount,
    net_amount: Amount,
    gross_amount: Amount,
    withdrawal_fee_token: String,
}

pub(super) async fn submit(
    http: &HttpClient,
    base: &str,
    credentials: &KrakenSpotCredentials,
    request: &WithdrawalSubmitRequest,
) -> ExchangeResult<WithdrawalSubmission> {
    let (body, net, fee, gross) = async {
    validate_wallet(&request.venue, request.wallet_type)?;
    if request.address.trim().is_empty()
        || request.client_withdrawal_id.trim().is_empty()
        || request.amount <= Decimal::ZERO
    {
        return Err(invalid(
            "withdrawal address, local identity or amount invalid",
        ));
    }
    let max_fee = request
        .max_fee
        .filter(|fee| *fee >= Decimal::ZERO)
        .ok_or_else(|| invalid("approved withdrawal fee ceiling missing"))?;
    let method = method(http, base, credentials, &request.currency, &request.network).await?;
    let step = asset_step(http, base, &request.currency).await?;
    if request.amount % step != Decimal::ZERO {
        return Err(invalid(
            "withdrawal amount exceeds asset accounting precision",
        ));
    }
    let mut candidates = Vec::new();
    for row in addresses(http, base, credentials, &request.network).await? {
        if row.verified && row.matches(&request.address, request.tag.as_deref())? {
            candidates.push(row);
        }
    }
    candidates.sort_by(|a, b| a.address_id.cmp(&b.address_id));
    let address = candidates
        .first()
        .ok_or_else(|| invalid("verified withdrawal destination missing"))?;
    let quoted_at = common::time::now_ms();
    let quote: Quote = get(
        http,
        base,
        &format!("/funding/v1/fees/{}", request.network),
        &[
            ("amount".into(), request.amount.normalize().to_string()),
            ("fee_included".into(), "false".into()),
        ],
        credentials,
    )
    .await?;
    let fee = quote.fee.exact(&request.currency)?;
    let net = quote.net_amount.exact(&request.currency)?;
    let gross = quote.gross_amount.exact(&request.currency)?;
    if net != request.amount
        || net.checked_add(fee) != Some(gross)
        || quote.withdrawal_fee_token.trim().is_empty()
    {
        return Err(invalid("withdrawal quote amount, fee or token mismatch"));
    }
    if fee > max_fee {
        return Err(invalid(format!(
            "Kraken 实时报价手续费 {fee} 超出已确认上限 {max_fee}，未提交提币"
        )));
    }
    let minimum = method
        .minimum_amount
        .as_deref()
        .ok_or_else(|| invalid("withdrawal minimum unknown"))
        .and_then(decimal)?;
    if net < minimum
        || method
            .maximum_amount
            .as_deref()
            .map(decimal)
            .transpose()?
            .is_some_and(|max| gross > max)
    {
        return Err(invalid("withdrawal amount outside method limits"));
    }
    let limits = limits(http, base, credentials, &request.currency).await?;
    check_limits(&limits, &request.network, &request.currency, gross)?;
    if !(0..240_000).contains(&common::time::now_ms().saturating_sub(quoted_at)) {
        return Err(invalid("withdrawal fee quote expired before submission"));
    }
    let body = json!({"scope":{"method_id":request.network},"address_id":address.address_id,"expected_address":request.address,
        "amount":{"asset_amount":{"asset":{"class":"currency","name":method.asset.name},"amount":net.normalize().to_string()}},
        "fee":{"quoted_fee":{"token":quote.withdrawal_fee_token},"fee_included":false}});
    Ok::<_, ExchangeError>((body, net, fee, gross))
    }.await.map_err(|error| ExchangeError::Api {
        exchange: "kraken".into(), code: "LOCAL_WITHDRAWAL_PREFLIGHT_REJECTED".into(), message: error.to_string(),
    })?;
    let ack: Value = post_once(http, base, WITHDRAWALS, &body, credentials).await?;
    let id = ack
        .get("withdrawal_id")
        .and_then(Value::as_str)
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| invalid("withdrawal receipt missing; do not resend"))?;
    // A returned ID must survive inconsistent amount fields so read-only recovery remains possible.
    let problem = check_ack_amounts(&ack, &request.currency, net, fee, gross)
        .err()
        .map(|error| {
            format!("Kraken 提币已返回编号，但回执金额需要核对；保留编号，仅查询不重发：{error}")
        });
    Ok(WithdrawalSubmission {
        venue: "kraken".into(),
        provider_withdrawal_id: id.into(),
        client_withdrawal_id: request.client_withdrawal_id.clone(),
        submitted_at_ms: common::time::now_ms(),
        source_url: format!("{base}{WITHDRAWALS}"),
        problem,
    })
}

fn check_limits(
    limits: &Limits,
    method: &str,
    currency: &str,
    gross: Decimal,
) -> ExchangeResult<()> {
    let mut matched = limits
        .withdrawal_limits
        .iter()
        .filter(|row| row.method_id == method);
    let row = matched
        .next()
        .ok_or_else(|| invalid("withdrawal method account limits missing"))?;
    if matched.next().is_some()
        || gross > limits.available_balance.exact(currency)?
        || gross > row.maximum_amount.exact(currency)?
    {
        return Err(invalid(
            "Kraken 可提余额或本充值方式剩余额度不足，未提交提币",
        ));
    }
    for window in &row.limits {
        match window.limit.get("limit_type").and_then(Value::as_str) {
            Some("attempt" | "success") => {
                let remaining = window
                    .limit
                    .get("remaining")
                    .and_then(Value::as_str)
                    .ok_or_else(|| invalid("withdrawal count limit missing"))
                    .and_then(decimal)?;
                if remaining < Decimal::ONE {
                    return Err(invalid("Kraken 本时间窗口提币次数已用完，未提交提币"));
                }
            }
            // The official maximum_amount already incorporates monetary limits in the requested asset.
            Some(
                "amount" | "equiv_amount_usd" | "equiv_amount_eur" | "equiv_amount_cad"
                | "equiv_amount_gbp",
            ) => (),
            _ => return Err(invalid("unknown withdrawal limit type")),
        }
    }
    Ok(())
}
fn check_ack_amounts(
    ack: &Value,
    currency: &str,
    net: Decimal,
    fee: Decimal,
    gross: Decimal,
) -> ExchangeResult<()> {
    for (field, expected) in [("net_amount", net), ("fee", fee), ("gross_amount", gross)] {
        let amount: Amount = serde_json::from_value(
            ack.get(field)
                .and_then(|v| v.get("asset_amount"))
                .cloned()
                .ok_or_else(|| invalid(format!("{field} missing")))?,
        )
        .map_err(|error| invalid(error.to_string()))?;
        if amount.exact(currency)? != expected {
            return Err(invalid(format!("withdrawal {field} differs from quote")));
        }
    }
    Ok(())
}

#[derive(Deserialize)]
struct History {
    withdrawal_id: String,
    amount: Amount,
    fee: Amount,
    method_id: String,
    status: String,
    create_time: String,
    address_id: Option<String>,
    onchain_transaction: Option<String>,
}
pub(super) async fn status(
    http: &HttpClient,
    base: &str,
    credentials: &KrakenSpotCredentials,
    request: &WithdrawalStatusRequest,
) -> ExchangeResult<Option<WithdrawalStatusEvidence>> {
    let now = common::time::now_ms();
    if request.venue != "kraken"
        || request.client_withdrawal_id.trim().is_empty()
        || request.address.trim().is_empty()
        || request.network.trim().is_empty()
        || request.submitted_at_ms <= 0
        || request.submitted_at_ms > now + CLOCK_SKEW_MS
    {
        return Err(invalid("invalid withdrawal history request"));
    }
    validate_currency(&request.currency)?;
    let id = nonempty(request.provider_withdrawal_id.as_deref()).ok_or_else(|| {
        invalid("Kraken 不能按本地提币号查询；缺少官方回执编号，请人工核对原提币，不重发")
    })?;
    let start = request.submitted_at_ms.saturating_sub(CLOCK_SKEW_MS).max(0);
    let end = now.max(request.submitted_at_ms);
    let history: Vec<History> = rows(
        http,
        base,
        WITHDRAWALS,
        vec![
            ("asset[class]".into(), "currency".into()),
            ("asset[name]".into(), funding_asset(&request.currency)),
            ("scope[method_id]".into(), request.network.clone()),
            ("limit".into(), "100".into()),
            ("start_time".into(), rfc3339(start)?),
            ("end_time".into(), rfc3339(end)?),
        ],
        "withdrawals",
        credentials,
    )
    .await?;
    let mut matched = history.iter().filter(|row| row.withdrawal_id == id);
    let Some(row) = matched.next() else {
        return Ok(None);
    };
    let created = DateTime::parse_from_rfc3339(&row.create_time)
        .map_err(|_| invalid("withdrawal timestamp malformed"))?
        .timestamp_millis();
    if matched.next().is_some()
        || row.method_id != request.network
        || created < start
        || created > end
    {
        return Err(invalid("withdrawal identity or time mismatch"));
    }
    let address_id = nonempty(row.address_id.as_deref())
        .ok_or_else(|| invalid("withdrawal saved-address identity missing"))?;
    let addresses = addresses(http, base, credentials, &request.network).await?;
    let address = addresses
        .iter()
        .find(|address| address.address_id == address_id)
        .ok_or_else(|| invalid("withdrawal original saved address unavailable"))?;
    if !address.matches(&request.address, request.tag.as_deref())? {
        return Err(invalid("withdrawal destination differs from original plan"));
    }
    let status = match row.status.as_str() {
        "pending" => WithdrawalStatus::Pending,
        "success" => WithdrawalStatus::Completed,
        "failed" => WithdrawalStatus::Failed,
        _ => return Err(invalid("unknown withdrawal status")),
    };
    let transaction_id = nonempty(row.onchain_transaction.as_deref()).map(str::to_owned);
    if status == WithdrawalStatus::Completed && transaction_id.is_none() {
        return Err(invalid(
            "successful withdrawal has no on-chain transaction ID",
        ));
    }
    Ok(Some(WithdrawalStatusEvidence {
        venue: "kraken".into(),
        provider_withdrawal_id: id.into(),
        client_withdrawal_id: request.client_withdrawal_id.clone(),
        currency: canonical_asset(&request.currency),
        network: request.network.clone(),
        address: request.address.clone(),
        amount: row.amount.exact(&request.currency)?,
        transaction_fee: row.fee.exact(&request.currency)?,
        status,
        transaction_id,
        confirmations: None,
        checked_at_ms: now,
        source_url: format!("{base}{WITHDRAWALS}"),
        problem: (status == WithdrawalStatus::Failed)
            .then(|| "Kraken 提币失败，请核对交易所原记录，不自动重发".into()),
    }))
}

fn validate_currency(value: &str) -> ExchangeResult<()> {
    if value.is_empty()
        || value.len() > 16
        || !value
            .bytes()
            .all(|v| v.is_ascii_alphanumeric() || matches!(v, b'.' | b'-'))
    {
        Err(invalid("funding asset invalid"))
    } else {
        Ok(())
    }
}
fn validate_wallet(venue: &str, wallet: WithdrawalWalletType) -> ExchangeResult<()> {
    if venue != "kraken" || wallet != WithdrawalWalletType::Spot {
        Err(ExchangeError::UnsupportedCapability(
            "Kraken withdrawals require Spot wallet",
        ))
    } else {
        Ok(())
    }
}
fn decimal(value: &str) -> ExchangeResult<Decimal> {
    Decimal::from_str_exact(value)
        .ok()
        .filter(|v| *v >= Decimal::ZERO)
        .ok_or_else(|| invalid("invalid exact nonnegative decimal"))
}
fn nonempty(value: Option<&str>) -> Option<&str> {
    value.filter(|v| !v.trim().is_empty())
}
fn same_address(left: &str, right: &str) -> bool {
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
fn rfc3339(value: i64) -> ExchangeResult<String> {
    DateTime::<Utc>::from_timestamp_millis(value)
        .map(|v| v.to_rfc3339())
        .ok_or_else(|| invalid("history time invalid"))
}

#[cfg(test)]
mod tests;
