use super::*;
use rust_decimal::Decimal;
use shared_types::stocks::comparison::positive;

pub(super) fn compile(
    request: &StockPlanRequest,
    terms: &StockPlanTerms,
) -> Result<StockCexInstruction, String> {
    let quantity = positive(&terms.cex_shares).ok_or("交易所股数无效")?;
    let notional = positive(&terms.cex_notional_usdc).ok_or("交易所金额无效")?;
    let price = notional
        .checked_div(quantity)
        .filter(|p| *p > Decimal::ZERO)
        .ok_or("交易所价格无效")?;
    if price.checked_mul(quantity) != Some(notional) || request.asset != terms.security.asset {
        return Err("交易所价格、股数与计划证券不一致".into());
    }
    let side = if request.direction == StockChainDirection::Buy {
        StockRfqSide::Ask
    } else {
        StockRfqSide::Bid
    };
    let symbol = terms
        .route
        .symbol
        .as_ref()
        .ok_or("缺少官方市场编号")?
        .clone();
    match terms.route.kind {
        StockRouteKind::OrderBook => {
            let market = terms
                .security
                .order_books
                .iter()
                .find(|m| m.symbol == symbol && m.quote == "USDC" && m.state == "Open")
                .ok_or("股票订单簿未开放或规格未核实")?;
            aligned(quantity, &market.min_quantity, &market.step_size)?;
            let tick = positive(&market.tick_size).ok_or("股票价格步长无效")?;
            if price.checked_rem(tick).is_none_or(|r| !r.is_zero()) || terms.rfq.is_some() {
                return Err("限价不满足官方价格步长或混入 RFQ 参数".into());
            }
            let bytes = serde_json::to_vec(&(request, &terms.account_fingerprint))
                .map_err(|_| "订单标识编码失败")?;
            let hash = common::signing::hmac_sha256_hex(b"stock-order-client-v1", &bytes);
            let client_id = u32::from_str_radix(&hash[..8], 16)
                .map_err(|_| "订单标识生成失败")?
                .max(1);
            Ok(StockCexInstruction::OrderBook {
                client_id,
                symbol,
                side,
                quantity: quantity.normalize().to_string(),
                limit_price: price.normalize().to_string(),
            })
        }
        StockRouteKind::Rfq => {
            let rfq = terms.rfq.as_ref().ok_or("缺少原始 RFQ 报价")?;
            let session = terms
                .route
                .session
                .as_ref()
                .ok_or("缺少股票 RFQ 时段规格")?;
            aligned(quantity, &session.min_quantity, &session.step_size)?;
            if symbol != terms.security.rfq_symbol
                || positive(&rfq.candidate.taker_price) != Some(price)
                || rfq.expiry_time_ms < terms.market_valid_until_ms
                || rfq.candidate.received_at_ms > terms.created_at_ms
                || [&rfq.rfq_id, &rfq.candidate.quote_id]
                    .iter()
                    .any(|id| !id.parse::<u64>().is_ok_and(|id| id > 0))
                || session
                    .max_quantity
                    .as_deref()
                    .is_some_and(|m| positive(m).is_none_or(|m| quantity > m))
            {
                return Err("RFQ 市场、股数、报价或有效期与计划不一致".into());
            }
            Ok(StockCexInstruction::AcceptRfq {
                rfq_id: rfq.rfq_id.clone(),
                quote_id: rfq.candidate.quote_id.clone(),
                symbol,
                side,
                quantity: quantity.normalize().to_string(),
                taker_price: price.normalize().to_string(),
            })
        }
        _ => Err("该股票时段没有可编译的交易请求".into()),
    }
}

fn aligned(q: Decimal, min: &str, step: &str) -> Result<(), String> {
    let min = positive(min).ok_or("股票最小股数无效")?;
    let step = positive(step).ok_or("股票数量步长无效")?;
    if q < min || q.checked_rem(step).is_none_or(|r| !r.is_zero()) {
        return Err("股数不满足官方最小数量或步长".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
