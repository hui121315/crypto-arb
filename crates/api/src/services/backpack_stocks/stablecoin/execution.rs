use super::*;
use crate::services::onchain_comparison::stock_costs::execution as chain;
use futures::future::BoxFuture;
use stablecoin_store::native_topup as topup;

impl BackpackStocks {
    pub(crate) async fn submit_stablecoin(
        self: &Arc<Self>,
        request: StockStablecoinSubmitRequest,
        hub: realtime::WsHub,
        trading: Arc<crate::trading_service::TradingService>,
    ) -> Result<StockMarketSnapshot, String> {
        self.submit_stablecoin_with(
            request,
            hub,
            move || trading.risk_config(),
            |c| Box::pin(chain::recheck_original(c)),
            chain_execution::sign,
            |c, s| Box::pin(chain::submit(c, s)),
        )
        .await
    }

    async fn submit_stablecoin_with<R, V, S>(
        self: &Arc<Self>,
        request: StockStablecoinSubmitRequest,
        hub: realtime::WsHub,
        risk: R,
        verify: V,
        signer: fn(&StockChainCost) -> Result<String, String>,
        send: S,
    ) -> Result<StockMarketSnapshot, String>
    where
        R: Fn() -> trading::RiskConfig + Send + Sync + 'static,
        V: for<'a> FnOnce(&'a StockChainCost) -> BoxFuture<'a, Result<(), String>> + Send + 'static,
        S: for<'a> FnOnce(
                &'a StockChainCost,
                &'a str,
            ) -> BoxFuture<'a, Result<Option<String>, String>>
            + Send
            + 'static,
    {
        self.submit_stablecoin_operation(request, None, hub, risk, verify, signer, send)
            .await
    }

    pub(crate) async fn submit_stablecoin_topup(
        self: &Arc<Self>,
        request: StockStablecoinTopupSubmitRequest,
        hub: realtime::WsHub,
        trading: Arc<crate::trading_service::TradingService>,
    ) -> Result<StockMarketSnapshot, String> {
        self.submit_stablecoin_operation(
            StockStablecoinSubmitRequest {
                plan_id: request.plan_id,
                revision: request.revision,
                confirm_live: request.confirm_live,
            },
            Some(request.index),
            hub,
            move || trading.risk_config(),
            |c| Box::pin(chain::recheck_original(c)),
            chain_execution::sign,
            |c, s| Box::pin(chain::submit(c, s)),
        )
        .await
    }

    pub(super) async fn submit_stablecoin_operation<R, V, S>(
        self: &Arc<Self>,
        request: StockStablecoinSubmitRequest,
        index: Option<usize>,
        hub: realtime::WsHub,
        risk: R,
        verify: V,
        signer: fn(&StockChainCost) -> Result<String, String>,
        send: S,
    ) -> Result<StockMarketSnapshot, String>
    where
        R: Fn() -> trading::RiskConfig + Send + Sync + 'static,
        V: for<'a> FnOnce(&'a StockChainCost) -> BoxFuture<'a, Result<(), String>> + Send + 'static,
        S: for<'a> FnOnce(
                &'a StockChainCost,
                &'a str,
            ) -> BoxFuture<'a, Result<Option<String>, String>>
            + Send
            + 'static,
    {
        if self.stablecoin_previous(&request, index)? {
            return Ok(self.snapshot());
        }
        submission::check_live(&risk())?;
        let guard = self
            .submission_lock
            .clone()
            .try_lock_owned()
            .map_err(|_| "资金提交正在处理，不会排队重复兑换")?;
        let service = self.clone();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        // HTTP cancellation must not cancel ownership between journaling and transport.
        tokio::spawn(async move {
            let result = tokio::time::timeout(Duration::from_secs(25), async {
                let _chain = service
                    .chain_lock
                    .try_lock()
                    .map_err(|_| "链上原交易正在处理")?;
                if service.stablecoin_previous(&request, index)? {
                    return Ok(service.snapshot());
                }
                let original = service.stablecoin_store.get(&request.plan_id)?;
                let cost = operation_cost(&original, index)?;
                submission::check_live(&risk())?;
                verify(&cost).await?;
                if service.stablecoin_previous(&request, index)? {
                    return Ok(service.snapshot());
                }
                submission::check_live(&risk())?;
                let signed = signer(&cost)?;
                submission::check_live(&risk())?;
                let (plan, once) = if let Some(i) = index {
                    service.stablecoin_store.begin_topup(
                        &request,
                        i,
                        &signed,
                        common::time::now_ms(),
                    )?
                } else {
                    service
                        .stablecoin_store
                        .begin(&request, &signed, common::time::now_ms())?
                };
                if once {
                    service.publish_plan(&hub);
                    let result = send(&cost, &signed).await;
                    service.change_stablecoin_operation(
                        &plan.plan_id,
                        index,
                        common::time::now_ms(),
                        |s| {
                            match result {
                                Ok(hint) => {
                                    s.provider_acknowledged = true;
                                    s.provider_transaction_id = hint;
                                    s.problem =
                                        Some("Provider 已回复，尚未核实链上最终到账".into());
                                }
                                Err(e) => s.problem = Some(e.chars().take(400).collect()),
                            }
                            Ok(())
                        },
                    )?;
                }
                Ok(service.snapshot())
            })
            .await
            .map_err(|_| "兑换处理超时，请查看原计划；已提交的只查原交易，不重发".to_owned())
            .and_then(|r| r);
            service.publish_plan(&hub);
            drop(guard);
            let _ = sender.send(result);
        });
        receiver
            .await
            .map_err(|_| "兑换任务中断，请核对原计划，不重复提交".to_owned())?
    }

    fn stablecoin_previous(
        &self,
        r: &StockStablecoinSubmitRequest,
        index: Option<usize>,
    ) -> Result<bool, String> {
        if !r.confirm_live {
            return Err("请确认本次稳定币实盘兑换".into());
        }
        let p = self.stablecoin_store.get(&r.plan_id)?;
        if let Some(i) = index {
            let row = p.native_topups.get(i).ok_or("SOL 补回计划不存在")?;
            if row.terms.submission.is_some() {
                return Ok(true);
            }
            if p.phase != StockStablecoinPlanPhase::Completed
                || p.revision != r.revision
                || i + 1 != p.native_topups.len()
                || !row.current(common::time::now_ms())
            {
                return Err("补回已取消、过期或版本变化，未发送".into());
            }
            return Ok(false);
        }
        if p.submission.is_some() {
            return Ok(true);
        }
        let now = common::time::now_ms();
        if p.revision != r.revision
            || p.phase_at(now) != StockStablecoinPlanPhase::Reserved
            || !p.preview.can_reserve(now)
        {
            return Err("兑换计划版本变化、已取消或报价过期，未提交".into());
        }
        Ok(false)
    }

    pub(crate) async fn recheck_stablecoin(
        &self,
        id: &str,
        hub: &realtime::WsHub,
    ) -> Result<StockMarketSnapshot, String> {
        self.recheck_stablecoin_with(id, hub, |c, s| Box::pin(chain::lookup(c, s)))
            .await
    }

    async fn recheck_stablecoin_with<F>(
        &self,
        id: &str,
        hub: &realtime::WsHub,
        lookup: F,
    ) -> Result<StockMarketSnapshot, String>
    where
        F: for<'a> FnOnce(
            &'a StockChainCost,
            &'a StockChainSubmission,
        ) -> BoxFuture<'a, Result<chain::Lookup, String>>,
    {
        self.recheck_stablecoin_operation(id, None, hub, lookup)
            .await
    }

    pub(crate) async fn recheck_stablecoin_topup(
        &self,
        request: StockTopupRecheckRequest,
        hub: &realtime::WsHub,
    ) -> Result<StockMarketSnapshot, String> {
        self.recheck_stablecoin_operation(&request.plan_id, Some(request.index), hub, |c, s| {
            Box::pin(chain::lookup(c, s))
        })
        .await
    }

    pub(super) async fn recheck_stablecoin_operation<F>(
        &self,
        id: &str,
        index: Option<usize>,
        hub: &realtime::WsHub,
        lookup: F,
    ) -> Result<StockMarketSnapshot, String>
    where
        F: for<'a> FnOnce(
            &'a StockChainCost,
            &'a StockChainSubmission,
        ) -> BoxFuture<'a, Result<chain::Lookup, String>>,
    {
        let _guard = self
            .chain_lock
            .try_lock()
            .map_err(|_| "链上原交易正在核对")?;
        let now = common::time::now_ms();
        let p = self.change_stablecoin_operation(id, index, now, |s| {
            if s.receipt.is_some() {
                return Ok(());
            }
            if now < s.next_recheck_at_ms {
                return Err("原兑换核对冷却中，请稍后再试".into());
            }
            s.recheck_attempts = s.recheck_attempts.checked_add(1).ok_or("核对次数溢出")?;
            s.next_recheck_at_ms = now.saturating_add(5000);
            Ok(())
        })?;
        let row = operation_submission(&p, index).ok_or("尚未提交原交易")?;
        if row.receipt.is_some() {
            return Ok(self.snapshot());
        }
        self.publish_plan(hub);
        let result = lookup(&operation_cost(&p, index)?, row).await;
        self.change_stablecoin_operation(id, index, common::time::now_ms(), |s| {
            match result {
                Ok(found) => {
                    s.search_before = found.before;
                    if let Some(receipt) = found.receipt {
                        s.transaction_id = Some(receipt.transaction_id.clone());
                        s.receipt = Some(receipt);
                        s.problem = None;
                    } else {
                        s.problem = Some("尚未找到最终回执，继续保留钱包占用；不会重发".into());
                    }
                }
                Err(e) => s.problem = Some(e.chars().take(400).collect()),
            }
            Ok(())
        })?;
        self.publish_plan(hub);
        Ok(self.snapshot())
    }

    fn change_stablecoin_operation(
        &self,
        id: &str,
        index: Option<usize>,
        now: i64,
        change: impl FnOnce(&mut StockChainSubmission) -> Result<(), String>,
    ) -> Result<StockStablecoinPlan, String> {
        if let Some(index) = index {
            self.stablecoin_store.change_topup(id, index, now, change)
        } else {
            self.stablecoin_store.change_submission(id, now, change)
        }
    }
}

fn operation_cost(p: &StockStablecoinPlan, index: Option<usize>) -> Result<StockChainCost, String> {
    if let Some(i) = index {
        topup::cost(p, i)
    } else {
        p.preview.cost.clone().ok_or("兑换缺少原交易".into())
    }
}

fn operation_submission(
    p: &StockStablecoinPlan,
    index: Option<usize>,
) -> Option<&StockChainSubmission> {
    if let Some(i) = index {
        p.native_topups.get(i)?.terms.submission.as_ref()
    } else {
        p.submission.as_ref()
    }
}

#[cfg(test)]
mod tests;
