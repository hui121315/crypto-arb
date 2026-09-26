use super::*;
use crate::services::onchain_comparison::stock_costs::execution as chain;

impl BackpackStocks {
    pub(in crate::services::backpack_stocks) async fn send_recovery(
        &self,
        id: &str,
        index: usize,
        hub: &realtime::WsHub,
    ) -> Result<StockExecutionPlan, String> {
        self.send_recovery_with(id, index, hub, chain_execution::sign, |cost, signed| {
            Box::pin(chain::submit(cost, signed))
        })
        .await
    }

    pub(in crate::services::backpack_stocks) async fn send_recovery_with<F>(
        &self,
        id: &str,
        index: usize,
        hub: &realtime::WsHub,
        signer: fn(&StockChainCost) -> Result<String, String>,
        send: F,
    ) -> Result<StockExecutionPlan, String>
    where
        F: for<'a> FnOnce(
            &'a StockChainCost,
            &'a str,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Option<String>, String>> + Send + 'a>,
        >,
    {
        let _guard = self
            .chain_lock
            .try_lock()
            .map_err(|_| "原交易或补偿正在处理")?;
        let fingerprint = (self.credential_loader)()?.fingerprint();
        let old = self.plan_store.get(id)?;
        if old.terms.account_fingerprint != fingerprint {
            return Err("补偿属于其他账户凭证".into());
        }
        let row = old.recoveries.get(index).ok_or("补偿计划不存在")?;
        if row.submission.is_some() {
            return Ok(old);
        }
        if index + 1 != old.recoveries.len() || row.cancelled_at_ms.is_some() {
            return Err("补偿已取消或被替代".into());
        }
        validate(&old, row, common::time::now_ms())?;
        let cost = row.cost.clone();
        let (plan, send_once, signed) = self.with_plan_costs(&old, || {
            let signed = signer(&cost)?;
            let (plan, send_once) = self.plan_store.begin_recovery(
                id,
                index,
                &fingerprint,
                &signed,
                common::time::now_ms(),
            )?;
            Ok((plan, send_once, signed))
        })?;
        if !send_once {
            return Ok(plan);
        }
        self.publish_rfq(hub);
        let result = send(&cost, &signed).await;
        let changed = self
            .plan_store
            .change_recovery(id, index, common::time::now_ms(), |s| {
                match result {
                    Ok(hint) => {
                        s.provider_acknowledged = true;
                        s.provider_transaction_id = hint;
                        s.problem = Some("补偿已回复，等待原交易最终回执".into());
                    }
                    Err(e) => s.problem = Some(e),
                }
                Ok(())
            });
        self.publish_rfq(hub);
        changed
    }

    pub(crate) fn cancel_recovery(
        &self,
        request: StockRecoveryActionRequest,
        hub: &realtime::WsHub,
    ) -> Result<StockMarketSnapshot, String> {
        self.plan_store
            .cancel_recovery(&request, common::time::now_ms())?;
        self.publish_rfq(hub);
        Ok(self.snapshot())
    }

    pub(crate) async fn recheck_recovery(
        &self,
        request: StockRecoveryActionRequest,
        hub: &realtime::WsHub,
    ) -> Result<StockMarketSnapshot, String> {
        let _guard = self
            .chain_lock
            .try_lock()
            .map_err(|_| "原交易或补偿正在处理")?;
        let now = common::time::now_ms();
        let plan = self
            .plan_store
            .change_recovery(&request.plan_id, request.index, now, |s| {
                if s.receipt.is_some() {
                    return Ok(());
                }
                if now < s.next_recheck_at_ms {
                    return Err("原补偿交易核对冷却中".into());
                }
                s.recheck_attempts = s.recheck_attempts.checked_add(1).ok_or("核对次数溢出")?;
                s.next_recheck_at_ms = now.saturating_add(5000);
                Ok(())
            })?;
        let row = &plan.recoveries[request.index];
        let submission = row.submission.as_ref().ok_or("补偿尚无原提交记录")?;
        if submission.receipt.is_some() {
            return Ok(self.snapshot());
        }
        let result = chain::lookup(&row.cost, submission).await;
        self.finish_recovery_recheck(&request.plan_id, request.index, result)?;
        self.publish_rfq(hub);
        Ok(self.snapshot())
    }

    pub(in crate::services::backpack_stocks) fn finish_recovery_recheck(
        &self,
        id: &str,
        index: usize,
        result: Result<chain::Lookup, String>,
    ) -> Result<StockExecutionPlan, String> {
        self.plan_store
            .change_recovery(id, index, common::time::now_ms(), |s| {
                match result {
                    Ok(found) => {
                        s.search_before = found.before;
                        if let Some(receipt) = found.receipt {
                            s.transaction_id = Some(receipt.transaction_id.clone());
                            s.receipt = Some(receipt);
                            s.problem = None;
                        } else {
                            s.problem = Some("尚未找到原补偿回执，保留占用，不重复发送".into());
                        }
                    }
                    Err(e) => s.problem = Some(e),
                }
                Ok(())
            })
    }
}
