use super::*;

impl BackpackStocks {
    pub(crate) fn settle_peer_plan(
        &self,
        request: StockPlanRevisionRequest,
        hub: &realtime::WsHub,
    ) -> Result<StockMarketSnapshot, String> {
        let _submission = self
            .submission_lock
            .try_lock()
            .map_err(|_| "原交易、处置或回执正在处理，暂不能结算")?;
        let _preflight = self
            .preflight_lock
            .try_lock()
            .map_err(|_| "股票账户预检或计划构建进行中")?;
        let _quote = self.quote_lock.try_lock().map_err(|_| "股票询价进行中")?;
        let (_, changed) = self
            .peer_plan_store
            .settle(&request, common::time::now_ms())?;
        if changed {
            let mut s = self.snapshot.write();
            s.peer_preflight = None;
            s.preflight = None;
            s.chain_costs.clear();
            drop(s);
            self.publish_plan(hub);
        }
        Ok(self.snapshot())
    }
}

#[cfg(test)]
pub(super) mod tests;
