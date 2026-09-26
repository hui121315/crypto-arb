use super::*;
use rust_decimal::Decimal;
use std::collections::BTreeMap;
mod sizing;
pub use sizing::*;

pub const STOCK_CONVERSION_SYMBOL: &str = "USDT_USDC";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockExchangeConversionRequest {
    pub request_id: String,
    pub input_usdt: String,
    pub minimum_usdc: String,
}

impl StockExchangeConversionRequest {
    pub fn amounts(&self) -> Result<(Decimal, Decimal), String> {
        if !(16..=128).contains(&self.request_id.len())
            || !self
                .request_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        {
            return Err("兑换请求编号无效".into());
        }
        let input = amount(&self.input_usdt)?;
        let minimum = amount(&self.minimum_usdc)?;
        if input <= Decimal::ZERO
            || minimum <= Decimal::ZERO
            || input > Decimal::from(1_000_000)
            || minimum > Decimal::from(1_000_000)
            || input.scale() > 6
            || minimum.scale() > 6
        {
            return Err("兑换金额必须为正，最多 6 位小数且不超过 100 万".into());
        }
        Ok((input, minimum))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockConversionMarket {
    pub symbol: String,
    pub base_symbol: String,
    pub quote_symbol: String,
    pub market_type: String,
    pub order_book_state: String,
    pub min_quantity: String,
    pub max_quantity: Option<String>,
    pub step_size: String,
    pub tick_size: String,
    pub checked_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockExchangeConversionTerms {
    pub account_fingerprint: String,
    pub market: StockConversionMarket,
    pub book: StockBookQuote,
    pub available_usdt: String,
    pub balance_at_ms: i64,
    pub taker_fee_bps: String,
    pub fees_at_ms: i64,
    pub fee_budget_usdc: String,
    pub minimum_net_usdc: String,
    pub instruction: StockCexInstruction,
    pub created_at_ms: i64,
    pub valid_until_ms: i64,
}

pub fn exchange_conversion_amounts(
    r: &StockExchangeConversionRequest,
    m: &StockConversionMarket,
    b: &StockBookQuote,
    fee_bps: &str,
) -> Result<(Decimal, Decimal), String> {
    let (input, minimum) = r.amounts()?;
    if m.symbol != STOCK_CONVERSION_SYMBOL
        || m.base_symbol != "USDT"
        || m.quote_symbol != "USDC"
        || m.market_type != "SPOT"
        || m.order_book_state != "Open"
        || b.symbol != m.symbol
    {
        return Err("USDT/USDC 官方现货市场未开放或身份不符".into());
    }
    let step = amount(&m.step_size)?;
    let tick = amount(&m.tick_size)?;
    let min = amount(&m.min_quantity)?;
    let price = amount(b.bid.as_deref().ok_or("USDT/USDC 缺少 WS 买价")?)?;
    let depth = amount(b.bid_quantity.as_deref().ok_or("USDT/USDC 缺少 WS 买量")?)?;
    let fee = amount(fee_bps)?;
    if step <= Decimal::ZERO
        || tick <= Decimal::ZERO
        || min <= Decimal::ZERO
        || price <= Decimal::ZERO
        || input < min
        || depth < input
        || input.checked_rem(step) != Some(Decimal::ZERO)
        || price.checked_rem(tick) != Some(Decimal::ZERO)
        || m.max_quantity
            .as_deref()
            .map(amount)
            .transpose()?
            .is_some_and(|max| input > max)
        || fee < Decimal::ZERO
        || fee >= Decimal::from(10_000)
    {
        return Err("兑换数量、限价、盘口量或费用不满足官方规格".into());
    }
    let gross = input.checked_mul(price).ok_or("兑换金额溢出")?;
    let fees = gross
        .checked_mul(fee)
        .and_then(|v| v.checked_div(Decimal::from(10_000)))
        .ok_or("兑换费用溢出")?;
    let net = gross.checked_sub(fees).ok_or("兑换净到账溢出")?;
    if net < minimum {
        return Err(format!(
            "按当前买价和手续费，最低仅 {net} USDC，低于要求 {minimum} USDC"
        ));
    }
    Ok((fees, net))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockExchangeConversionPlan {
    pub plan_id: String,
    pub request: StockExchangeConversionRequest,
    pub terms: StockExchangeConversionTerms,
    pub revision: u64,
    pub updated_at_ms: i64,
    pub cancelled_at_ms: Option<i64>,
    pub order: Option<StockCexOrder>,
}

impl StockExchangeConversionPlan {
    pub fn can_submit(&self, now: i64) -> bool {
        self.order.is_none()
            && self.cancelled_at_ms.is_none()
            && now >= self.terms.created_at_ms
            && now < self.terms.valid_until_ms
    }
    pub fn holds_funds(&self, now: i64) -> bool {
        if self.order.is_some() {
            self.accounting().is_err()
        } else {
            self.can_submit(now)
        }
    }
    pub fn accounting(&self) -> Result<BTreeMap<String, String>, String> {
        let order = self.order.as_ref().ok_or("尚未提交兑换")?;
        let changes = order
            .net_asset_changes(&self.terms.instruction)
            .ok_or("原订单终态、成交或费用尚未完整")?;
        let usdt = amount(changes.get("USDT").ok_or("缺少 USDT 收支")?)?;
        let usdc = amount(changes.get("USDC").ok_or("缺少 USDC 收支")?)?;
        if changes.iter().any(|(a, n)| {
            !matches!(a.as_str(), "USDT" | "USDC") && amount(n).is_ok_and(|v| v != Decimal::ZERO)
        }) {
            return Err("兑换发生其他币种费用，需核对后再释放占用".into());
        }
        if order.phase == StockCexOrderPhase::Filled {
            let (input, minimum) = self.request.amounts()?;
            if usdt != -input || usdc < minimum {
                return Err("实际投入或净到账超出原计划，保留占用".into());
            }
            let fees = order.fills.iter().try_fold(Decimal::ZERO, |n, f| {
                let fee = f.fee.as_ref().ok_or("费用缺失")?;
                if fee.asset != "USDC" && amount(&fee.quantity)? != Decimal::ZERO {
                    return Err("实际费用不是 USDC".into());
                }
                n.checked_add(amount(&fee.quantity)?)
                    .ok_or_else(|| "费用溢出".to_owned())
            })?;
            // Price improvement may increase absolute fees; compare the actual rate, not the old notional.
            let (_, gross) = order.fill_totals().ok_or("成交金额缺失")?;
            let rate_budget = gross
                .checked_mul(amount(&self.terms.taker_fee_bps)?)
                .and_then(|v| v.checked_div(Decimal::from(10_000)))
                .ok_or("费用预算溢出")?;
            if fees > rate_budget {
                return Err("实际手续费高于已确认费率，保留占用".into());
            }
        } else if usdt != Decimal::ZERO || usdc != Decimal::ZERO {
            return Err("FOK 未全部成交但存在实际收支，保留占用".into());
        }
        Ok(changes)
    }
}

fn amount(value: &str) -> Result<Decimal, String> {
    Decimal::from_str_exact(value).map_err(|_| "兑换金额格式无效".into())
}
