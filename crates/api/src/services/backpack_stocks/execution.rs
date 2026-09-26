use super::*;
use crate::services::onchain_comparison::stock_costs::execution as chain;

impl BackpackStocks {
    // Only the explicitly confirmed full plan has a public submission path.
    pub(super) async fn send_stock_pair(
        self: &Arc<Self>,
        id: &str,
        hub: realtime::WsHub,
    ) -> Result<StockExecutionPlan, String> {
        let fingerprint = (self.credential_loader)()?.fingerprint();
        let plan = self.plan_store.get(id)?;
        if plan.terms.account_fingerprint != fingerprint {
            return Err("请恢复原计划的账户凭证".into());
        }
        if plan.two_leg_started_at_ms.is_some() {
            return Ok(plan);
        }
        let ready = if plan.terms.rfq.is_some() {
            self.rfq_subscription.borrow().as_deref() == Some(&fingerprint)
        } else {
            self.order_subscription.borrow().as_deref() == Some(&fingerprint)
        };
        if !ready {
            return Err("股票回执 WS 未就绪，请重新预检；没有提交".into());
        }
        self.dispatch_stock_pair(id, &hub, chain_execution::sign, |cost, signed| {
            Box::pin(chain::submit(cost, signed))
        })
        .await
    }

    async fn dispatch_stock_pair<F>(
        &self,
        id: &str,
        hub: &realtime::WsHub,
        signer: fn(&StockChainCost) -> Result<String, String>,
        send_chain: F,
    ) -> Result<StockExecutionPlan, String>
    where
        F: for<'a> FnOnce(
            &'a StockChainCost,
            &'a str,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Option<String>, String>> + Send + 'a>,
        >,
    {
        let _order = self.order_lock.try_lock().map_err(|_| "股票订单正在处理")?;
        let _rfq = self.rfq_lock.try_lock().map_err(|_| "股票 RFQ 正在处理")?;
        let _chain = self
            .chain_lock
            .try_lock()
            .map_err(|_| "原链上交易正在处理")?;
        let keys = (self.credential_loader)()?;
        let fingerprint = keys.fingerprint();
        let old = self.plan_store.get(id)?;
        if old.terms.account_fingerprint != fingerprint {
            return Err("请恢复原计划的账户凭证".into());
        }
        if old.two_leg_started_at_ms.is_some() {
            return Ok(old);
        }
        let now = common::time::now_ms();
        if old.phase_at(now) != StockPlanPhase::Reserved
            || now < old.terms.created_at_ms
            || now >= old.terms.market_valid_until_ms
        {
            return Err("股票原计划已过期或已提交，未签名或发送".into());
        }
        chain::validate_artifact(&old.terms.chain_cost)?;
        let (plan, send, signed) = self.with_plan_costs(&old, || {
            let signed = signer(&old.terms.chain_cost)?;
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
            snapshot.rfq_problem = self
                .rfq_store
                .problem()
                .or_else(|| self.rfq_problem.read().clone());
            let now = common::time::now_ms();
            plans::validate_for_submission(&old, &snapshot, evidence, now)?;
            let rfq = old
                .terms
                .rfq
                .as_ref()
                .map(|b| {
                    self.stock_rfq(&b.request_id)
                        .ok_or("原 RFQ 缺失".to_owned())
                })
                .transpose()?;
            let (plan, send) = self
                .plan_store
                .begin_pair(id, &fingerprint, &signed, rfq, now)?;
            Ok((plan, send, signed))
        })?;
        if !send {
            return Ok(plan);
        }
        self.publish_rfq(hub);
        // Each result is journaled independently, even if the other request times out.
        let cex = async {
            if plan.terms.rfq.is_some() {
                self.submit_prepared_rfq(&keys, &plan, hub).await
            } else {
                self.submit_prepared_order(&keys, &plan, hub).await
            }
        };
        let chain = async {
            let result = send_chain(&plan.terms.chain_cost, &signed).await;
            let saved = self
                .plan_store
                .change_chain(id, common::time::now_ms(), |r| {
                    match result {
                        Ok(hint) => {
                            r.provider_acknowledged = true;
                            r.provider_transaction_id = hint;
                            r.problem = Some("Provider 已回复，等待原交易最终回执".into());
                        }
                        Err(problem) => r.problem = Some(problem),
                    }
                    Ok(())
                });
            self.publish_rfq(hub);
            saved
        };
        let (cex, chain) = tokio::join!(cex, chain);
        cex?;
        chain?;
        self.plan_store.get(id)
    }

    pub(super) async fn recheck_stock_pair(
        self: &Arc<Self>,
        id: &str,
        hub: realtime::WsHub,
    ) -> Result<StockMarketSnapshot, String> {
        let _order = self
            .order_lock
            .try_lock()
            .map_err(|_| "原股票计划正在核对")?;
        let plan = self.plan_store.get(id)?;
        if plan.phase == StockPlanPhase::Settled {
            return Ok(self.snapshot());
        }
        let mut problems = vec![];
        if plan
            .chain_submission
            .as_ref()
            .is_some_and(|r| r.receipt.is_none())
        {
            if let Err(e) = self.recheck_stock_chain(id, hub.clone()).await {
                problems.push(e);
            }
        }
        if let Some(rfq) = &plan.rfq_acceptance {
            if let Err(e) = self.recheck_rfq(&rfq.request.request_id, hub.clone()).await {
                problems.push(e);
            }
        } else if plan.cex_order.is_some() {
            let result = async {
                let keys = (self.credential_loader)()?;
                self.reconcile_stock_order(id, &keys, false).await
            }
            .await;
            if let Err(e) = result {
                self.stock_order_problem(id, e.clone())?;
                problems.push(e);
            }
        }
        self.publish_rfq(&hub);
        if !problems.is_empty() {
            return Err(problems.join("；"));
        }
        Ok(self.snapshot())
    }
}

#[cfg(test)]
mod tests;
