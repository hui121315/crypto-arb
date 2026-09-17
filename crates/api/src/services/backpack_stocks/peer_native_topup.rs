use super::peer_execution::{ChainTransport, LiveChain};
use super::*;
use crate::services::onchain_comparison::stock_costs::execution as chain;
use crate::services::onchain_comparison::{stock_costs, stock_inventory};

impl BackpackStocks {
    pub(crate) async fn prepare_peer_native_topup(
        &self,
        r: StockPeerNativeTopupRequest,
        hub: &realtime::WsHub,
    ) -> Result<StockMarketSnapshot, String> {
        self.prepare_peer_native_topup_with(r, hub, |p, target, slot| async move {
            let mut c = p.terms.basis.chain_cost.clone();
            c.mint.slot = c.mint.slot.max(slot);
            let wallet = stock_inventory::read(&c.wallet_address, &c.mint).await?;
            let valuation = stock_costs::read_native_replacement(&c, target).await?;
            Ok((wallet, valuation))
        })
        .await
    }
    async fn prepare_peer_native_topup_with<F, Fut>(
        &self,
        r: StockPeerNativeTopupRequest,
        hub: &realtime::WsHub,
        read: F,
    ) -> Result<StockMarketSnapshot, String>
    where
        F: FnOnce(StockPeerPlan, u64, u64) -> Fut,
        Fut: std::future::Future<
            Output = Result<(StockWalletEvidence, StockNativeValuation), String>,
        >,
    {
        let _owner = self
            .submission_lock
            .try_lock()
            .map_err(|_| "原交易或 SOL 补回正在处理")?;
        let p = self.peer_plan_store.get(&r.plan_id)?;
        if p.native_topups
            .last()
            .is_some_and(|t| t.terms.source_revision == r.revision && t.usdc_limit == r.usdc_limit)
        {
            return Ok(self.snapshot());
        }
        if p.revision != r.revision
            || stock_peer_recovery_limit(&r.usdc_limit).is_none()
            || !p.peer_native_available(common::time::now_ms())
        {
            return Err("原计划变化、限额无效或已有处置待核对".into());
        }
        let adapter = self.peer_execution_adapter(&p)?;
        let (target, slot) = p.peer_native_target()?;
        let (wallet, valuation) = read(p.clone(), target, slot).await?;
        if !Arc::ptr_eq(&adapter, &self.peer_execution_adapter(&p)?) {
            return Err("补回准备期间原账户已改变".into());
        }
        self.peer_plan_store.prepare_native_topup(
            &p.plan_id,
            StockPeerNativeTopup {
                terms: StockNativeTopup {
                    source_revision: p.revision,
                    prepared_at_ms: common::time::now_ms(),
                    valuation,
                    wallet,
                    submission: None,
                },
                usdc_limit: r.usdc_limit,
                cancelled_at_ms: None,
            },
        )?;
        self.publish_plan(hub);
        Ok(self.snapshot())
    }
    pub(crate) fn cancel_peer_native_topup(
        &self,
        r: StockRecoveryActionRequest,
        hub: &realtime::WsHub,
    ) -> Result<StockMarketSnapshot, String> {
        self.peer_plan_store.cancel_native_topup(&r)?;
        self.publish_plan(hub);
        Ok(self.snapshot())
    }
    pub(crate) async fn submit_peer_native_topup(
        self: &Arc<Self>,
        r: StockPeerRecoverySubmitRequest,
        hub: realtime::WsHub,
        trading: Arc<crate::trading_service::TradingService>,
    ) -> Result<StockMarketSnapshot, String> {
        self.submit_peer_native_topup_with(
            r,
            hub,
            Arc::new(LiveChain),
            Arc::new(move || submission::check_live(&trading.risk_config())),
        )
        .await
    }
    async fn submit_peer_native_topup_with(
        self: &Arc<Self>,
        r: StockPeerRecoverySubmitRequest,
        hub: realtime::WsHub,
        io: Arc<dyn ChainTransport>,
        check: Arc<dyn Fn() -> Result<(), String> + Send + Sync>,
    ) -> Result<StockMarketSnapshot, String> {
        check()?;
        if !r.confirm_live {
            return Err("请确认本次真实 SOL 补回".into());
        }
        let p = self.peer_plan_store.get(&r.plan_id)?;
        if p.native_topups
            .get(r.index)
            .ok_or("SOL 补回不存在")?
            .terms
            .submission
            .is_some()
        {
            return Ok(self.snapshot());
        }
        let adapter = self.peer_execution_adapter(&p)?;
        let owner = self
            .submission_lock
            .clone()
            .try_lock_owned()
            .map_err(|_| "原交易或补回正在处理，不会另行排队")?;
        let service = self.clone();
        let (tx, rx) = tokio::sync::oneshot::channel();
        // The journal and task own the single send even when the HTTP caller leaves.
        tokio::spawn(async move {
            let result = tokio::time::timeout(Duration::from_secs(25), async {
                let p = service.peer_plan_store.get(&r.plan_id)?;
                let row = checked(&p, &r)?;
                let cost = row.cost(&p)?;
                io.check(&cost).await?;
                let mut refreshed = row.clone();
                refreshed.terms.wallet = io.wallet(&cost).await?;
                let mut prefix = p.clone();
                prefix.native_topups.truncate(r.index);
                refreshed.validate(&prefix, common::time::now_ms())?;
                check()?;
                if !Arc::ptr_eq(&adapter, &service.peer_execution_adapter(&p)?) {
                    return Err("签名前原账户已改变".into());
                }
                checked(&service.peer_plan_store.get(&r.plan_id)?, &r)?;
                let signed = io.sign(&cost)?;
                check()?;
                if !Arc::ptr_eq(&adapter, &service.peer_execution_adapter(&p)?) {
                    return Err("签名后原账户已改变，未发送".into());
                }
                service.peer_plan_store.begin_native_topup(&r, &signed)?;
                service.publish_plan(&hub);
                let result = io.send(&cost, &signed).await;
                service
                    .peer_plan_store
                    .change_native_topup(&r.plan_id, r.index, |s| {
                        match result {
                            Ok(hint) => {
                                s.provider_acknowledged = true;
                                s.provider_transaction_id = hint;
                                s.problem = Some("SOL 补回已回复，等待原交易最终回执".into());
                            }
                            Err(_) => {
                                s.problem =
                                    Some("SOL 补回提交结果未明，只核对原交易，不重发".into())
                            }
                        }
                        Ok(())
                    })?;
                Ok::<_, String>(())
            })
            .await
            .unwrap_or_else(|_| Err("SOL 补回处理超时，请核对原记录；没有自动重发".into()));
            service.publish_plan(&hub);
            drop(owner);
            let _ = tx.send(result.map(|_| service.snapshot()));
        });
        rx.await
            .map_err(|_| "SOL 补回任务中断，请核对原记录".to_owned())?
    }
    pub(crate) async fn recheck_peer_native_topup(
        &self,
        r: StockRecoveryActionRequest,
        hub: &realtime::WsHub,
    ) -> Result<StockMarketSnapshot, String> {
        self.recheck_peer_native_topup_with(r, hub, &LiveChain)
            .await
    }
    async fn recheck_peer_native_topup_with(
        &self,
        r: StockRecoveryActionRequest,
        hub: &realtime::WsHub,
        io: &dyn ChainTransport,
    ) -> Result<StockMarketSnapshot, String> {
        let _owner = self
            .submission_lock
            .try_lock()
            .map_err(|_| "原交易或 SOL 补回正在处理")?;
        let now = common::time::now_ms();
        let p = self
            .peer_plan_store
            .change_native_topup(&r.plan_id, r.index, |s| {
                if s.receipt.is_some() {
                    return Ok(());
                }
                if now < s.next_recheck_at_ms {
                    return Err("SOL 原补回核对冷却中".into());
                }
                s.recheck_attempts = s.recheck_attempts.checked_add(1).ok_or("核对次数溢出")?;
                s.next_recheck_at_ms = now.saturating_add(5000);
                Ok(())
            })?;
        let row = &p.native_topups[r.index];
        let s = row.terms.submission.as_ref().ok_or("SOL 补回尚未提交")?;
        if s.receipt.is_none() {
            let result = io.lookup(&row.cost(&p)?, s).await;
            self.peer_plan_store
                .change_native_topup(&r.plan_id, r.index, |s| {
                    match result {
                        Ok(found) => {
                            s.search_before = found.before;
                            if let Some(receipt) = found.receipt {
                                s.transaction_id = Some(receipt.transaction_id.clone());
                                s.receipt = Some(receipt);
                                s.problem = None;
                            } else {
                                s.problem =
                                    Some("原补回最终回执未找到，保留占用，不重复发送".into());
                            }
                        }
                        Err(_) => s.problem = Some("原补回查询失败，保留占用，稍后核对".into()),
                    }
                    Ok(())
                })?;
        }
        self.publish_plan(hub);
        Ok(self.snapshot())
    }
}

fn checked<'a>(
    p: &'a StockPeerPlan,
    r: &StockPeerRecoverySubmitRequest,
) -> Result<&'a StockPeerNativeTopup, String> {
    let row = p.native_topups.get(r.index).ok_or("SOL 补回不存在")?;
    if !r.confirm_live
        || p.revision != r.revision
        || r.index + 1 != p.native_topups.len()
        || row.cancelled_at_ms.is_some()
        || row.terms.submission.is_some()
    {
        return Err("补回未确认、已变化、已取消或已提交".into());
    }
    let mut prefix = p.clone();
    prefix.native_topups.truncate(r.index);
    row.validate(&prefix, common::time::now_ms())?;
    chain::validate_artifact(&row.cost(p)?)?;
    Ok(row)
}

#[cfg(test)]
mod tests;
