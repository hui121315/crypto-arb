use super::*;
use crate::services::onchain_comparison::stock_costs::execution as chain;

impl BackpackStocks {
    pub(super) async fn send_native_topup(
        &self,
        id: &str,
        index: usize,
        hub: &realtime::WsHub,
    ) -> Result<StockExecutionPlan, String> {
        self.send_topup_with(id, index, hub, chain_execution::sign, |cost, signed| {
            Box::pin(chain::submit(cost, signed))
        })
        .await
    }

    pub(super) async fn send_topup_with<F>(
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
            .map_err(|_| "原交易或补回正在处理")?;
        let fingerprint = (self.credential_loader)()?.fingerprint();
        let old = self.plan_store.get(id)?;
        if old.terms.account_fingerprint != fingerprint {
            return Err("股票计划属于其他账户凭证".into());
        }
        let row = old.native_topups.get(index).ok_or("SOL 补回计划不存在")?;
        if row.submission.is_some() {
            return Ok(old);
        }
        let cost = settlement::native_cost(&old, row)?;
        let now = common::time::now_ms();
        if old.phase != StockPlanPhase::SubmissionUnknown
            || index + 1 != old.native_topups.len()
            || now < row.prepared_at_ms
            || now >= cost.valid_until_ms
        {
            return Err("SOL 补回计划已变化或过期，未签名或发送".into());
        }
        let signed = signer(&cost)?;
        let (plan, send_once) = self.plan_store.begin_topup(
            id,
            index,
            &fingerprint,
            &signed,
            common::time::now_ms(),
        )?;
        if !send_once {
            return Ok(plan);
        }
        self.publish_rfq(hub);
        let result = send(&cost, &signed).await;
        let changed = self
            .plan_store
            .change_topup(id, index, common::time::now_ms(), |r| {
                match result {
                    Ok(hint) => {
                        r.provider_acknowledged = true;
                        r.provider_transaction_id = hint;
                        r.problem = Some("SOL 补回已回复，等待原交易最终回执".into());
                    }
                    Err(e) => r.problem = Some(e),
                }
                Ok(())
            });
        self.publish_rfq(hub);
        changed
    }

    pub(crate) async fn recheck_native_topup(
        &self,
        id: &str,
        index: usize,
        hub: &realtime::WsHub,
    ) -> Result<StockMarketSnapshot, String> {
        let _guard = self
            .chain_lock
            .try_lock()
            .map_err(|_| "原交易或补回正在处理")?;
        let now = common::time::now_ms();
        let plan = self.plan_store.change_topup(id, index, now, |r| {
            if r.receipt.is_some() {
                return Ok(());
            }
            if now < r.next_recheck_at_ms {
                return Err("原补回交易核对冷却中".into());
            }
            r.recheck_attempts = r.recheck_attempts.checked_add(1).ok_or("核对次数溢出")?;
            r.next_recheck_at_ms = now.saturating_add(5000);
            Ok(())
        })?;
        let row = &plan.native_topups[index];
        let submission = row.submission.as_ref().ok_or("SOL 补回没有原提交记录")?;
        if submission.receipt.is_some() {
            return Ok(self.snapshot());
        }
        let result = chain::lookup(&settlement::native_cost(&plan, row)?, submission).await;
        self.finish_topup_recheck(id, index, result)?;
        self.publish_rfq(hub);
        Ok(self.snapshot())
    }

    pub(super) fn finish_topup_recheck(
        &self,
        id: &str,
        index: usize,
        result: Result<chain::Lookup, String>,
    ) -> Result<StockExecutionPlan, String> {
        self.plan_store
            .change_topup(id, index, common::time::now_ms(), |r| {
                match result {
                    Ok(found) => {
                        r.search_before = found.before;
                        if let Some(receipt) = found.receipt {
                            r.transaction_id = Some(receipt.transaction_id.clone());
                            r.receipt = Some(receipt);
                            r.problem = None;
                        } else {
                            r.problem = Some("尚未找到原补回回执，保留占用，不重复发送".into());
                        }
                    }
                    Err(e) => r.problem = Some(e),
                }
                Ok(())
            })
    }
}

#[cfg(test)]
pub(super) mod tests;
