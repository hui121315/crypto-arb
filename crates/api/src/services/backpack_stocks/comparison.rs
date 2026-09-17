use super::*;
use crate::services::onchain_comparison::stock_quotes;
use rust_decimal::{prelude::ToPrimitive, Decimal};
use shared_types::stocks::comparison::{positive, shares, SOLANA_USDC};

pub(super) fn issuer(
    snapshot: &StockMarketSnapshot,
) -> Result<(&'static str, &'static str, u8), String> {
    let profile = shared_types::stocks::identity::backpack_token_identity(snapshot)?;
    Ok((profile.solana_mint, profile.redemption_source, profile.decimals))
}

pub(super) fn budget_raw(request: &StockQuoteRequest) -> Result<String, String> {
    let budget = positive(&request.budget_usdc)
        .filter(|n| *n <= Decimal::from(100_000) && n.scale() <= 6)
        .ok_or("询价金额须大于 0、不超过 100000 USDC，最多 6 位小数")?;
    budget
        .checked_mul(Decimal::from(1_000_000))
        .and_then(|n| n.to_u64())
        .filter(|n| *n > 0)
        .map(|n| n.to_string())
        .ok_or_else(|| "USDC 原始金额无效".into())
}

fn sell_raw(
    snapshot: &StockMarketSnapshot,
    mint: &StockMintEvidence,
    buy: &StockDexQuote,
) -> Result<String, SellSizeError> {
    let route = snapshot
        .trading_route
        .as_ref()
        .ok_or("官方交易通道尚未确认")?;
    if route.valid_until_ms <= common::time::now_ms() {
        return Err("官方交易通道已过期，等待重新核实".into());
    }
    let (step, minimum, maximum) = match route.kind {
        StockRouteKind::Rfq => {
            let session = route.session.as_ref().ok_or("RFQ 时段的数量约束缺失")?;
            let maximum = session
                .max_quantity
                .as_deref()
                .map(|value| positive(value).ok_or("RFQ 最大股数无效"))
                .transpose()?;
            (
                positive(&session.step_size),
                positive(&session.min_quantity),
                maximum,
            )
        }
        StockRouteKind::OrderBook => {
            let market = snapshot
                .security
                .as_ref()
                .and_then(|s| {
                    s.order_books
                        .iter()
                        .find(|m| Some(&m.symbol) == route.symbol.as_ref() && m.quote == "USDC")
                })
                .ok_or("该证券没有匹配的 USDC 股票订单簿")?;
            (
                positive(&market.step_size),
                positive(&market.min_quantity),
                None,
            )
        }
        _ => return Err(route.reason.clone().into()),
    };
    let step = step.ok_or("股票下单步长无效")?;
    let minimum = minimum.ok_or("最小股票下单数量无效")?;
    let amount = shares(&buy.minimum_output_raw, mint).ok_or("链上到账股数无法换算")?;
    let quantity = amount
        .checked_div(step)
        .map(|n| n.floor())
        .and_then(|n| n.checked_mul(step))
        .ok_or("股票数量对齐计算无效")?;
    if quantity < minimum || maximum.is_some_and(|maximum| quantity > maximum) {
        let problem = if quantity < minimum {
            format!("本次最低到账 {} 股，按 {} 股步长对齐后不足当前时段最小股数 {}；请调整金额或等待时段变化", amount.normalize(), step.normalize(), minimum.normalize())
        } else {
            format!(
                "本次对齐后 {} 股，超过当前时段最大股数 {}；请降低询价金额",
                quantity.normalize(),
                maximum.unwrap().normalize()
            )
        };
        return Err(SellSizeError::Quantity(
            problem,
            StockQuoteQuantityLimit {
                quoted_shares: amount.normalize().to_string(),
                min_quantity: minimum.normalize().to_string(),
                max_quantity: maximum.map(|n| n.normalize().to_string()),
                step_size: step.normalize().to_string(),
            },
        ));
    }
    quantity
        .checked_div(positive(&mint.ui_multiplier).ok_or("股数倍率无效")?)
        .and_then(|n| n.checked_mul(Decimal::from(10_u64.pow(u32::from(mint.decimals)))))
        .and_then(|n| n.floor().to_u64())
        .filter(|n| *n > 0)
        .map(|n| n.to_string())
        .ok_or_else(|| "股票卖出原始数量无法换算".into())
}

#[derive(Debug)]
enum SellSizeError {
    Quantity(String, StockQuoteQuantityLimit),
    Unavailable(String),
}

impl From<&str> for SellSizeError {
    fn from(message: &str) -> Self {
        Self::Unavailable(message.into())
    }
}

impl From<String> for SellSizeError {
    fn from(message: String) -> Self {
        Self::Unavailable(message)
    }
}

impl std::fmt::Display for SellSizeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Quantity(message, _) | Self::Unavailable(message) => f.write_str(message),
        }
    }
}

impl BackpackStocks {
    pub(crate) async fn compare(
        &self,
        request: StockQuoteRequest,
        hub: realtime::WsHub,
    ) -> Result<StockMarketSnapshot, String> {
        let _guard = self
            .quote_lock
            .try_lock()
            .map_err(|_| "股票正在询价，请等待当前请求完成")?;
        let raw = budget_raw(&request)?;
        let generation = self.generation.load(Ordering::SeqCst);
        self.ensure_generation(generation, &request.asset)?;
        self.refresh_context(generation).await?;
        let snapshot = self.snapshot();
        if snapshot
            .security
            .as_ref()
            .is_none_or(|s| s.asset != request.asset)
        {
            return Err("所选股票已改变，请重新询价".into());
        }
        let now = common::time::now_ms();
        if snapshot
            .token_metadata_at_ms
            .is_none_or(|t| now < t || now.saturating_sub(t) > CATALOG_TTL_MS)
        {
            return Err("官方合约目录需更新，请重新选择该证券后询价".into());
        }
        let (address, docs, decimals) = issuer(&snapshot)?;
        if snapshot.comparison.as_ref().is_some_and(|c| {
            c.budget_usdc == request.budget_usdc
                && c.keyed == request.keyed
                && now.saturating_sub(c.buy.requested_at_ms)
                    < stock_quotes::interval_ms(request.keyed)
        }) {
            return Err("当前询价仍新鲜；请勿重复请求".into());
        }
        let cached_mint = snapshot
            .comparison
            .as_ref()
            .map(|c| &c.mint)
            .filter(|m| {
                m.address == address
                    && m.decimals == decimals
                    && now >= m.checked_at_ms
                    && now - m.checked_at_ms < 30_000
                    && m.next_change_at_ms.is_none_or(|t| now < t)
            })
            .cloned();
        let mint = match cached_mint {
            Some(mint) => mint,
            None => stock_quotes::mint(address, decimals).await?,
        };
        self.ensure_generation(generation, &request.asset)?;
        let buy = stock_quotes::jupiter(request.keyed, SOLANA_USDC, address, &raw).await?;
        self.ensure_generation(generation, &request.asset)?;
        self.update_route(common::time::now_ms());
        let (reverse, quantity_limit) = match sell_raw(&self.snapshot(), &mint, &buy) {
            Ok(amount) => (
                stock_quotes::jupiter(request.keyed, address, SOLANA_USDC, &amount).await,
                None,
            ),
            Err(SellSizeError::Quantity(message, limit)) => (Err(message), Some(limit)),
            Err(error) => (Err(error.to_string()), None),
        };
        let (sell, sell_problem) = match reverse {
            Ok(q) => (Some(q), None),
            Err(e) => (None, Some(e)),
        };
        self.finish_comparison(
            generation,
            StockComparison {
                asset: request.asset,
                issuer_docs: docs.into(),
                budget_usdc: request.budget_usdc,
                keyed: request.keyed,
                mint,
                buy,
                sell,
                sell_problem,
                quantity_limit,
            },
            &hub,
        )
    }

    pub(super) fn ensure_generation(&self, generation: u64, asset: &str) -> Result<(), String> {
        if self.generation.load(Ordering::SeqCst) != generation
            || self
                .snapshot
                .read()
                .security
                .as_ref()
                .is_none_or(|s| s.asset != asset)
        {
            Err("股票已切换或停止，旧询价已丢弃".into())
        } else {
            Ok(())
        }
    }

    fn finish_comparison(
        &self,
        generation: u64,
        comparison: StockComparison,
        hub: &realtime::WsHub,
    ) -> Result<StockMarketSnapshot, String> {
        let mut snapshot = self.snapshot.write();
        if self.generation.load(Ordering::SeqCst) != generation
            || snapshot
                .security
                .as_ref()
                .is_none_or(|s| s.asset != comparison.asset)
        {
            return Err("股票已切换或停止，旧询价已丢弃".into());
        }
        snapshot.comparison = Some(comparison);
        snapshot.observed_at_ms =
            common::time::now_ms().max(snapshot.observed_at_ms.saturating_add(1));
        drop(snapshot);
        self.publish(hub);
        Ok(self.snapshot())
    }
}

#[cfg(test)]
pub(super) mod tests;
