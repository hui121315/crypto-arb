use super::*;
use crate::services::{onchain_comparison::stock_costs::execution as chain, onchain_signer};

impl BackpackStocks {
    // Internal transport only; the public API cannot submit a single unhedged leg.
    #[allow(dead_code)]
    pub(super) async fn send_chain_leg(
        &self,
        id: &str,
        hub: &realtime::WsHub,
    ) -> Result<StockExecutionPlan, String> {
        self.send_chain_with(id, hub, sign, |cost, signed| {
            Box::pin(chain::submit(cost, signed))
        })
        .await
    }

    async fn send_chain_with<F>(
        &self,
        id: &str,
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
            .map_err(|_| "链上原交易正在处理")?;
        let fingerprint = (self.credential_loader)()?.fingerprint();
        let old = self.plan_store.get(id)?;
        if old.terms.account_fingerprint != fingerprint {
            return Err("股票计划属于其他账户凭证".into());
        }
        if old.chain_submission.is_some() {
            return Ok(old);
        }
        let now = common::time::now_ms();
        if old.phase_at(now) != StockPlanPhase::Reserved
            || now < old.terms.created_at_ms
            || now >= old.terms.market_valid_until_ms
        {
            return Err("股票计划已过期或不再处于预留状态，未签名或发送".into());
        }
        chain::validate_artifact(&old.terms.chain_cost)?;
        let signed = signer(&old.terms.chain_cost)?;
        let (plan, send_once) = {
            let _state = self.rfq_state_lock.lock();
            let account = self.account.read();
            let evidence = account
                .evidence
                .as_ref()
                .filter(|a| a.fingerprint == fingerprint)
                .ok_or("原账户预检已失效")?;
            let current = self.snapshot.read();
            let mut snapshot = current.clone();
            snapshot.rfqs = self.visible_rfqs();
            snapshot.rfq_connected =
                self.rfq_subscription.borrow().as_deref() == Some(&fingerprint);
            snapshot.rfq_problem = self.rfq_store.problem().or_else(|| self.rfq_problem.read().clone());
            let now = common::time::now_ms();
            plans::validate_for_submission(&old, &snapshot, evidence, now)?;
            self.plan_store
                .begin_chain(id, &fingerprint, &signed, now)?
        };
        if !send_once {
            return Ok(plan);
        }
        self.publish_rfq(hub);
        let result = send(&plan.terms.chain_cost, &signed).await;
        let updated = self
            .plan_store
            .change_chain(id, common::time::now_ms(), |r| {
                match result {
                    Ok(hint) => {
                        r.provider_acknowledged = true;
                        r.provider_transaction_id = hint;
                        r.problem = Some("Provider 已回复，等待原交易链上最终回执".into());
                    }
                    Err(problem) => r.problem = Some(problem),
                }
                Ok(())
            });
        self.publish_rfq(hub);
        updated
    }

    pub(super) async fn recheck_stock_chain(
        &self,
        id: &str,
        hub: realtime::WsHub,
    ) -> Result<StockMarketSnapshot, String> {
        let _guard = self
            .chain_lock
            .try_lock()
            .map_err(|_| "链上原交易正在核对")?;
        let plan = self.start_chain_recheck(id, common::time::now_ms())?;
        let row = plan.chain_submission.as_ref().ok_or("没有链上原交易")?;
        if row.receipt.is_some() {
            return Ok(self.snapshot());
        }
        let result = chain::lookup(&plan.terms.chain_cost, row).await;
        self.finish_chain_recheck(id, result)?;
        self.publish_rfq(&hub);
        Ok(self.snapshot())
    }

    pub(super) fn start_chain_recheck(&self, id: &str, now: i64) -> Result<StockExecutionPlan, String> {
        self.plan_store.change_chain(id, now, |r| {
            if r.receipt.is_some() {
                return Ok(());
            }
            if now < r.next_recheck_at_ms {
                return Err("原交易核对冷却中，请稍后再试".into());
            }
            r.recheck_attempts = r
                .recheck_attempts
                .checked_add(1)
                .ok_or("原交易核对次数溢出")?;
            r.next_recheck_at_ms = now.saturating_add(5000);
            Ok(())
        })
    }

    pub(super) fn finish_chain_recheck(
        &self,
        id: &str,
        result: Result<chain::Lookup, String>,
    ) -> Result<StockExecutionPlan, String> {
        self.plan_store
            .change_chain(id, common::time::now_ms(), |r| {
                match result {
                    Ok(found) => {
                        r.search_before = found.before;
                        if let Some(receipt) = found.receipt {
                            r.transaction_id = Some(receipt.transaction_id.clone());
                            r.receipt = Some(receipt);
                            r.problem = None;
                        } else {
                            r.problem =
                                Some("尚未找到原交易的最终回执，保留占用，不重复提交".into());
                        }
                    }
                    Err(problem) => r.problem = Some(problem),
                }
                Ok(())
            })
    }
}

pub(super) fn sign(cost: &StockChainCost) -> Result<String, String> {
    onchain_signer::sign(
        "solana",
        &cost.wallet_address,
        cost.transaction.as_ref().ok_or("原交易缺失")?,
        None,
        None,
    )
}

#[cfg(test)]
mod tests;
