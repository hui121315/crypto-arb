//! Private funding-payment parsing and request helpers.

use crate::adapter::strip_common_suffixes;
use crate::adapters::kucoin_market_data::kucoin_to_normalized;
use crate::error::{ExchangeError, ExchangeResult};
use serde::Deserialize;
use serde_json::Value;
use shared_types::FundingPaymentData;
use std::collections::HashSet;

const BINANCE: &str = "binance";
const OKX: &str = "okx";
const BYBIT: &str = "bybit";
const BITGET: &str = "bitget";
const GATE: &str = "gate";
const KUCOIN: &str = "kucoin";
const BINANCE_FUNDING_INCOME_TYPE: &str = "FUNDING_FEE";
const OKX_FUNDING_SUBTYPE: &str = "173";
const BYBIT_FUNDING_TYPE: &str = "SETTLEMENT";
const BITGET_UTA_FUNDING_TYPES: &[&str] = &[
    "CONTRACT_MAIN_SETTLE_FEE_USER_IN",
    "CONTRACT_MAIN_SETTLE_FEE_USER_OUT",
    "MARGIN_SETTLE_FEE_USER_IN",
    "MARGIN_SETTLE_FEE_USER_OUT",
];
const GATE_FUNDING_TYPE: &str = "fund";
pub(super) const BINANCE_FUNDING_INCOME_PATH: &str = "/fapi/v1/income";
pub(super) const OKX_FUNDING_BILLS_PATH: &str = "/api/v5/account/bills";
pub(super) const BYBIT_TRANSACTION_LOG_PATH: &str = "/v5/account/transaction-log";
pub(super) const BITGET_UTA_FINANCIAL_RECORDS_PATH: &str = "/api/v3/account/financial-records";
pub(super) const GATE_ACCOUNT_BOOK_PATH: &str = "/api/v4/futures/usdt/account_book";
pub(super) const KUCOIN_FUNDING_HISTORY_PATH: &str = "/api/v1/funding-history";
const MAX_FUNDING_PAYMENT_PAGES: usize = 100;

#[derive(Debug)]
pub(super) struct FundingPaymentPage {
    payments: Vec<FundingPaymentData>,
    continuation: Option<String>,
    source_row_count: usize,
}

pub(super) struct FundingPaymentPagination {
    venue: &'static str,
    page_count: usize,
    continuations: HashSet<String>,
}

impl FundingPaymentPagination {
    pub(super) fn new(venue: &'static str) -> Self {
        Self {
            venue,
            page_count: 0,
            continuations: HashSet::new(),
        }
    }

    pub(super) fn accept(
        &mut self,
        page: FundingPaymentPage,
        payments: &mut Vec<FundingPaymentData>,
        venue_event_ids: &mut HashSet<String>,
    ) -> ExchangeResult<Option<String>> {
        self.page_count += 1;
        for payment in page.payments {
            if venue_event_ids.insert(payment.venue_event_id.clone()) {
                payments.push(payment);
            }
        }

        let Some(continuation) = page.continuation else {
            return Ok(None);
        };
        if page.source_row_count == 0 {
            return Err(pagination_error(self.venue, "zero-row continuation"));
        }
        if !self.continuations.insert(continuation.clone()) {
            return Err(pagination_error(self.venue, "repeated continuation"));
        }
        if self.page_count >= MAX_FUNDING_PAYMENT_PAGES {
            return Err(pagination_error(self.venue, "100-page cap reached"));
        }
        Ok(Some(continuation))
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct BinanceIncomeRow {
    #[serde(default)]
    symbol: String,
    #[serde(default)]
    income_type: String,
    #[serde(default)]
    income: Value,
    #[serde(default)]
    asset: String,
    #[serde(default)]
    time: Value,
    #[serde(default)]
    tran_id: Value,
}

#[derive(Debug, Deserialize)]
pub(super) struct OkxBillRow {
    #[serde(default, rename = "instId")]
    inst_id: String,
    #[serde(default, rename = "billId")]
    bill_id: String,
    #[serde(default)]
    ccy: String,
    #[serde(default)]
    pnl: Value,
    #[serde(default, rename = "subType")]
    sub_type: String,
    #[serde(default)]
    ts: Value,
}

#[derive(Debug, Deserialize)]
pub(super) struct BybitTransactionLogRow {
    #[serde(default)]
    id: String,
    #[serde(default)]
    symbol: String,
    #[serde(default)]
    funding: Value,
    #[serde(default)]
    currency: String,
    #[serde(default, rename = "transactionTime")]
    transaction_time: Value,
    #[serde(default, rename = "type")]
    row_type: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct BybitTransactionLogPage {
    #[serde(default)]
    list: Vec<BybitTransactionLogRow>,
    #[serde(default)]
    next_page_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct BitgetUtaFinancialRecordsPage {
    #[serde(default)]
    list: Vec<BitgetUtaFinancialRecordRow>,
    #[serde(default)]
    cursor: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BitgetUtaFinancialRecordRow {
    #[serde(default)]
    id: Value,
    #[serde(default)]
    symbol: String,
    #[serde(default, rename = "type")]
    row_type: String,
    #[serde(default)]
    amount: Value,
    #[serde(default)]
    coin: String,
    #[serde(default)]
    ts: Value,
}

#[derive(Debug, Deserialize)]
pub(super) struct GateAccountBookRow {
    #[serde(default)]
    time: Value,
    #[serde(default)]
    change: Value,
    #[serde(default, rename = "type")]
    row_type: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    contract: String,
    #[serde(default)]
    id: Value,
    #[serde(default)]
    trade_id: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct KucoinFundingHistoryPage {
    #[serde(default, rename = "dataList")]
    data_list: Vec<KucoinFundingHistoryRow>,
    #[serde(default)]
    has_more: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct KucoinFundingHistoryRow {
    #[serde(default)]
    id: Value,
    #[serde(default)]
    symbol: String,
    #[serde(default)]
    funding: Value,
    #[serde(default)]
    settle_currency: String,
    #[serde(default)]
    time_point: Value,
}

pub(super) fn binance_funding_income_params(
    exchange_symbol: Option<&str>,
    start_time_ms: Option<i64>,
    end_time_ms: Option<i64>,
    limit: u16,
) -> Vec<(&'static str, String)> {
    let mut params = Vec::new();
    if let Some(symbol) = exchange_symbol {
        params.push(("symbol", symbol.to_owned()));
    }
    params.push(("incomeType", BINANCE_FUNDING_INCOME_TYPE.to_owned()));
    if let Some(start) = start_time_ms {
        params.push(("startTime", start.to_string()));
    }
    if let Some(end) = end_time_ms {
        params.push(("endTime", end.to_string()));
    }
    params.push(("limit", limit.min(1_000).to_string()));
    params.push(("recvWindow", "5000".to_owned()));
    params
}

pub(super) fn okx_funding_bills_path(
    inst_id: Option<&str>,
    begin_ms: Option<i64>,
    end_ms: Option<i64>,
) -> String {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    serializer
        .append_pair("instType", "SWAP")
        .append_pair("subType", OKX_FUNDING_SUBTYPE)
        .append_pair("limit", "100");
    if let Some(inst_id) = inst_id {
        serializer.append_pair("instId", inst_id);
    }
    if let Some(begin) = begin_ms {
        serializer.append_pair("begin", &begin.to_string());
    }
    if let Some(end) = end_ms {
        serializer.append_pair("end", &end.to_string());
    }
    format!("{}?{}", OKX_FUNDING_BILLS_PATH, serializer.finish())
}

pub(super) fn bybit_transaction_log_query(
    account_type: &str,
    base_coin: Option<&str>,
    start_time_ms: Option<i64>,
    end_time_ms: Option<i64>,
    cursor: Option<&str>,
) -> String {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    serializer
        .append_pair("accountType", account_type)
        .append_pair("category", "linear")
        .append_pair("type", BYBIT_FUNDING_TYPE)
        .append_pair("limit", "50");
    if let Some(base_coin) = base_coin {
        serializer.append_pair("baseCoin", base_coin);
    }
    if let Some(start) = start_time_ms {
        serializer.append_pair("startTime", &start.to_string());
    }
    if let Some(end) = end_time_ms {
        serializer.append_pair("endTime", &end.to_string());
    }
    let mut query = serializer.finish();
    if let Some(cursor) = cursor {
        query.push_str("&cursor=");
        query.push_str(cursor);
    }
    query
}

pub(super) fn bitget_uta_financial_records_path(
    funding_type: &str,
    start_time_ms: Option<i64>,
    end_time_ms: Option<i64>,
    cursor: Option<&str>,
) -> String {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    serializer
        .append_pair("category", "USDT-FUTURES")
        .append_pair("type", funding_type)
        .append_pair("limit", "100");
    if let Some(start) = start_time_ms {
        serializer.append_pair("startTime", &start.to_string());
    }
    if let Some(end) = end_time_ms {
        serializer.append_pair("endTime", &end.to_string());
    }
    if let Some(cursor) = cursor {
        serializer.append_pair("cursor", cursor);
    }
    format!(
        "{}?{}",
        BITGET_UTA_FINANCIAL_RECORDS_PATH,
        serializer.finish()
    )
}

pub(super) fn gate_account_book_query(
    contract: Option<&str>,
    start_time_ms: Option<i64>,
    end_time_ms: Option<i64>,
) -> String {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    serializer
        .append_pair("type", GATE_FUNDING_TYPE)
        .append_pair("limit", "100");
    if let Some(contract) = contract {
        serializer.append_pair("contract", contract);
    }
    if let Some(start) = start_time_ms {
        serializer.append_pair("from", &ms_to_seconds_floor(start).to_string());
    }
    if let Some(end) = end_time_ms {
        serializer.append_pair("to", &ms_to_seconds_floor(end).to_string());
    }
    serializer.finish()
}

pub(super) fn kucoin_funding_history_path(
    exchange_symbol: Option<&str>,
    start_time_ms: Option<i64>,
    end_time_ms: Option<i64>,
    offset: Option<&str>,
) -> String {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    if let Some(symbol) = exchange_symbol {
        serializer.append_pair("symbol", symbol);
    }
    if let Some(start) = start_time_ms {
        serializer.append_pair("startAt", &start.to_string());
    }
    if let Some(end) = end_time_ms {
        serializer.append_pair("endAt", &end.to_string());
    }
    if let Some(offset) = offset {
        serializer.append_pair("offset", offset);
    }
    serializer.append_pair("maxCount", "100");
    format!("{}?{}", KUCOIN_FUNDING_HISTORY_PATH, serializer.finish())
}

pub(super) fn parse_binance_funding_payments(
    rows: Vec<BinanceIncomeRow>,
) -> ExchangeResult<Vec<FundingPaymentData>> {
    rows.into_iter()
        .filter(|row| {
            row.income_type
                .eq_ignore_ascii_case(BINANCE_FUNDING_INCOME_TYPE)
        })
        .map(parse_binance_payment)
        .filter_map(Result::transpose)
        .collect()
}

pub(super) fn parse_okx_funding_payments(
    rows: Vec<OkxBillRow>,
) -> ExchangeResult<Vec<FundingPaymentData>> {
    rows.into_iter()
        .filter(|row| row.sub_type == OKX_FUNDING_SUBTYPE)
        .map(parse_okx_payment)
        .filter_map(Result::transpose)
        .collect()
}

pub(super) fn parse_bybit_funding_payments(
    page: BybitTransactionLogPage,
) -> ExchangeResult<FundingPaymentPage> {
    let source_row_count = page.list.len();
    let payments = page
        .list
        .into_iter()
        .filter(|row| row.row_type.eq_ignore_ascii_case(BYBIT_FUNDING_TYPE))
        .map(parse_bybit_payment)
        .filter_map(Result::transpose)
        .collect::<ExchangeResult<Vec<_>>>()?;
    Ok(FundingPaymentPage {
        payments,
        continuation: page.next_page_cursor.and_then(non_empty_continuation),
        source_row_count,
    })
}

pub(super) fn parse_bitget_funding_payments(
    page: BitgetUtaFinancialRecordsPage,
) -> ExchangeResult<FundingPaymentPage> {
    let source_row_count = page.list.len();
    let payments = page
        .list
        .into_iter()
        .filter(|row| is_bitget_uta_funding_type(&row.row_type))
        .map(parse_bitget_payment)
        .filter_map(Result::transpose)
        .collect::<ExchangeResult<Vec<_>>>()?;
    Ok(FundingPaymentPage {
        payments,
        continuation: non_empty_continuation(page.cursor),
        source_row_count,
    })
}

pub(super) fn parse_gate_funding_payments(
    rows: &[GateAccountBookRow],
    settle_currency: &str,
) -> ExchangeResult<Vec<FundingPaymentData>> {
    rows.iter()
        .filter(|row| row.row_type.eq_ignore_ascii_case(GATE_FUNDING_TYPE))
        .map(|row| parse_gate_payment(row, settle_currency))
        .filter_map(Result::transpose)
        .collect()
}

pub(super) fn parse_kucoin_funding_payments(
    page: KucoinFundingHistoryPage,
) -> ExchangeResult<FundingPaymentPage> {
    let source_row_count = page.data_list.len();
    let continuation = kucoin_continuation(&page)?;
    let payments = page
        .data_list
        .into_iter()
        .map(parse_kucoin_payment)
        .filter_map(Result::transpose)
        .collect::<ExchangeResult<Vec<_>>>()?;
    Ok(FundingPaymentPage {
        payments,
        continuation,
        source_row_count,
    })
}

fn parse_binance_payment(row: BinanceIncomeRow) -> ExchangeResult<Option<FundingPaymentData>> {
    let amount = parse_f64(BINANCE, "income", &row.income)?;
    if amount == 0.0 {
        return Ok(None);
    }
    require_non_empty(BINANCE, "symbol", &row.symbol)?;
    require_non_empty(BINANCE, "asset", &row.asset)?;
    let time_ms = parse_i64(BINANCE, "time", &row.time)?;
    let tran_id = event_id_value(BINANCE, "tranId", &row.tran_id)?;
    Ok(Some(payment(
        BINANCE,
        strip_common_suffixes(&row.symbol),
        amount,
        row.asset,
        time_ms,
        format!("binance_funding:{tran_id}"),
    )))
}

fn parse_okx_payment(row: OkxBillRow) -> ExchangeResult<Option<FundingPaymentData>> {
    let amount = parse_f64(OKX, "pnl", &row.pnl)?;
    if amount == 0.0 {
        return Ok(None);
    }
    require_non_empty(OKX, "instId", &row.inst_id)?;
    require_non_empty(OKX, "ccy", &row.ccy)?;
    require_non_empty(OKX, "billId", &row.bill_id)?;
    let time_ms = parse_i64(OKX, "ts", &row.ts)?;
    Ok(Some(payment(
        OKX,
        strip_common_suffixes(&row.inst_id),
        amount,
        row.ccy,
        time_ms,
        format!("okx_funding:{}", row.bill_id),
    )))
}

fn parse_bybit_payment(row: BybitTransactionLogRow) -> ExchangeResult<Option<FundingPaymentData>> {
    if is_blank_value(&row.funding) {
        return Ok(None);
    }
    let amount = parse_f64(BYBIT, "funding", &row.funding)?;
    if amount == 0.0 {
        return Ok(None);
    }
    require_non_empty(BYBIT, "symbol", &row.symbol)?;
    require_non_empty(BYBIT, "currency", &row.currency)?;
    require_non_empty(BYBIT, "id", &row.id)?;
    let time_ms = parse_i64(BYBIT, "transactionTime", &row.transaction_time)?;
    Ok(Some(payment(
        BYBIT,
        strip_common_suffixes(&row.symbol),
        amount,
        row.currency,
        time_ms,
        format!("bybit_funding:{}:{time_ms}", row.id),
    )))
}

fn parse_bitget_payment(
    row: BitgetUtaFinancialRecordRow,
) -> ExchangeResult<Option<FundingPaymentData>> {
    let amount = parse_f64(BITGET, "amount", &row.amount)?;
    if amount == 0.0 {
        return Ok(None);
    }
    require_non_empty(BITGET, "symbol", &row.symbol)?;
    require_non_empty(BITGET, "coin", &row.coin)?;
    let id = event_id_value(BITGET, "id", &row.id)?;
    let time_ms = parse_i64(BITGET, "ts", &row.ts)?;
    Ok(Some(payment(
        BITGET,
        strip_common_suffixes(&row.symbol),
        amount,
        row.coin,
        time_ms,
        format!("bitget_funding:{id}"),
    )))
}

fn parse_gate_payment(
    row: &GateAccountBookRow,
    settle_currency: &str,
) -> ExchangeResult<Option<FundingPaymentData>> {
    let amount = parse_f64(GATE, "change", &row.change)?;
    if amount == 0.0 {
        return Ok(None);
    }
    require_non_empty(GATE, "contract", &row.contract)?;
    require_non_empty(GATE, "settle", settle_currency)?;
    let event_id = gate_event_id(row)?;
    let time_ms = parse_unix_seconds_ms(GATE, "time", &row.time)?;
    Ok(Some(payment(
        GATE,
        strip_common_suffixes(&row.contract),
        amount,
        settle_currency.to_ascii_uppercase(),
        time_ms,
        format!("gate_funding:{event_id}"),
    )))
}

fn parse_kucoin_payment(
    row: KucoinFundingHistoryRow,
) -> ExchangeResult<Option<FundingPaymentData>> {
    let amount = parse_f64(KUCOIN, "funding", &row.funding)?;
    if amount == 0.0 {
        return Ok(None);
    }
    require_non_empty(KUCOIN, "symbol", &row.symbol)?;
    require_non_empty(KUCOIN, "settleCurrency", &row.settle_currency)?;
    let id = event_id_value(KUCOIN, "id", &row.id)?;
    let time_ms = parse_i64(KUCOIN, "timePoint", &row.time_point)?;
    Ok(Some(payment(
        KUCOIN,
        kucoin_to_normalized(&row.symbol),
        amount,
        row.settle_currency,
        time_ms,
        format!("kucoin_funding:{id}"),
    )))
}

fn payment(
    venue: &str,
    symbol: String,
    amount: f64,
    currency: String,
    funding_time_ms: i64,
    venue_event_id: String,
) -> FundingPaymentData {
    FundingPaymentData {
        venue: venue.to_owned(),
        symbol,
        amount,
        currency,
        funding_time_ms,
        venue_event_id,
    }
}

fn parse_f64(venue: &str, field: &str, value: &Value) -> ExchangeResult<f64> {
    let parsed = match value {
        Value::String(raw) => raw
            .trim()
            .parse::<f64>()
            .map_err(|_| parse_error(venue, field, value))?,
        Value::Number(number) => number
            .as_f64()
            .ok_or_else(|| parse_error(venue, field, value))?,
        _ => return Err(parse_error(venue, field, value)),
    };
    if parsed.is_finite() {
        Ok(parsed)
    } else {
        Err(parse_error(venue, field, value))
    }
}

fn parse_i64(venue: &str, field: &str, value: &Value) -> ExchangeResult<i64> {
    let parsed = match value {
        Value::String(raw) => raw
            .trim()
            .parse::<i64>()
            .map_err(|_| parse_error(venue, field, value))?,
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_u64().and_then(|value| i64::try_from(value).ok()))
            .ok_or_else(|| parse_error(venue, field, value))?,
        _ => return Err(parse_error(venue, field, value)),
    };
    if parsed > 0 {
        Ok(parsed)
    } else {
        Err(parse_error(venue, field, value))
    }
}

fn event_id_value(venue: &str, field: &str, value: &Value) -> ExchangeResult<String> {
    let id = match value {
        Value::String(raw) => raw.trim().to_owned(),
        Value::Number(number) => number.to_string(),
        _ => return Err(parse_error(venue, field, value)),
    };
    if id.is_empty() {
        Err(parse_error(venue, field, value))
    } else {
        Ok(id)
    }
}

fn non_empty_continuation(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
}

fn kucoin_continuation(page: &KucoinFundingHistoryPage) -> ExchangeResult<Option<String>> {
    if !page.has_more {
        return Ok(None);
    }
    let row = page
        .data_list
        .last()
        .ok_or_else(|| pagination_error(KUCOIN, "zero-row continuation"))?;
    let offset = event_id_value(KUCOIN, "offset", &row.id)?;
    let valid = offset.parse::<u64>().is_ok_and(|value| value > 0);
    if valid {
        Ok(Some(offset))
    } else {
        Err(pagination_error(KUCOIN, "invalid offset"))
    }
}

fn pagination_error(venue: &str, reason: &str) -> ExchangeError {
    ExchangeError::Parse(format!("{venue} funding payment pagination {reason}"))
}

fn ms_to_seconds_floor(value: i64) -> i64 {
    value.div_euclid(1_000)
}

pub(super) fn bitget_uta_funding_types() -> &'static [&'static str] {
    BITGET_UTA_FUNDING_TYPES
}

fn is_bitget_uta_funding_type(value: &str) -> bool {
    BITGET_UTA_FUNDING_TYPES
        .iter()
        .any(|candidate| value.eq_ignore_ascii_case(candidate))
}

fn gate_event_id(row: &GateAccountBookRow) -> ExchangeResult<String> {
    if let Ok(id) = event_id_value(GATE, "id", &row.id) {
        return Ok(id);
    }
    if let Ok(id) = event_id_value(GATE, "trade_id", &row.trade_id) {
        return Ok(id);
    }
    require_non_empty(GATE, "text", &row.text)?;
    Ok(format!("{}:{}", row.contract, row.text))
}

fn parse_unix_seconds_ms(venue: &str, field: &str, value: &Value) -> ExchangeResult<i64> {
    let parsed = parse_f64(venue, field, value)?;
    let time_ms = (parsed * 1000.0).round() as i64;
    if time_ms > 0 {
        Ok(time_ms)
    } else {
        Err(parse_error(venue, field, value))
    }
}

fn require_non_empty(venue: &str, field: &str, value: &str) -> ExchangeResult<()> {
    if value.trim().is_empty() {
        Err(ExchangeError::Parse(format!(
            "{venue} funding payment field {field} empty"
        )))
    } else {
        Ok(())
    }
}

fn is_blank_value(value: &Value) -> bool {
    matches!(value, Value::Null) || matches!(value, Value::String(raw) if raw.trim().is_empty())
}

fn parse_error(venue: &str, field: &str, value: &Value) -> ExchangeError {
    ExchangeError::Parse(format!(
        "{venue} funding payment field {field} invalid: {value}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::bitget_response::BitgetObjectResponse;
    use crate::adapters::bybit_response::BybitObjectResponse;
    use crate::adapters::kucoin_response::KucoinResponse;
    use crate::adapters::okx_response::OkxResponse;

    #[test]
    fn binance_funding_payment_parses_official_income_fixture() -> ExchangeResult<()> {
        let rows: Vec<BinanceIncomeRow> = serde_json::from_str(include_str!(
            "../../fixtures/binance/usdm_income_funding_fee_btcusdt.json"
        ))
        .map_err(|error| ExchangeError::Parse(error.to_string()))?;
        let payments = parse_binance_funding_payments(rows)?;

        assert_eq!(payments.len(), 1);
        let row = &payments[0];
        assert_eq!(row.venue, BINANCE);
        assert_eq!(row.symbol, "BTC");
        assert_eq!(row.amount, -0.375);
        assert_eq!(row.currency, "USDT");
        assert_eq!(row.funding_time_ms, 1_570_608_000_000);
        assert_eq!(row.venue_event_id, "binance_funding:9689322392");
        Ok(())
    }

    #[test]
    fn binance_funding_payments_request_uses_income_history_funding_fee_filter() {
        assert_eq!(BINANCE_FUNDING_INCOME_PATH, "/fapi/v1/income");

        let params =
            binance_funding_income_params(Some("BTCUSDT"), Some(1_700), Some(1_800), 2_000);
        assert_eq!(param_value(&params, "symbol"), Some("BTCUSDT"));
        assert_eq!(param_value(&params, "incomeType"), Some("FUNDING_FEE"));
        assert_eq!(param_value(&params, "startTime"), Some("1700"));
        assert_eq!(param_value(&params, "endTime"), Some("1800"));
        assert_eq!(param_value(&params, "limit"), Some("1000"));
        assert_eq!(param_value(&params, "recvWindow"), Some("5000"));
    }

    #[test]
    fn okx_funding_payment_parses_official_bills_fixture() -> ExchangeResult<()> {
        let wrap: OkxResponse<OkxBillRow> = serde_json::from_str(include_str!(
            "../../fixtures/okx/account_bills_funding_fee_btc_usdt_swap.json"
        ))
        .map_err(|error| ExchangeError::Parse(error.to_string()))?;
        let payments = parse_okx_funding_payments(wrap.into_data("okx bills")?)?;

        assert_eq!(payments.len(), 1);
        let row = &payments[0];
        assert_eq!(row.venue, OKX);
        assert_eq!(row.symbol, "BTC");
        assert_eq!(row.amount, 0.021933823221);
        assert_eq!(row.currency, "USDT");
        assert_eq!(row.funding_time_ms, 1_695_033_476_167);
        assert_eq!(row.venue_event_id, "okx_funding:623950854533513219");
        Ok(())
    }

    #[test]
    fn okx_funding_payments_request_uses_bills_subtype_filter() {
        assert_eq!(OKX_FUNDING_BILLS_PATH, "/api/v5/account/bills");

        let path = okx_funding_bills_path(
            Some("BTC-USDT-SWAP"),
            Some(1_695_000_000_000),
            Some(1_695_100_000_000),
        );
        assert!(path.starts_with("/api/v5/account/bills?"));
        assert!(path.contains("instType=SWAP"));
        assert!(path.contains("subType=173"));
        assert!(path.contains("instId=BTC-USDT-SWAP"));
        assert!(path.contains("begin=1695000000000"));
        assert!(path.contains("end=1695100000000"));
    }

    #[test]
    fn bybit_funding_payment_parses_official_transaction_log_fixture() -> ExchangeResult<()> {
        let wrap: BybitObjectResponse<BybitTransactionLogPage> = serde_json::from_str(
            include_str!("../../fixtures/bybit/account_transaction_log_funding_fee.json"),
        )
        .map_err(|error| ExchangeError::Parse(error.to_string()))?;
        let page = parse_bybit_funding_payments(wrap.into_result("transaction-log")?)?;
        assert_eq!(page.continuation.as_deref(), Some("21963%3A1%2C14954%3A1"));
        let payments = page.payments;

        assert_eq!(payments.len(), 1);
        let row = &payments[0];
        assert_eq!(row.venue, BYBIT);
        assert_eq!(row.symbol, "XRP");
        assert_eq!(row.amount, -0.003676);
        assert_eq!(row.currency, "USDT");
        assert_eq!(row.funding_time_ms, 1_672_128_000_000);
        assert_eq!(
            row.venue_event_id,
            "bybit_funding:592324_XRPUSDT_161440249321:1672128000000"
        );
        Ok(())
    }

    #[test]
    fn bybit_empty_terminal_page_accepts_a_null_cursor() -> ExchangeResult<()> {
        let wrap: BybitObjectResponse<BybitTransactionLogPage> = serde_json::from_str(
            include_str!("../../fixtures/bybit/account_transaction_log_empty_terminal_page.json"),
        )
        .map_err(|error| ExchangeError::Parse(error.to_string()))?;
        let page = parse_bybit_funding_payments(wrap.into_result("transaction-log")?)?;

        assert!(page.payments.is_empty());
        assert!(page.continuation.is_none());
        assert_eq!(page.source_row_count, 0);
        Ok(())
    }

    #[test]
    fn bybit_funding_payments_request_uses_transaction_log_settlement_filter() {
        assert_eq!(BYBIT_TRANSACTION_LOG_PATH, "/v5/account/transaction-log");

        let query = bybit_transaction_log_query(
            "UNIFIED",
            Some("BTC"),
            Some(1_700_000_000_000),
            Some(1_700_086_400_000),
            Some("21963%3A1%2C14954%3A1"),
        );
        assert!(query.contains("accountType=UNIFIED"));
        assert!(query.contains("category=linear"));
        assert!(query.contains("type=SETTLEMENT"));
        assert!(query.contains("baseCoin=BTC"));
        assert!(query.contains("limit=50"));
        assert!(query.contains("startTime=1700000000000"));
        assert!(query.contains("endTime=1700086400000"));
        assert!(query.contains("cursor=21963%3A1%2C14954%3A1"));
    }

    #[test]
    fn bitget_uta_funding_payment_parses_official_financial_records_fixture() -> ExchangeResult<()>
    {
        let wrap: BitgetObjectResponse<BitgetUtaFinancialRecordsPage> = serde_json::from_str(
            include_str!("../../fixtures/bitget/uta_financial_records_funding_fee_btcusdt.json"),
        )
        .map_err(|error| ExchangeError::Parse(error.to_string()))?;
        let page = parse_bitget_funding_payments(wrap.into_result("financial records")?)?;
        assert_eq!(page.continuation.as_deref(), Some("next-cursor"));
        let payments = page.payments;

        assert_eq!(payments.len(), 2);
        let row = &payments[0];
        assert_eq!(row.venue, BITGET);
        assert_eq!(row.symbol, "BTC");
        assert_eq!(row.amount, 0.125);
        assert_eq!(row.currency, "USDT");
        assert_eq!(row.funding_time_ms, 1_700_006_400_000);
        assert_eq!(row.venue_event_id, "bitget_funding:1234567890");
        assert_eq!(payments[1].amount, -0.03125);
        assert_eq!(payments[1].venue_event_id, "bitget_funding:1234567891");
        Ok(())
    }

    #[test]
    fn bitget_uta_funding_payments_request_uses_financial_records_type_filter() {
        assert_eq!(
            BITGET_UTA_FINANCIAL_RECORDS_PATH,
            "/api/v3/account/financial-records"
        );

        let path = bitget_uta_financial_records_path(
            "CONTRACT_MAIN_SETTLE_FEE_USER_OUT",
            Some(1_700_000_000_000),
            Some(1_700_086_400_000),
            Some("1234567890"),
        );

        assert!(path.starts_with("/api/v3/account/financial-records?"));
        assert!(path.contains("category=USDT-FUTURES"));
        assert!(path.contains("type=CONTRACT_MAIN_SETTLE_FEE_USER_OUT"));
        assert!(path.contains("startTime=1700000000000"));
        assert!(path.contains("endTime=1700086400000"));
        assert!(path.contains("limit=100"));
        assert!(path.contains("cursor=1234567890"));
    }

    #[test]
    fn gate_funding_payment_parses_official_account_book_fixture() -> ExchangeResult<()> {
        let rows: Vec<GateAccountBookRow> = serde_json::from_str(include_str!(
            "../../fixtures/gate/futures_usdt_account_book_fund_btc_usdt.json"
        ))
        .map_err(|error| ExchangeError::Parse(error.to_string()))?;
        let payments = parse_gate_funding_payments(&rows, "USDT")?;

        assert_eq!(payments.len(), 1);
        let row = &payments[0];
        assert_eq!(row.venue, GATE);
        assert_eq!(row.symbol, "BTC");
        assert_eq!(row.amount, -0.03125);
        assert_eq!(row.currency, "USDT");
        assert_eq!(row.funding_time_ms, 1_700_006_400_123);
        assert_eq!(row.venue_event_id, "gate_funding:9000001");
        Ok(())
    }

    #[test]
    fn gate_funding_payments_request_uses_account_book_fund_filter() {
        assert_eq!(GATE_ACCOUNT_BOOK_PATH, "/api/v4/futures/usdt/account_book");

        let query = gate_account_book_query(
            Some("BTC_USDT"),
            Some(1_700_000_000_123),
            Some(1_700_086_400_999),
        );

        assert!(query.contains("type=fund"));
        assert!(query.contains("contract=BTC_USDT"));
        assert!(query.contains("from=1700000000"));
        assert!(query.contains("to=1700086400"));
        assert!(query.contains("limit=100"));
    }

    #[test]
    fn kucoin_funding_payment_parses_official_history_fixture() -> ExchangeResult<()> {
        let wrap: KucoinResponse<KucoinFundingHistoryPage> = serde_json::from_str(include_str!(
            "../../fixtures/kucoin/funding_history_xbtusdtm.json"
        ))
        .map_err(|error| ExchangeError::Parse(error.to_string()))?;
        let page = parse_kucoin_funding_payments(wrap.into_data("funding-history")?)?;
        assert_eq!(page.continuation.as_deref(), Some("1472387374042586"));
        let payments = page.payments;

        assert_eq!(payments.len(), 1);
        let row = &payments[0];
        assert_eq!(row.venue, KUCOIN);
        assert_eq!(row.symbol, "BTC");
        assert_eq!(row.amount, -0.05585669);
        assert_eq!(row.currency, "USDT");
        assert_eq!(row.funding_time_ms, 1_731_470_400_000);
        assert_eq!(row.venue_event_id, "kucoin_funding:1472387374042586");
        Ok(())
    }

    #[test]
    fn kucoin_funding_payments_request_uses_futures_history_path() {
        assert_eq!(KUCOIN_FUNDING_HISTORY_PATH, "/api/v1/funding-history");

        let path = kucoin_funding_history_path(
            Some("XBTUSDTM"),
            Some(1_700_310_700_000),
            Some(1_702_310_700_000),
            Some("1472387374042586"),
        );
        assert!(path.starts_with("/api/v1/funding-history?"));
        assert!(path.contains("symbol=XBTUSDTM"));
        assert!(path.contains("startAt=1700310700000"));
        assert!(path.contains("endAt=1702310700000"));
        assert!(path.contains("maxCount=100"));
        assert!(path.contains("offset=1472387374042586"));
    }

    #[test]
    fn pagination_deduplicates_cross_page_event_ids_in_order() -> ExchangeResult<()> {
        let mut pagination = FundingPaymentPagination::new(BYBIT);
        let mut payments = Vec::new();
        let mut event_ids = HashSet::new();

        let next = pagination.accept(
            test_page(&["a", "b"], Some("cursor-1"), 2),
            &mut payments,
            &mut event_ids,
        )?;
        assert_eq!(next.as_deref(), Some("cursor-1"));
        let next = pagination.accept(
            test_page(&["b", "c"], None, 2),
            &mut payments,
            &mut event_ids,
        )?;

        assert!(next.is_none());
        assert_eq!(
            payments
                .iter()
                .map(|row| row.venue_event_id.as_str())
                .collect::<Vec<_>>(),
            ["a", "b", "c"]
        );
        Ok(())
    }

    #[test]
    fn pagination_rejects_repeated_and_zero_row_continuations() -> ExchangeResult<()> {
        let mut pagination = FundingPaymentPagination::new(BITGET);
        let mut payments = Vec::new();
        let mut event_ids = HashSet::new();
        pagination.accept(
            test_page(&["a"], Some("same"), 1),
            &mut payments,
            &mut event_ids,
        )?;

        let repeated = pagination
            .accept(
                test_page(&["b"], Some("same"), 1),
                &mut payments,
                &mut event_ids,
            )
            .expect_err("a repeated token must fail closed");
        assert!(repeated.to_string().contains("repeated continuation"));

        let mut pagination = FundingPaymentPagination::new(BYBIT);
        let empty = pagination
            .accept(
                test_page(&[], Some("cursor"), 0),
                &mut Vec::new(),
                &mut HashSet::new(),
            )
            .expect_err("continuation without source rows must fail closed");
        assert!(empty.to_string().contains("zero-row continuation"));
        Ok(())
    }

    #[test]
    fn pagination_rejects_a_continuation_after_page_cap() -> ExchangeResult<()> {
        let mut pagination = FundingPaymentPagination::new(BYBIT);
        let mut payments = Vec::new();
        let mut event_ids = HashSet::new();
        for page in 1..MAX_FUNDING_PAYMENT_PAGES {
            pagination.accept(
                test_page(&[], Some(&format!("cursor-{page}")), 1),
                &mut payments,
                &mut event_ids,
            )?;
        }

        let capped = pagination
            .accept(
                test_page(&[], Some("cursor-100"), 1),
                &mut payments,
                &mut event_ids,
            )
            .expect_err("page 100 with another continuation must fail closed");
        assert!(capped.to_string().contains("100-page cap reached"));
        Ok(())
    }

    #[test]
    fn kucoin_continuation_rejects_invalid_offset() {
        let page: KucoinFundingHistoryPage = serde_json::from_value(serde_json::json!({
            "dataList": [{"id": "not-an-offset"}],
            "hasMore": true
        }))
        .expect("page fixture");

        let error = parse_kucoin_funding_payments(page).expect_err("offset must be positive u64");
        assert!(error.to_string().contains("invalid offset"));
    }

    fn test_page(
        event_ids: &[&str],
        continuation: Option<&str>,
        source_row_count: usize,
    ) -> FundingPaymentPage {
        FundingPaymentPage {
            payments: event_ids
                .iter()
                .map(|event_id| {
                    payment(
                        BYBIT,
                        "BTC".into(),
                        1.0,
                        "USDT".into(),
                        1,
                        (*event_id).into(),
                    )
                })
                .collect(),
            continuation: continuation.map(str::to_owned),
            source_row_count,
        }
    }

    fn param_value<'a>(params: &'a [(&'static str, String)], key: &str) -> Option<&'a str> {
        params
            .iter()
            .find(|(candidate, _)| *candidate == key)
            .map(|(_, value)| value.as_str())
    }
}
