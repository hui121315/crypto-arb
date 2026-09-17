use super::*;

impl BackpackStocks {
    pub(crate) async fn chain_cost(
        &self,
        mut request: StockChainCostRequest,
        hub: realtime::WsHub,
    ) -> Result<StockMarketSnapshot, String> {
        let _preflight = self
            .preflight_lock
            .try_lock()
            .map_err(|_| "另一项股票预检进行中")?;
        let _quote = self
            .quote_lock
            .try_lock()
            .map_err(|_| "股票正在询价，请稍后试算费用")?;
        request.wallet_address = request.wallet_address.trim().to_owned();
        super::super::onchain_comparison::stock_inventory::validate_owner(&request.wallet_address)?;
        let generation = self.generation.load(Ordering::SeqCst);
        self.ensure_generation(generation, &request.asset)?;
        let snapshot = self.snapshot();
        let (address, _, decimals) = comparison::issuer(&snapshot)?;
        let comparison = snapshot.comparison.as_ref().ok_or("请先取得双向链上报价")?;
        let now = common::time::now_ms();
        if comparison.asset != request.asset
            || comparison.mint.address != address
            || comparison.mint.decimals != decimals
            || now < comparison.mint.checked_at_ms
            || now - comparison.mint.checked_at_ms > 60_000
            || comparison.mint.next_change_at_ms.is_some_and(|t| now >= t)
        {
            return Err("股票合约或份额倍率需刷新，请先重新询价".into());
        }
        let cost =
            super::super::onchain_comparison::stock_costs::read(&request, comparison).await?;
        self.finish_chain_cost(generation, comparison, cost, &hub)
    }

    pub(super) fn finish_chain_cost(
        &self,
        generation: u64,
        baseline: &StockComparison,
        cost: StockChainCost,
        hub: &realtime::WsHub,
    ) -> Result<StockMarketSnapshot, String> {
        let mut snapshot = self.snapshot.write();
        if self.generation.load(Ordering::SeqCst) != generation
            || snapshot
                .security
                .as_ref()
                .is_none_or(|s| s.asset != cost.asset)
            || snapshot.comparison.as_ref() != Some(baseline)
        {
            return Err("股票或报价已变化，旧费用结果已丢弃".into());
        }
        apply(&mut snapshot, cost)?;
        snapshot.observed_at_ms =
            common::time::now_ms().max(snapshot.observed_at_ms.saturating_add(1));
        drop(snapshot);
        self.publish(hub);
        Ok(self.snapshot())
    }
}

pub(super) fn apply(snapshot: &mut StockMarketSnapshot, cost: StockChainCost) -> Result<(), String> {
    // The taker-specific build can change the quote. Publish amounts and costs together.
    let comparison = snapshot.comparison.as_mut().ok_or("股票报价已丢失")?;
    match cost.direction {
        StockChainDirection::Buy => comparison.buy = cost.quote.clone(),
        StockChainDirection::Sell => {
            comparison.sell = Some(cost.quote.clone());
            comparison.sell_problem = None;
        }
    }
    snapshot.preflight = None;
    snapshot.chain_costs.retain(|c| c.direction != cost.direction);
    snapshot.chain_costs.push(cost);
    Ok(())
}
