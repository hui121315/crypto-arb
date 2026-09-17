use super::*;
use stablecoin_store::native_topup as store;

impl BackpackStocks {
    pub(crate) async fn prepare_stablecoin_topup(
        &self,
        request: StockPlanRevisionRequest,
        hub: &realtime::WsHub,
    ) -> Result<StockMarketSnapshot, String> {
        let _guard = self
            .chain_lock
            .try_lock()
            .map_err(|_| "链上资金计划正在处理")?;
        let p = self.stablecoin_store.get(&request.plan_id)?;
        let now = common::time::now_ms();
        if p.revision != request.revision || p.native_topups.iter().any(|r| r.holds_funds(now)) {
            return Err("计划版本变化或已有补回待处理，请核对原计划".into());
        }
        let (native, slot) = store::target(&p)?;
        self.wallet_claims
            .check("solana", &p.request.conversion.wallet_address, now)?;
        let mut context = p.preview.cost.clone().ok_or("原兑换交易缺失")?;
        context.mint = stock_quotes::mint(STOCK_SOLANA_USDT, 6).await?;
        validate_mint(&context.mint)?;
        context.mint.slot = context.mint.slot.max(slot);
        let wallet = stock_inventory::read(&context.wallet_address, &context.mint).await?;
        let valuation = stock_costs::read_native_replacement(&context, native).await?;
        self.stablecoin_store.prepare_topup(
            &p.plan_id,
            StockNativeTopup {
                source_revision: p.revision,
                prepared_at_ms: common::time::now_ms(),
                valuation,
                wallet,
                submission: None,
            },
        )?;
        self.publish_plan(hub);
        Ok(self.snapshot())
    }

    pub(crate) fn cancel_stablecoin_topup(
        &self,
        request: StockTopupRecheckRequest,
        hub: &realtime::WsHub,
    ) -> Result<StockMarketSnapshot, String> {
        self.stablecoin_store.cancel_topup(
            &request.plan_id,
            request.index,
            common::time::now_ms(),
        )?;
        self.publish_plan(hub);
        Ok(self.snapshot())
    }
}

#[cfg(test)]
mod tests;
