use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockExchangeConversionSizingRequest {
    pub minimum_usdc: String,
}

impl StockExchangeConversionSizingRequest {
    pub fn minimum(&self) -> Result<Decimal, String> {
        let r = StockExchangeConversionRequest {
            request_id: "stock-conversion-read-only".into(),
            input_usdt: "1".into(),
            minimum_usdc: self.minimum_usdc.clone(),
        };
        r.amounts().map(|(_, minimum)| minimum)
    }

    pub fn size(
        &self,
        market: &StockConversionMarket,
        book: &StockBookQuote,
        fee_bps: &str,
    ) -> Result<String, String> {
        let target = self.minimum()?;
        let step = amount(&market.step_size)?;
        let min = amount(&market.min_quantity)?;
        let bid = amount(book.bid.as_deref().ok_or("USDT/USDC 缺少 WS 买价")?)?;
        let fee = amount(fee_bps)?;
        if step <= Decimal::ZERO
            || step.normalize().scale() > 6
            || min <= Decimal::ZERO
            || bid <= Decimal::ZERO
            || fee < Decimal::ZERO
            || fee >= Decimal::from(10_000)
        {
            return Err("兑换步长、买价或账户费率无法用于反算".into());
        }
        let rate = fee
            .checked_div(Decimal::from(10_000))
            .ok_or("兑换费率溢出")?;
        let unit_net = bid
            .checked_mul(Decimal::ONE.checked_sub(rate).ok_or("兑换费率无效")?)
            .filter(|n| *n > Decimal::ZERO)
            .ok_or("兑换净价无法计算")?;
        let raw = target.checked_div(unit_net).ok_or("兑换投入溢出")?.max(min);
        let mut input = raw
            .checked_div(step)
            .map(|n| n.ceil())
            .and_then(|n| n.checked_mul(step))
            .ok_or("兑换数量对齐失败")?;
        // Check the forward calculation too: Decimal division may round at its precision limit.
        let gross = input.checked_mul(bid).ok_or("兑换金额溢出")?;
        let net = gross
            .checked_mul(rate)
            .and_then(|fees| gross.checked_sub(fees))
            .ok_or("兑换净到账溢出")?;
        if net < target {
            input = input.checked_add(step).ok_or("兑换数量溢出")?;
        }
        let r = StockExchangeConversionRequest {
            request_id: "stock-conversion-read-only".into(),
            input_usdt: input.normalize().to_string(),
            minimum_usdc: self.minimum_usdc.clone(),
        };
        exchange_conversion_amounts(&r, market, book, fee_bps)?;
        Ok(r.input_usdt)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockExchangeConversionSizing {
    pub request: StockExchangeConversionSizingRequest,
    pub input_usdt: String,
    pub minimum_net_usdc: String,
    pub fee_budget_usdc: String,
    pub available_usdt: String,
    pub bid_usdc: String,
    pub step_size: String,
    pub checked_at_ms: i64,
    pub valid_until_ms: i64,
}
