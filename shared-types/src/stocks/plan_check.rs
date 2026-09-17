use super::*;
use rust_decimal::Decimal;

fn decimal(s: &str) -> Option<Decimal> {
    Decimal::from_str_exact(s).ok()
}

fn fresh(at: i64, now: i64, age: i64) -> bool {
    at > 0 && now >= at && now.saturating_sub(at) <= age
}

impl StockExecutionPlan {
    pub fn validate_preflight_evidence(&self) -> Result<(), String> {
        let t = &self.terms;
        let p = t
            .preflight_evidence
            .as_ref()
            .ok_or("旧计划缺少原预检记录，请取消并重新构建")?;
        let cost = &t.chain_cost;
        let q = &cost.quote;
        let row = p
            .directions
            .iter()
            .find(|r| r.direction == self.request.direction.label())
            .ok_or("原计划缺少该方向预检")?;
        let (quote_terms, quote_at) = match self.request.direction {
            StockChainDirection::Buy => {
                (&p.price_basis.buy_terms, p.price_basis.buy_requested_at_ms)
            }
            StockChainDirection::Sell => (
                &p.price_basis.sell_terms,
                p.price_basis.sell_requested_at_ms,
            ),
        };
        let last = t.market_valid_until_ms.saturating_sub(1);
        if p.asset != self.request.asset
            || p.wallet_address.as_deref() != Some(&self.request.wallet_address)
            || p.checked_at_ms != self.request.preflight_at_ms
            || p.checked_at_ms > t.created_at_ms
            || !p.problems.is_empty()
            || t.market_valid_until_ms > p.valid_until_ms
            || p.account_at_ms
                .is_none_or(|at| !fresh(at, t.created_at_ms, 30_000))
            || p.wallet_at_ms.is_none_or(|at| !fresh(at, last, 30_000))
            || p.price_basis.asset.as_deref() != Some(&self.request.asset)
            || p.price_basis.mint_units.as_ref()
                != Some(&(
                    cost.mint.address.clone(),
                    cost.mint.decimals,
                    cost.mint.ui_multiplier.clone(),
                ))
            || quote_terms.as_ref()
                != Some(&(
                    q.input_mint.clone(),
                    q.output_mint.clone(),
                    q.input_raw.clone(),
                    q.minimum_output_raw.clone(),
                ))
            || quote_at != Some(q.requested_at_ms)
            || !comparison::quote_current(q, last)
            || t.market_valid_until_ms > cost.valid_until_ms
            || t.market_valid_until_ms > t.route.valid_until_ms
            || !fresh(cost.mint.checked_at_ms, last, 60_000)
            || cost.mint.next_change_at_ms.is_some_and(|at| last >= at)
            || row.after_known_costs_usdc.as_deref() != Some(&t.after_known_costs_usdc)
            || row.cex_fee_usdc.as_deref()
                != t.cex_fee_budget.as_ref().map(|f| f.additional_fee.as_str())
            || row.inventory.len() != t.allocations.len()
            || row.inventory.iter().zip(&t.allocations).any(|(i, a)| {
                i.sufficient != Some(true)
                    || i.location != a.location
                    || i.asset != a.asset
                    || i.required.as_deref() != Some(&a.quantity)
                    || i.available.as_deref() != Some(&a.available_at_reservation)
            })
        {
            return Err("原预检与计划的金额、库存或有效期不一致".into());
        }
        Ok(())
    }

    // Observation quotes may change; an accepted plan always keeps its original payload and deadline.
    pub fn submission_market_check(&self, s: &StockMarketSnapshot, now: i64) -> Result<(), String> {
        let t = &self.terms;
        if self.phase_at(now) != StockPlanPhase::Reserved
            || self.cex_order.is_some()
            || self.rfq_acceptance.is_some()
            || self.chain_submission.is_some()
        {
            return Err("计划已提交、取消或不再预留".into());
        }
        if now < t.created_at_ms || now >= t.market_valid_until_ms {
            return Err("原报价已过期，请重新构建".into());
        }
        self.validate_preflight_evidence()?;
        let security = s
            .security
            .as_ref()
            .filter(|v| v.asset == t.security.asset && v.cusip == t.security.cusip)
            .ok_or("原证券身份已变化，请重新构建")?;
        let cost = &t.chain_cost;
        let c = s
            .comparison
            .as_ref()
            .filter(|c| c.asset == self.request.asset)
            .ok_or("原股票的链上身份暂不可核验")?;
        if !cost.simulation_passed
            || !cost.problems.is_empty()
            || cost.complete_native_usdc_budget(now).is_none()
            || !comparison::quote_current(&cost.quote, now)
            || !fresh(c.mint.checked_at_ms, now, 60_000)
            || c.mint.checked_at_ms < cost.mint.checked_at_ms
            || c.mint.slot < cost.mint.slot
            || c.mint.chain_time_ms < cost.mint.chain_time_ms
            || c.mint.address != cost.mint.address
            || c.mint.decimals != cost.mint.decimals
            || decimal(&c.mint.ui_multiplier) != decimal(&cost.mint.ui_multiplier)
            || c.mint.extensions != cost.mint.extensions
            || c.mint.next_change_at_ms != cost.mint.next_change_at_ms
            || c.mint.next_change_at_ms.is_some_and(|at| now >= at)
        {
            return Err("原链上报价、费用或股票份额已失效".into());
        }
        let tokens = s
            .tokens
            .iter()
            .filter(|t| t.blockchain == "Solana")
            .collect::<Vec<_>>();
        if s.token_metadata_problem.is_some()
            || tokens.len() != 1
            || tokens[0].contract_address.as_deref() != Some(&cost.mint.address)
            || tokens[0].native_decimals != Some(cost.mint.decimals)
        {
            return Err("官方股票合约映射不可确认".into());
        }
        let route = s
            .trading_route
            .as_ref()
            .filter(|r| {
                r.valid_until_ms > now
                    && r.kind == t.route.kind
                    && r.symbol == t.route.symbol
                    && r.session == t.route.session
            })
            .ok_or("原股票交易通道或时段已变化")?;
        match t.cex_instruction.as_ref().ok_or("原计划缺少交易指令")? {
            StockCexInstruction::OrderBook {
                symbol,
                side,
                quantity,
                limit_price,
                ..
            } => {
                if route.kind != StockRouteKind::OrderBook || !s.connected || s.problem.is_some() {
                    return Err("股票盘口 WS 未就绪，等待恢复".into());
                }
                let market = security
                    .order_books
                    .iter()
                    .find(|m| m.symbol == *symbol)
                    .ok_or("原股票订单簿不可用")?;
                if Some(market) != t.security.order_books.iter().find(|m| m.symbol == *symbol)
                    || market.state != "Open"
                {
                    return Err("股票订单规格或开放状态已变化".into());
                }
                let book = s
                    .books
                    .iter()
                    .find(|b| b.symbol == *symbol)
                    .ok_or("原股票盘口缺失")?;
                if !fresh(book.source_at_ms, now, 3_000) || !fresh(book.received_at_ms, now, 3_000)
                {
                    return Err("股票盘口已陈旧，等待 WS 更新".into());
                }
                let (price, depth) = match side {
                    StockRfqSide::Ask => (&book.bid, &book.bid_quantity),
                    StockRfqSide::Bid => (&book.ask, &book.ask_quantity),
                };
                let price = price
                    .as_deref()
                    .and_then(decimal)
                    .filter(|p| *p > Decimal::ZERO)
                    .ok_or("对应一档价格缺失")?;
                let limit = decimal(limit_price).ok_or("原限价无效")?;
                if match side {
                    StockRfqSide::Ask => price < limit,
                    StockRfqSide::Bid => price > limit,
                } {
                    return Err("当前价格已差于原限价，请重新构建".into());
                }
                if depth
                    .as_deref()
                    .and_then(decimal)
                    .zip(decimal(quantity))
                    .is_none_or(|(d, q)| q <= Decimal::ZERO || d < q)
                {
                    return Err("当前一档数量不足以覆盖原计划".into());
                }
            }
            StockCexInstruction::AcceptRfq {
                symbol,
                side,
                quantity,
                ..
            } => {
                if route.kind != StockRouteKind::Rfq || s.rfq_problem.is_some() {
                    return Err("股票 RFQ 状态不可确认".into());
                }
                let bound = t.rfq.as_ref().ok_or("原 RFQ 身份缺失")?;
                let r = s
                    .rfqs
                    .iter()
                    .find(|r| r.request.request_id == bound.request_id)
                    .ok_or("原 RFQ 已不可用")?;
                if r.account_fingerprint != t.account_fingerprint
                    || r.request.asset != self.request.asset
                    || r.symbol != *symbol
                    || r.request.side != *side
                    || decimal(&r.request.quantity) != decimal(quantity)
                    || r.rfq_id.as_deref() != Some(&bound.rfq_id)
                    || r.expiry_time_ms != Some(bound.expiry_time_ms)
                    || r.current_candidate(s.rfq_connected, now) != Some(&bound.candidate)
                {
                    return Err("原 RFQ 报价或身份已变化，请重新构建".into());
                }
            }
        }
        Ok(())
    }
}
